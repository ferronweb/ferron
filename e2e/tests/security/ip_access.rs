use std::io::Write;

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};

use crate::common;

async fn create_ferron_container(
    webroot_dir: &std::path::Path,
    config_file: &std::path::Path,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let ferron_image = self::common::build_ferron_image().await?;
    ferron_image
        .with_exposed_port(ContainerPort::Tcp(80))
        .with_wait_for(WaitFor::Http(Box::new(
            HttpWaitStrategy::new("/")
                .with_port(ContainerPort::Tcp(80))
                .with_response_matcher(|_| true),
        )))
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

async fn raw_http_get(addr: &str, port: u16, path: &str) -> u16 {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let request =
        format!("GET {path} HTTP/1.1\r\nHost: {addr}:{port}\r\nConnection: close\r\n\r\n");

    let mut stream = tokio::net::TcpStream::connect((addr, port))
        .await
        .expect("Failed to connect");

    stream.write_all(request.as_bytes()).await.unwrap();
    stream.flush().await.unwrap();

    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf).await.unwrap();
    buf.truncate(n);

    let response = String::from_utf8_lossy(&buf);
    response
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// Test that `block "0.0.0.0/0"` rejects all requests with 403.
#[tokio::test]
async fn test_ip_block_all() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();

    std::fs::write(webroot_dir.path().join("index.html"), b"hello").unwrap();

    let mut config_file = common::create_temp_file();

    config_file
        .as_file_mut()
        .write_all(
            br#"*:80 {
    block "0.0.0.0/0"
    root "/var/www/ferron"
}
"#,
        )
        .unwrap();
    config_file.flush().unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .expect("Failed to create container");

    let ferron_port = container
        .get_host_port_ipv4(80)
        .await
        .expect("Failed to get host port");

    let status = raw_http_get("127.0.0.1", ferron_port, "/index.html").await;
    assert_eq!(
        status, 403,
        "Expected 403 Forbidden when all IPs are blocked, got {status}"
    );
}

/// Test that `allow "127.0.0.1"` blocks non-localhost connections.
/// In Docker, the connection comes from the Docker bridge gateway,
/// so it should be rejected.
#[tokio::test]
async fn test_ip_allow_only_localhost() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();

    std::fs::write(webroot_dir.path().join("index.html"), b"hello").unwrap();

    let mut config_file = common::create_temp_file();

    config_file
        .as_file_mut()
        .write_all(
            br#"*:80 {
    allow "127.0.0.1"
    root "/var/www/ferron"
}
"#,
        )
        .unwrap();
    config_file.flush().unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .expect("Failed to create container");

    let ferron_port = container
        .get_host_port_ipv4(80)
        .await
        .expect("Failed to get host port");

    // The Docker bridge connection comes from a non-localhost IP
    // (e.g., 172.17.0.1), so it should be blocked.
    let status = raw_http_get("127.0.0.1", ferron_port, "/index.html").await;
    assert_eq!(
        status, 403,
        "Expected 403 when only 127.0.0.1 is allowed (Docker IP is not localhost), got {status}"
    );
}

/// Test that without any block/allow directives, requests succeed normally.
#[tokio::test]
async fn test_ip_no_restrictions() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();

    std::fs::write(webroot_dir.path().join("index.html"), b"hello").unwrap();

    let mut config_file = common::create_temp_file();

    config_file
        .as_file_mut()
        .write_all(
            br#"*:80 {
    root "/var/www/ferron"
}
"#,
        )
        .unwrap();
    config_file.flush().unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .expect("Failed to create container");

    let ferron_port = container
        .get_host_port_ipv4(80)
        .await
        .expect("Failed to get host port");

    let status = raw_http_get("127.0.0.1", ferron_port, "/index.html").await;
    assert_eq!(
        status, 200,
        "Expected 200 when no IP restrictions are configured, got {status}"
    );
}
