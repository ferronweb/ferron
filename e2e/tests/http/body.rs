//! Request body handling tests for Ferron reverse proxy.
//!
//! These tests are inspired by the nginx-tests `body.t` and `body_chunked.t`
//! test files, which verify correct request body reading, buffering, and
//! forwarding to upstream backends. The original nginx tests cover body
//! buffering, max body size, body discard, and chunked transfer encoding.
//!
//! See: https://github.com/nginx/nginx-tests/blob/master/body.t
//! See: https://github.com/nginx/nginx-tests/blob/master/body_chunked.t

use std::io::Write;

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};

mod common;

async fn create_backend_container(
    network: &str,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let backend_image = self::common::build_backend_image().await?;
    backend_image
        .with_exposed_port(ContainerPort::Tcp(3000))
        .with_wait_for(WaitFor::Http(Box::new(
            HttpWaitStrategy::new("/")
                .with_port(ContainerPort::Tcp(3000))
                .with_response_matcher(|_| true),
        )))
        .with_network(network)
        .with_hostname("backend")
        .start()
        .await
}

async fn create_ferron_container(
    network: &str,
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
        .with_network(network)
        .with_hostname("ferron")
        .with_mount(Mount::bind_mount(
            config_file.to_string_lossy(),
            "/etc/ferron.conf",
        ))
        .start()
        .await
}

/// Test that a small POST body is correctly proxied to the backend.
///
/// Inspired by nginx-tests body.t — verifies body reading and forwarding.
#[tokio::test]
async fn test_proxy_small_post_body() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = self::common::create_temp_file();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-body-small";

    let _backend = create_backend_container(network).await.unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
  proxy "http://backend:3000"
}
"#,
        )
        .unwrap();

    let ferron = create_ferron_container(network, config_file.path())
        .await
        .unwrap();

    let port = ferron
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();

    let client = reqwest::Client::new();

    let body = "Hello from the client!";
    let response = client
        .post(format!("http://localhost:{}/echo-body", port))
        .header("Content-Type", "text/plain")
        .body(body)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), body);

    ferron.stop().await.unwrap();
}

/// Test that a large POST body is correctly proxied.
///
/// Inspired by nginx-tests body.t — verifies body buffering with large payloads.
#[tokio::test]
async fn test_proxy_large_post_body() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = self::common::create_temp_file();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-body-large";

    let _backend = create_backend_container(network).await.unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
  proxy "http://backend:3000"
}
"#,
        )
        .unwrap();

    let ferron = create_ferron_container(network, config_file.path())
        .await
        .unwrap();

    let port = ferron
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .unwrap();

    // Create a 16KB body
    let body = "X".repeat(16384);
    let response = client
        .post(format!("http://localhost:{}/echo-body", port))
        .header("Content-Type", "text/plain")
        .body(body.clone())
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), body);

    ferron.stop().await.unwrap();
}

/// Test that a chunked POST body is correctly proxied.
///
/// Inspired by nginx-tests body_chunked.t — verifies chunked transfer encoding
/// is handled correctly when forwarding request bodies.
#[tokio::test]
async fn test_proxy_chunked_post_body() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = self::common::create_temp_file();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-body-chunked";

    let _backend = create_backend_container(network).await.unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
  proxy "http://backend:3000"
}
"#,
        )
        .unwrap();

    let ferron = create_ferron_container(network, config_file.path())
        .await
        .unwrap();

    let port = ferron
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();

    // Use raw TCP to send chunked request
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();

    let body = "chunked body data";
    let request = format!(
        "POST /echo-body HTTP/1.1\r\n\
         Host: localhost\r\n\
         Content-Type: text/plain\r\n\
         Transfer-Encoding: chunked\r\n\
         \r\n\
         {:x}\r\n\
         {}\r\n\
         0\r\n\
         \r\n",
        body.len(),
        body
    );

    stream.write_all(request.as_bytes()).await.unwrap();
    stream.flush().await.unwrap();

    let mut buf = vec![0u8; 8192];
    let mut response_bytes = Vec::new();
    loop {
        match tokio::time::timeout(std::time::Duration::from_secs(5), stream.read(&mut buf)).await {
            Ok(Ok(0)) => break,
            Ok(Ok(n)) => response_bytes.extend_from_slice(&buf[..n]),
            _ => break,
        }
    }

    let response = String::from_utf8_lossy(&response_bytes);
    // Find the body after the double CRLF
    if let Some(pos) = response.find("\r\n\r\n") {
        let response_body = &response[pos + 4..];
        assert_eq!(
            response_body.trim(),
            body,
            "Chunked body should be proxied correctly"
        );
    } else {
        panic!("Could not find response body separator in: {}", response);
    }

    ferron.stop().await.unwrap();
}

/// Test that a PUT body is correctly proxied.
///
/// Inspired by nginx-tests body.t — verifies non-POST methods also forward bodies.
#[tokio::test]
async fn test_proxy_put_body() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = self::common::create_temp_file();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-body-put";

    let _backend = create_backend_container(network).await.unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
  proxy "http://backend:3000"
}
"#,
        )
        .unwrap();

    let ferron = create_ferron_container(network, config_file.path())
        .await
        .unwrap();

    let port = ferron
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();

    let client = reqwest::Client::new();

    let body = "PUT body content";
    let response = client
        .put(format!("http://localhost:{}/echo-body", port))
        .header("Content-Type", "text/plain")
        .body(body)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), body);

    ferron.stop().await.unwrap();
}

/// Test that an empty POST body is correctly proxied.
///
/// Inspired by nginx-tests body.t — edge case with zero-length body.
#[tokio::test]
async fn test_proxy_empty_body() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = self::common::create_temp_file();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-body-empty";

    let _backend = create_backend_container(network).await.unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
  proxy "http://backend:3000"
}
"#,
        )
        .unwrap();

    let ferron = create_ferron_container(network, config_file.path())
        .await
        .unwrap();

    let port = ferron
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();

    let client = reqwest::Client::new();

    let response = client
        .post(format!("http://localhost:{}/echo-body", port))
        .header("Content-Type", "text/plain")
        .body("")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    // Empty body should result in empty response
    assert_eq!(response.text().await.unwrap(), "");

    ferron.stop().await.unwrap();
}
