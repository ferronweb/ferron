#[cfg(unix)]
use std::{io::Write, path::Path};

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};

async fn create_ferron_container(
    webroot_dir: &Path,
    config_file: &Path,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let ferron_image = crate::common::build_ferron_image().await?;
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

/// Helper: send raw HTTP request and return the full response text.
async fn raw_http_get_full(host: &str, port: u16, raw_path: &str, extra_headers: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let request = format!(
        "GET {raw_path} HTTP/1.1\r\nHost: {host}:{port}\r\n{extra_headers}Connection: close\r\n\r\n"
    );

    let mut stream = tokio::net::TcpStream::connect((host, port))
        .await
        .expect("Failed to connect");

    stream.write_all(request.as_bytes()).await.unwrap();
    stream.flush().await.unwrap();

    let mut buf = vec![0u8; 8192];
    let mut response = Vec::new();
    loop {
        match stream.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => response.extend_from_slice(&buf[..n]),
            Err(_) => break,
        }
    }

    String::from_utf8_lossy(&response).to_string()
}

/// Helper: get response status code from raw HTTP response.
fn get_status(response: &str) -> u16 {
    response
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

#[tokio::test]
async fn test_redirecting() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let webroot_dir = crate::common::create_temp_dir();
    #[cfg(unix)]
    let mut config_file = crate::common::create_temp_file();
    #[cfg(not(unix))]
    let webroot_dir = tempfile::tempdir().unwrap();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
  root "/var/www/ferron"
  status 301 {
    url "/"
    location "/basic.txt"
  }
}
"#
            .as_bytes(),
        )
        .unwrap();

    crate::common::write_file(webroot_dir.path().join("basic.txt"), b"test content").unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::new();

    // Request that should be redirected
    let response = client
        .get(format!("http://localhost:{}/", port))
        .send()
        .await
        .unwrap();

    assert!(
        matches!(
            response.status(),
            reqwest::StatusCode::OK | reqwest::StatusCode::MOVED_PERMANENTLY
        ),
        "expected 200 OK or 301 Moved Permanently, got {}",
        response.status()
    );

    container.stop().await.unwrap();
}

/// Test that trailing_slash_redirect false prevents the automatic
/// 301 redirect for directory paths without a trailing slash.
#[tokio::test]
async fn test_trailing_slash_redirect_disabled() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let webroot_dir = crate::common::create_temp_dir();
    #[cfg(unix)]
    let mut config_file = crate::common::create_temp_file();
    #[cfg(not(unix))]
    let webroot_dir = tempfile::tempdir().unwrap();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    // Create a directory in the webroot
    crate::common::create_dir(webroot_dir.path().join("subdir")).unwrap();
    crate::common::write_file(
        webroot_dir.path().join("subdir/index.html"),
        b"index file content",
    )
    .unwrap();

    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
    root "/var/www/ferron"
    trailing_slash_redirect false
}
"#
            .as_bytes(),
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

    let response = raw_http_get_full("127.0.0.1", port, "/subdir", "").await;
    let status = get_status(&response);

    assert!(
        status != 301 && status != 308 && status != 302,
        "trailing_slash_redirect false should prevent redirect, got status: {}\nFull response: {}",
        status,
        response
    );

    assert!(
        status == 200 || status == 403,
        "Expected 200 (directory listing/index) or 403 (if disabled), got: {}\nFull response: {}",
        status,
        response
    );
}
