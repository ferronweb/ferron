#[cfg(unix)]
use std::{fs::Permissions, os::unix::fs::PermissionsExt};
use std::{io::Write, path::Path};

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};

mod common;

async fn create_ferron_container(
    webroot_dir: &Path,
    config_file: &Path,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let ferron_image = self::common::build_ferron_image().await?;
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

fn write_config(config_file: &mut impl Write, config: &str) {
    config_file.write_all(config.as_bytes()).unwrap();
}

#[tokio::test]
async fn test_abuse_protection_does_not_block_normal_traffic() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let webroot_dir = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o777))
        .tempdir()
        .unwrap();
    #[cfg(unix)]
    let mut config_file = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();
    #[cfg(not(unix))]
    let webroot_dir = tempfile::tempdir().unwrap();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    write_config(
        &mut config_file,
        r#"
*:80 {
  root "/var/www/ferron"
  abuse_protection {
    ban_duration "1m"
    rate_limit_threshold {
      events 1
      window "60s"
    }
  }
}
"#,
    );

    self::common::write_file(webroot_dir.path().join("test.txt"), b"test content").unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();

    // Requests should pass through since no events have been recorded to trigger a ban
    let response = client
        .get(format!("http://localhost:{}/test.txt", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    let response = client
        .get(format!("http://localhost:{}/test.txt", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    // Verify the server properly handles concurrent requests with abuse protection
    let mut handles = Vec::new();
    for _ in 0..10 {
        let url = format!("http://localhost:{}/test.txt", port);
        let cl = client.clone();
        handles.push(tokio::spawn(async move { cl.get(&url).send().await }));
    }
    for handle in handles {
        let response = handle.await.unwrap().unwrap();
        assert_eq!(response.status(), 200);
    }

    container.stop().await.unwrap();
}

#[tokio::test]
async fn test_abuse_protection_blocks_rate_limit_abusers() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let webroot_dir = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o777))
        .tempdir()
        .unwrap();
    #[cfg(unix)]
    let mut config_file = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();
    #[cfg(not(unix))]
    let webroot_dir = tempfile::tempdir().unwrap();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    // Configure rate_limit to allow only 1 request, and abuse_protection
    // to ban after 2 rate limit events. Sending 3 fast requests should:
    //   1st → pass (consumes the only token)
    //   2nd → 429 (rate limited, emits abuse event #1)
    //   3rd → 429 (rate limited, emits abuse event #2 → ban triggered)
    //   4th → 403 (banned by abuse protection)
    write_config(
        &mut config_file,
        r#"
*:80 {
  root "/var/www/ferron"

  rate_limit {
    rate 2
    burst 0
    key remote_address
    window 60
  }

  abuse_protection {
    ban_duration "1m"
    rate_limit_threshold {
      events 2
      window "60s"
    }
  }
}
"#,
    );

    self::common::write_file(webroot_dir.path().join("data.txt"), b"data").unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let url = format!("http://localhost:{}/data.txt", port);

    // 1st request: passes (consumes the token)
    let resp = client.get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 200, "first request should pass");

    // 2nd request: rate limited (429), triggers abuse event #1
    let resp = client.get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 429, "second request should be rate limited");

    // 3rd request: rate limited (429), triggers abuse event #2 → ban
    let resp = client.get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 429, "third request should be rate limited");

    // 4th request: IP is now banned → 403 Forbidden with Retry-After
    let resp = client.get(&url).send().await.unwrap();
    assert_eq!(
        resp.status(),
        403,
        "fourth request should be banned by abuse protection"
    );

    // Verify the Retry-After header is present
    let retry_after = resp.headers().get("retry-after");
    assert!(
        retry_after.is_some(),
        "banned response should have Retry-After header"
    );

    container.stop().await.unwrap();
}

#[tokio::test]
async fn test_abuse_protection_without_rate_limit_does_not_block() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let webroot_dir = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o777))
        .tempdir()
        .unwrap();
    #[cfg(unix)]
    let mut config_file = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();
    #[cfg(not(unix))]
    let webroot_dir = tempfile::tempdir().unwrap();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    // Abuse protection without rate_limit — no events are emitted so
    // no bans should be triggered regardless of request volume.
    write_config(
        &mut config_file,
        r#"
*:80 {
  root "/var/www/ferron"
  abuse_protection {
    enabled true
    ban_duration "1m"
    rate_limit_threshold {
      events 1
      window "60s"
    }
  }
}
"#,
    );

    self::common::write_file(webroot_dir.path().join("page.html"), b"page").unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();

    // Many requests with no rate limiting should all pass
    for _ in 0..20 {
        let resp = client
            .get(format!("http://localhost:{}/page.html", port))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
    }

    container.stop().await.unwrap();
}
