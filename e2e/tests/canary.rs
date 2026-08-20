use std::{
    io::{Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    bollard::query_parameters::KillContainerOptionsBuilder,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};

mod common;

async fn create_ferron_container(
    webroot_dir: &Path,
    config_file: &Path,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let ferron_image = common::build_ferron_image().await?;
    ferron_image
        .with_exposed_port(ContainerPort::Tcp(80))
        .with_wait_for(WaitFor::Http(Box::new(
            HttpWaitStrategy::new("/")
                .with_port(ContainerPort::Tcp(80))
                .with_response_matcher(|_| true),
        )))
        .with_network("bridge")
        .with_mount(Mount::bind_mount(
            webroot_dir.to_string_lossy(),
            "/var/www/ferron",
        ))
        .with_mount(Mount::bind_mount(
            config_file.to_string_lossy(),
            "/etc/ferron.conf",
        ))
        .start()
        .await
}

fn canary_config(variant_entries: &str) -> String {
    format!(
        r#"
*:80 {{
  canary rollout {{
    affinity cookie ab_variant
    {variant_entries}
  }}
  root "/var/www/ferron/{{{{canary.variant}}}}"
  index index.html
}}
"#
    )
}

async fn fetch(client: &reqwest::Client, port: u16, cookie: Option<&str>) -> String {
    let mut request = client.get(format!("http://localhost:{}/", port));
    if let Some(cookie) = cookie {
        request = request.header(reqwest::header::COOKIE, format!("ab_variant={cookie}"));
    }
    request.send().await.unwrap().text().await.unwrap()
}

#[tokio::test]
async fn test_canary_sticky_cookie_affinity() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    let webroot_dir = create_dir_with_variants();
    let mut config_file = common::create_temp_file();
    config_file
        .as_file_mut()
        .write_all(canary_config("variant stable 50\n    variant next 50").as_bytes())
        .unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();
    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::new();

    let cookie = "user-1";
    let first = fetch(&client, port, Some(cookie)).await;
    assert!(first == "STABLE CONTENT" || first == "NEXT CONTENT");
    for _ in 0..3 {
        assert_eq!(fetch(&client, port, Some(cookie)).await, first);
    }

    let mut seen_stable = false;
    let mut seen_next = false;
    for i in 0..24 {
        match fetch(&client, port, Some(&format!("user-{i}")))
            .await
            .as_str()
        {
            "STABLE CONTENT" => seen_stable = true,
            "NEXT CONTENT" => seen_next = true,
            other => panic!("unexpected variant content: {other}"),
        }
    }
    assert!(seen_stable && seen_next, "expected both variants served");

    container.stop().await.unwrap();
}

#[tokio::test]
async fn test_canary_ip_fallback_without_cookie() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    let webroot_dir = create_dir_with_variants();
    let mut config_file = common::create_temp_file();
    config_file
        .as_file_mut()
        .write_all(canary_config("variant stable 50\n    variant next 50").as_bytes())
        .unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();
    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::new();

    let first = fetch(&client, port, None).await;
    assert!(first == "STABLE CONTENT" || first == "NEXT CONTENT");
    for _ in 0..2 {
        assert_eq!(fetch(&client, port, None).await, first);
    }

    container.stop().await.unwrap();
}

#[tokio::test]
async fn test_canary_reload_preserves_affinity() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    let webroot_dir = create_dir_with_variants();
    let mut config_file = common::create_temp_file();
    config_file
        .as_file_mut()
        .write_all(canary_config("variant stable 90\n    variant next 10").as_bytes())
        .unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();
    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::new();

    let cookie = "reload-user";
    let before = fetch(&client, port, Some(cookie)).await;
    assert!(before == "STABLE CONTENT" || before == "NEXT CONTENT");

    config_file.as_file_mut().seek(SeekFrom::Start(0)).unwrap();
    config_file.as_file_mut().set_len(0).unwrap();
    config_file
        .as_file_mut()
        .write_all(canary_config("variant stable 90\n    variant next 10").as_bytes())
        .unwrap();

    testcontainers::bollard::Docker::connect_with_local_defaults()
        .unwrap()
        .kill_container(
            container.id(),
            Some(KillContainerOptionsBuilder::new().signal("SIGHUP").build()),
        )
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    assert_eq!(fetch(&client, port, Some(cookie)).await, before);

    container.stop().await.unwrap();
}

#[tokio::test]
async fn test_canary_reload_weight_change_moves_few_keys() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    let webroot_dir = create_dir_with_variants();
    let mut config_file = common::create_temp_file();
    config_file
        .as_file_mut()
        .write_all(canary_config("variant stable 50\n    variant next 50").as_bytes())
        .unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();
    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::new();

    let cookies: Vec<String> = (0..12).map(|i| format!("shift-user-{i}")).collect();
    let before: Vec<String> = {
        let mut collected = Vec::with_capacity(cookies.len());
        for cookie in &cookies {
            collected.push(fetch(&client, port, Some(cookie)).await);
        }
        collected
    };

    config_file.as_file_mut().seek(SeekFrom::Start(0)).unwrap();
    config_file.as_file_mut().set_len(0).unwrap();
    config_file
        .as_file_mut()
        .write_all(canary_config("variant stable 55\n    variant next 45").as_bytes())
        .unwrap();

    testcontainers::bollard::Docker::connect_with_local_defaults()
        .unwrap()
        .kill_container(
            container.id(),
            Some(KillContainerOptionsBuilder::new().signal("SIGHUP").build()),
        )
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    let mut unchanged = 0;
    for (cookie, variant) in cookies.iter().zip(before.iter()) {
        if &fetch(&client, port, Some(cookie)).await == variant {
            unchanged += 1;
        }
    }
    // A 5-point weight change re-maps roughly 5% of keys. Out of 12 keys,
    // allow a margin and require at least 10 to keep their variant.
    assert!(
        unchanged >= 10,
        "expected at least 10 of 12 keys to keep their variant, got {unchanged}"
    );

    container.stop().await.unwrap();
}

fn create_dir_with_variants() -> tempfile::TempDir {
    let webroot_dir = common::create_temp_dir();
    let stable: PathBuf = webroot_dir.path().join("stable");
    common::create_dir(stable.clone()).unwrap();
    common::write_file(stable.join("index.html"), b"STABLE CONTENT").unwrap();
    let next: PathBuf = webroot_dir.path().join("next");
    common::create_dir(next.clone()).unwrap();
    common::write_file(next.join("index.html"), b"NEXT CONTENT").unwrap();
    webroot_dir
}

#[tokio::test]
async fn test_canary_ferron_sets_sticky_cookie() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    let webroot_dir = create_dir_with_variants();
    let mut config_file = common::create_temp_file();
    config_file
        .as_file_mut()
        .write_all(
            canary_config("set_cookie\n    variant stable 50\n    variant next 50").as_bytes(),
        )
        .unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();
    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::new();

    // First request without a cookie: Ferron assigns a variant and sets the
    // affinity cookie on the response.
    let first = client
        .get(format!("http://localhost:{}/", port))
        .send()
        .await
        .unwrap();
    let first_cookie = first
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .and_then(|h| h.to_str().ok())
        .expect("Set-Cookie header expected")
        .to_string();
    let first_body = first.text().await.unwrap();
    assert!(first_body == "STABLE CONTENT" || first_body == "NEXT CONTENT");
    let key = first_cookie
        .strip_prefix("ab_variant=")
        .and_then(|v| v.split_once(";"))
        .map(|(k, _)| k)
        .expect("unexpected Set-Cookie format")
        .to_string();
    assert_eq!(key.len(), 32);
    assert!(key.chars().all(|c| c.is_ascii_hexdigit()));

    // A second fresh client gets a different random key.
    let second_cookie = client
        .get(format!("http://localhost:{}/", port))
        .send()
        .await
        .unwrap()
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .and_then(|h| h.to_str().ok())
        .expect("Set-Cookie header expected")
        .to_string();
    assert!(!second_cookie.contains(&key), "random keys must differ");

    // Follow-up requests with the returned cookie stay on the variant and
    // do not trigger a new Set-Cookie.
    for _ in 0..3 {
        let response = client
            .get(format!("http://localhost:{}/", port))
            .header(reqwest::header::COOKIE, format!("ab_variant={key}"))
            .send()
            .await
            .unwrap();
        let has_set_cookie = response
            .headers()
            .get(reqwest::header::SET_COOKIE)
            .is_none();
        let body = response.text().await.unwrap();
        assert_eq!(body, first_body);
        assert!(has_set_cookie);
    }

    container.stop().await.unwrap();
}
