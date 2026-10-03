//! `OPTIONS *` (asterisk-form) handling tests for Ferron.
//!
//! Ferron answers asterisk-form `OPTIONS` requests itself, before the pipeline
//! runs, and advertises the methods from `http { options_allowed_methods }`.
//! Origin-form `OPTIONS /path` requests must keep going through the pipeline.
//!
//! See: modules/http-server/src/handler/mod.rs

use std::io::Write;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::common;

/// Send a raw request and return the response bytes.
///
/// Raw TCP is required because `reqwest` normalizes URLs and cannot send the
/// asterisk-form request target.
async fn send_raw(port: u16, request: &str) -> Vec<u8> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect failed");
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write failed");
    stream.flush().await.expect("flush failed");

    let mut buf = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut tmp = [0u8; 4096];
    loop {
        match tokio::time::timeout_at(deadline, stream.read(&mut tmp)).await {
            Ok(Ok(0)) | Err(_) => break,
            Ok(Ok(n)) => buf.extend_from_slice(&tmp[..n]),
            Ok(Err(_)) => break,
        }
    }
    buf
}

fn response_head(response: &[u8]) -> String {
    String::from_utf8_lossy(response)
        .split("\r\n\r\n")
        .next()
        .unwrap_or_default()
        .to_lowercase()
}

fn status_code(response: &[u8]) -> u16 {
    response_head(response)
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// Test that `OPTIONS *` returns the default `Allow` list.
#[tokio::test]
async fn test_options_star_default_allow_header() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let mut config_file = common::create_temp_file();
    common::write_file(webroot_dir.path().join("test.txt"), b"hello").unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"*:80 {
    root "/var/www/ferron"
}
"#,
        )
        .unwrap();

    let container = common::create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();
    let port = container
        .get_host_port_ipv4(testcontainers::core::ContainerPort::Tcp(80))
        .await
        .unwrap();

    let response = send_raw(
        port,
        "OPTIONS * HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n",
    )
    .await;

    assert_eq!(
        status_code(&response),
        200,
        "got: {}",
        response_head(&response)
    );
    assert!(
        response_head(&response).contains("allow: get, head, post, options"),
        "expected the documented default Allow list, got: {}",
        response_head(&response)
    );

    container.stop().await.unwrap();
}

/// Test that `options_allowed_methods` is honored per host.
#[tokio::test]
async fn test_options_star_per_host_allow_header() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let mut config_file = common::create_temp_file();
    common::write_file(webroot_dir.path().join("test.txt"), b"hello").unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"{
    http {
        options_allowed_methods "GET, HEAD, OPTIONS"
    }
}

readonly.example.com:80 {
    root "/var/www/ferron"
}

rw.example.com:80 {
    root "/var/www/ferron"
    http {
        options_allowed_methods "GET, HEAD, POST, PUT, DELETE, OPTIONS"
    }
}
"#,
        )
        .unwrap();

    let container = common::create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();
    let port = container
        .get_host_port_ipv4(testcontainers::core::ContainerPort::Tcp(80))
        .await
        .unwrap();

    // Inherited from the global `http` block.
    let inherited = send_raw(
        port,
        "OPTIONS * HTTP/1.1\r\nHost: readonly.example.com\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(
        response_head(&inherited).contains("allow: get, head, options"),
        "expected the host to inherit the global Allow list, got: {}",
        response_head(&inherited)
    );

    // Overridden by the host `http` block.
    let overridden = send_raw(
        port,
        "OPTIONS * HTTP/1.1\r\nHost: rw.example.com\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(
        response_head(&overridden).contains("allow: get, head, post, put, delete, options"),
        "expected the host Allow list to win, got: {}",
        response_head(&overridden)
    );

    container.stop().await.unwrap();
}

/// Test that origin-form `OPTIONS /path` is not answered by the asterisk-form shortcut.
#[tokio::test]
async fn test_options_origin_form_is_not_short_circuited() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let mut config_file = common::create_temp_file();
    common::write_file(webroot_dir.path().join("test.txt"), b"hello").unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"*:80 {
    root "/var/www/ferron"
    http {
        options_allowed_methods "GET, HEAD, OPTIONS"
    }
}
"#,
        )
        .unwrap();

    let container = common::create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();
    let port = container
        .get_host_port_ipv4(testcontainers::core::ContainerPort::Tcp(80))
        .await
        .unwrap();

    let response = send_raw(
        port,
        "OPTIONS /test.txt HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n",
    )
    .await;

    let head = response_head(&response);
    assert!(
        !head.contains("allow: get, head, options"),
        "origin-form OPTIONS must not use the asterisk-form Allow list, got: {head}"
    );
    // The static file stage answers origin-form OPTIONS on an existing file.
    assert_eq!(
        status_code(&response),
        204,
        "expected the pipeline to answer origin-form OPTIONS, got: {head}"
    );

    container.stop().await.unwrap();
}
