use reqwest::StatusCode;
use std::io::Write;
use std::path::Path;

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
        .with_exposed_port(ContainerPort::Tcp(8081))
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

#[tokio::test]
async fn test_admin_status_and_config() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let webroot_dir = tempfile::Builder::new()
        .permissions(std::os::unix::fs::PermissionsExt::from_mode(0o777))
        .tempdir()
        .unwrap();
    #[cfg(unix)]
    let mut config_file = tempfile::Builder::new()
        .permissions(std::os::unix::fs::PermissionsExt::from_mode(0o666))
        .tempfile()
        .unwrap();

    #[cfg(not(unix))]
    let webroot_dir = tempfile::tempdir().unwrap();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    config_file
        .as_file_mut()
        .write_all(
            r#"{
  admin {
    listen "0.0.0.0:8081"
    health true
    status true
    config true
    reload true
  }
}
*:80 {
  root "/var/www/ferron"
}
"#
            .as_bytes(),
        )
        .unwrap();

    common::write_file(webroot_dir.path().join("index.html"), b"ok").unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let admin_port = container
        .get_host_port_ipv4(ContainerPort::Tcp(8081))
        .await
        .unwrap();

    let client = reqwest::Client::new();

    // /health should return OK
    let resp = client
        .get(format!("http://localhost:{}/health", admin_port))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // /status should return JSON with expected fields
    let resp = client
        .get(format!("http://localhost:{}/status", admin_port))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json: serde_json::Value = resp.json().await.unwrap();
    assert!(json.get("uptime_sec").is_some());
    assert!(json.get("requests_total").is_some());

    // /config should return sanitized JSON
    let resp = client
        .get(format!("http://localhost:{}/config", admin_port))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json: serde_json::Value = resp.json().await.unwrap();
    assert!(json.get("global_config").is_some());
    assert!(json.get("ports").is_some());

    // POST /reload should initiate reload
    let resp = client
        .post(format!("http://localhost:{}/reload", admin_port))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let j: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        j.get("status").and_then(|v| v.as_str()),
        Some("reload_initiated")
    );

    container.stop().await.unwrap();
}
