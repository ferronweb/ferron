#[cfg(unix)]
use std::{io::Write, path::Path};

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};

mod common;

async fn create_ferron_container_with_env(
    webroot_dir: &Path,
    config_file: &Path,
    log_dir: &Path,
    env_vars: Vec<(&str, &str)>,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let ferron_image = self::common::build_ferron_image().await?;
    let mut image = ferron_image
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
        .with_mount(Mount::bind_mount(
            log_dir.to_string_lossy(),
            "/var/log/ferron",
        ));

    for (key, value) in env_vars {
        image = image.with_env_var(key, value);
    }

    image.start().await
}

fn setup_test_dirs() -> (
    tempfile::TempDir,
    tempfile::NamedTempFile,
    tempfile::TempDir,
) {
    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    let webroot_dir = common::create_temp_dir();
    let config_file = common::create_temp_file();
    let log_dir = common::create_temp_dir();

    common::write_file(
        webroot_dir.path().join("index.html"),
        b"<html><body>test</body></html>",
    )
    .unwrap();

    (webroot_dir, config_file, log_dir)
}

#[tokio::test]
async fn test_access_log_per_host_interpolation() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let (webroot_dir, mut config_file, log_dir) = setup_test_dirs();

    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
    root "/var/www/ferron"
    log "/var/log/ferron/{{accesslog.header_host}}/access.log"
}
"#
            .as_bytes(),
        )
        .unwrap();

    let container = create_ferron_container_with_env(
        webroot_dir.path(),
        config_file.path(),
        log_dir.path(),
        vec![],
    )
    .await
    .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::new();

    let host1_dir = log_dir.path().join("host1.example.com");
    let host2_dir = log_dir.path().join("host2.example.com");

    for _ in 0..5 {
        let resp = client
            .get(format!("http://localhost:{}/", port))
            .header("Host", "host1.example.com")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::OK);

        let resp = client
            .get(format!("http://localhost:{}/", port))
            .header("Host", "host2.example.com")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::OK);

        if host1_dir.exists() && host2_dir.exists() {
            break;
        }

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    assert!(
        host1_dir.exists(),
        "Expected host1.example.com log directory to exist"
    );
    assert!(
        host2_dir.exists(),
        "Expected host2.example.com log directory to exist"
    );

    tokio::time::sleep(std::time::Duration::from_millis(500)).await; // Wait until logs flush

    let host1_log = host1_dir.join("access.log");
    let host2_log = host2_dir.join("access.log");
    assert!(
        host1_log.exists(),
        "Expected host1.example.com/access.log to exist"
    );
    assert!(
        host2_log.exists(),
        "Expected host2.example.com/access.log to exist"
    );

    let host1_content = std::fs::read_to_string(&host1_log).unwrap_or_default();
    let host2_content = std::fs::read_to_string(&host2_log).unwrap_or_default();
    assert!(
        !host1_content.is_empty(),
        "Host1 log file should contain entries"
    );
    assert!(
        !host2_content.is_empty(),
        "Host2 log file should contain entries"
    );

    container.stop().await.unwrap();
}

#[tokio::test]
async fn test_access_log_env_variable_interpolation() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let (webroot_dir, mut config_file, log_dir) = setup_test_dirs();

    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
    root "/var/www/ferron"
    log "/var/log/ferron/{{env.LOG_SUBDIR}}/access.log"
}
"#
            .as_bytes(),
        )
        .unwrap();

    let container = create_ferron_container_with_env(
        webroot_dir.path(),
        config_file.path(),
        log_dir.path(),
        vec![("LOG_SUBDIR", "custom-subdir")],
    )
    .await
    .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::new();

    let subdir = log_dir.path().join("custom-subdir");
    let log_file = subdir.join("access.log");

    for _ in 0..5 {
        let resp = client
            .get(format!("http://localhost:{}/", port))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::OK);

        if log_file.exists() {
            break;
        }

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    assert!(
        log_file.exists(),
        "Expected custom-subdir/access.log to exist"
    );

    tokio::time::sleep(std::time::Duration::from_millis(500)).await; // Wait until logs flush

    let content = std::fs::read_to_string(&log_file).unwrap_or_default();
    assert!(!content.is_empty(), "Log file should contain entries");

    container.stop().await.unwrap();
}
