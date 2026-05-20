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
    log_dir: &Path,
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
        .with_mount(Mount::bind_mount(
            log_dir.to_string_lossy(),
            "/var/log/ferron",
        ))
        .start()
        .await
}

fn read_file_contents(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn file_exists(path: &Path) -> bool {
    path.exists()
}

fn setup_test_dirs() -> (
    tempfile::TempDir,
    tempfile::NamedTempFile,
    tempfile::TempDir,
) {
    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let webroot_dir = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o777))
        .tempdir()
        .unwrap();
    #[cfg(unix)]
    let config_file = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();
    #[cfg(unix)]
    let log_dir = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o777))
        .tempdir()
        .unwrap();

    #[cfg(not(unix))]
    let webroot_dir = tempfile::tempdir().unwrap();
    #[cfg(not(unix))]
    let config_file = tempfile::NamedTempFile::new().unwrap();
    #[cfg(not(unix))]
    let log_dir = tempfile::tempdir().unwrap();

    common::write_file(
        webroot_dir.path().join("index.html"),
        b"<html><body>test</body></html>",
    )
    .unwrap();

    (webroot_dir, config_file, log_dir)
}

#[tokio::test]
async fn test_log_rotation_basic() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let (webroot_dir, mut config_file, log_dir) = setup_test_dirs();

    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
  root "/var/www/ferron"
  log "/var/log/ferron/access.log" {
    access_log_rotate_size 100
    access_log_rotate_keep 3
  }
}
"#
            .as_bytes(),
        )
        .unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path(), log_dir.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::new();

    let access_log_1 = log_dir.path().join("access.log.1");

    for _ in 0..20 {
        let resp = client
            .get(format!("http://localhost:{}/", port))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::OK);

        if file_exists(&access_log_1) {
            break;
        }

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    assert!(
        file_exists(&access_log_1),
        "Expected rotated log file access.log.1 to exist after generating enough requests"
    );

    let rotated_content = read_file_contents(&access_log_1);
    assert!(
        !rotated_content.is_empty(),
        "Rotated log file should contain log entries"
    );

    container.stop().await.unwrap();
}

#[tokio::test]
async fn test_log_rotation_keep_limit() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let (webroot_dir, mut config_file, log_dir) = setup_test_dirs();

    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
  root "/var/www/ferron"
  log "/var/log/ferron/access.log" {
    access_log_rotate_size 50
    access_log_rotate_keep 2
  }
}
"#
            .as_bytes(),
        )
        .unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path(), log_dir.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::new();

    let access_log_3 = log_dir.path().join("access.log.3");

    for _ in 0..50 {
        let resp = client
            .get(format!("http://localhost:{}/", port))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::OK);

        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    }

    assert!(
        !file_exists(&access_log_3),
        "Expected access.log.3 to NOT exist (rotate_keep=2 should limit to .1 and .2)"
    );

    let access_log_1 = log_dir.path().join("access.log.1");
    let access_log_2 = log_dir.path().join("access.log.2");
    assert!(file_exists(&access_log_1), "Expected access.log.1 to exist");
    assert!(file_exists(&access_log_2), "Expected access.log.2 to exist");

    container.stop().await.unwrap();
}

#[tokio::test]
async fn test_log_rotation_keep_zero() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let (webroot_dir, mut config_file, log_dir) = setup_test_dirs();

    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
  root "/var/www/ferron"
  log "/var/log/ferron/access.log" {
    access_log_rotate_size 50
    access_log_rotate_keep 0
  }
}
"#
            .as_bytes(),
        )
        .unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path(), log_dir.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::new();

    let access_log_1 = log_dir.path().join("access.log.1");

    for _ in 0..30 {
        let resp = client
            .get(format!("http://localhost:{}/", port))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::OK);

        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    }

    assert!(
        !file_exists(&access_log_1),
        "Expected access.log.1 to NOT exist (rotate_keep=0 should delete on rotation)"
    );

    container.stop().await.unwrap();
}

#[tokio::test]
async fn test_log_rotation_disabled() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let (webroot_dir, mut config_file, log_dir) = setup_test_dirs();

    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
  root "/var/www/ferron"
  log "/var/log/ferron/access.log"
}
"#
            .as_bytes(),
        )
        .unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path(), log_dir.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::new();

    for _ in 0..10 {
        let resp = client
            .get(format!("http://localhost:{}/", port))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::OK);
    }

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let access_log = log_dir.path().join("access.log");
    let access_log_1 = log_dir.path().join("access.log.1");

    assert!(file_exists(&access_log), "Expected access.log to exist");
    assert!(
        !file_exists(&access_log_1),
        "Expected no rotated files when rotation is disabled"
    );

    let content = read_file_contents(&access_log);
    assert!(!content.is_empty(), "Log file should contain entries");

    container.stop().await.unwrap();
}

#[tokio::test]
async fn test_error_log_rotation_config_accepted() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let (webroot_dir, mut config_file, log_dir) = setup_test_dirs();

    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
  root "/var/www/ferron"
  error_log "/var/log/ferron/error.log" {
    error_log_rotate_size 100
    error_log_rotate_keep 2
  }
}
"#
            .as_bytes(),
        )
        .unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path(), log_dir.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("http://localhost:{}/", port))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    container.stop().await.unwrap();
}
