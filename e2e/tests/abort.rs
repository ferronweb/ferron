use std::io::Write;
#[cfg(unix)]
use std::{fs::Permissions, os::unix::fs::PermissionsExt};

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};

mod common;

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

/// Send a raw HTTP request and return whether we received any response.
async fn raw_http_expect_empty(host: &str, port: u16, request: &[u8]) -> bool {
    use tokio::io::AsyncReadExt;
    use tokio::io::AsyncWriteExt;
    use tokio::time::Duration;

    let mut stream = match tokio::net::TcpStream::connect((host, port)).await {
        Ok(s) => s,
        Err(_) => return true, // connection refused = aborted
    };

    let _ = stream.write_all(request).await;
    let _ = stream.flush().await;

    // Try to read — should get nothing (connection closed)
    let mut buf = [0u8; 1];
    let result = tokio::time::timeout(Duration::from_millis(500), stream.read(&mut buf)).await;

    result.is_err() || matches!(result, Ok(Err(_))) || matches!(result, Ok(Ok(0)))
}

/// abort true immediately closes the connection without sending any HTTP response.
#[tokio::test]
async fn test_abort_true() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let webroot_dir = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o777))
        .tempdir()
        .unwrap();
    #[cfg(not(unix))]
    let webroot_dir = tempfile::tempdir().unwrap();

    std::fs::write(webroot_dir.path().join("index.html"), b"hello").unwrap();

    #[cfg(unix)]
    let mut config_file = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
    root "/var/www/ferron"
}

abort.example.com:80 {
    abort true
    root "/var/www/ferron"
}
"#,
        )
        .unwrap();
    config_file.flush().unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .expect("Failed to create container");

    let port = container
        .get_host_port_ipv4(80)
        .await
        .expect("Failed to get host port");

    // Request with the abort host header — connection should be closed
    let request = format!(
        "GET /index.html HTTP/1.1\r\nHost: abort.example.com:{port}\r\nConnection: close\r\n\r\n"
    );

    let aborted = raw_http_expect_empty("127.0.0.1", port, request.as_bytes()).await;
    assert!(
        aborted,
        "Connection should be aborted (no response) for abort.example.com"
    );
}

/// abort false allows requests through normally.
#[tokio::test]
async fn test_abort_false_allows_requests() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let webroot_dir = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o777))
        .tempdir()
        .unwrap();
    #[cfg(not(unix))]
    let webroot_dir = tempfile::tempdir().unwrap();

    std::fs::write(webroot_dir.path().join("index.html"), b"hello").unwrap();

    #[cfg(unix)]
    let mut config_file = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
    root "/var/www/ferron"
}

normal.example.com:80 {
    abort false
    root "/var/www/ferron"
}
"#,
        )
        .unwrap();
    config_file.flush().unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .expect("Failed to create container");

    let port = container
        .get_host_port_ipv4(80)
        .await
        .expect("Failed to get host port");

    let client = reqwest::Client::new();
    let response = client
        .get(format!("http://127.0.0.1:{port}/index.html"))
        .header("Host", "normal.example.com")
        .send()
        .await
        .expect("Request should succeed");

    assert_eq!(response.status(), 200, "Expected 200 OK with abort false");
    let body = response.text().await.expect("Failed to read body");
    assert_eq!(body, "hello", "Response body should be normal");
}

/// Bare `abort` (without argument) should also abort the connection.
#[tokio::test]
async fn test_abort_bare_directive() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let webroot_dir = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o777))
        .tempdir()
        .unwrap();
    #[cfg(not(unix))]
    let webroot_dir = tempfile::tempdir().unwrap();

    std::fs::write(webroot_dir.path().join("index.html"), b"hello").unwrap();

    #[cfg(unix)]
    let mut config_file = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
    root "/var/www/ferron"
}

abort.example.com:80 {
    abort
    root "/var/www/ferron"
}
"#,
        )
        .unwrap();
    config_file.flush().unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .expect("Failed to create container");

    let port = container
        .get_host_port_ipv4(80)
        .await
        .expect("Failed to get host port");

    let request = format!(
        "GET /index.html HTTP/1.1\r\nHost: abort.example.com:{port}\r\nConnection: close\r\n\r\n"
    );

    let aborted = raw_http_expect_empty("127.0.0.1", port, request.as_bytes()).await;
    assert!(
        aborted,
        "Bare abort directive should also close the connection"
    );
}
