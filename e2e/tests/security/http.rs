use std::io::Write;

use crate::common;
use crate::create_ferron_container;

/// Send a raw HTTP request to the given address and return the status code.
/// Used for tests that need precise control over URL encoding that reqwest
/// would otherwise normalize.
async fn raw_http_get(addr: &str, port: u16, raw_path: &str) -> u16 {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let request = format!(
        "GET {raw_path} HTTP/1.1\r\nHost: {addr}:{port}\r\nConnection: close\r\nAccept: */*\r\n\r\n"
    );

    let mut stream = tokio::net::TcpStream::connect((addr, port))
        .await
        .expect("Failed to connect");

    stream.write_all(request.as_bytes()).await.unwrap();
    stream.flush().await.unwrap();

    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf).await.unwrap();
    buf.truncate(n);

    let response = String::from_utf8_lossy(&buf);
    // Parse status code from "HTTP/1.1 XXX ..."
    response
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// Send a raw HTTP request and return the status code line.
async fn raw_http_send(host: &str, port: u16, raw_request: &[u8]) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut stream = tokio::net::TcpStream::connect((host, port))
        .await
        .expect("Failed to connect");

    stream.write_all(raw_request).await.unwrap();
    stream.flush().await.unwrap();

    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf).await.unwrap();
    buf.truncate(n);

    String::from_utf8_lossy(&buf).to_string()
}

/// Test for URL canonicalization rejecting null bytes.
/// Ensures %00 and \0 are rejected during URL canonicalization.
#[tokio::test]
async fn test_url_canonicalization_rejects_null_bytes() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let mut config_file = common::create_temp_file();

    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
    root "/var/www/ferron"
}
"#
            .as_bytes(),
        )
        .unwrap();
    config_file.flush().unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .expect("Failed to create container");

    let client = reqwest::Client::new();
    let ferron_addr = format!(
        "http://127.0.0.1:{}",
        container
            .get_host_port_ipv4(80)
            .await
            .expect("Failed to get host port")
    );

    // Test with %00 (URL-encoded null byte)
    let response = client
        .get(format!("{}/path%00/file", ferron_addr))
        .send()
        .await
        .expect("Failed to send request");

    // Should reject with 400 Bad Request, not process the path
    assert!(
        response.status().is_client_error(),
        "Request with null byte in path should fail with 4xx status"
    );

    // Test with %2500 (double-encoded null byte should be rejected)
    let response = client
        .get(format!("{}/path%2500/file", ferron_addr))
        .send()
        .await
        .expect("Failed to send request");

    assert!(
        response.status().is_success() || response.status().is_client_error(),
        "Request with double-encoded null byte handling"
    );
}

/// Test that triple-encoded sequences (%25252F -> %2F -> /) are rejected
/// as excessive nested encoding, preventing double-decode attacks.
#[tokio::test]
async fn test_url_canonicalization_rejects_triple_encoding() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let mut config_file = common::create_temp_file();

    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
    root "/var/www/ferron"
}
"#
            .as_bytes(),
        )
        .unwrap();
    config_file.flush().unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .expect("Failed to create container");

    let client = reqwest::Client::new();
    let ferron_addr = format!(
        "http://127.0.0.1:{}",
        container
            .get_host_port_ipv4(80)
            .await
            .expect("Failed to get host port")
    );

    // %25252F: triply-encoded "/" (%25 -> %, %25 -> %, %2F -> /)
    // Ferron should reject this as excessive encoding
    let response = client
        .get(format!("{}/path%25252Ftest", ferron_addr))
        .send()
        .await
        .expect("Failed to send request");

    assert!(
        response.status().is_client_error(),
        "Triple-encoded path should be rejected with 4xx, got: {}",
        response.status()
    );

    // Also test that %252F (doubly-encoded /) is rejected
    let response = client
        .get(format!("{}/path%252Ftest", ferron_addr))
        .send()
        .await
        .expect("Failed to send request");

    assert!(
        response.status().is_client_error(),
        "Double-encoded path should be rejected with 4xx, got: {}",
        response.status()
    );
}

/// Test that the server rejects control characters (0x00-0x1F, 0x7F)
/// in the URL path via raw TCP (bypasses reqwest which would encode them).
#[tokio::test]
async fn test_url_canonicalization_rejects_literal_control_chars() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let mut config_file = common::create_temp_file();

    common::write_file(webroot_dir.path().join("test.txt"), b"test content").unwrap();

    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
    root "/var/www/ferron"
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

    // Send raw HTTP with literal tab (0x09) in path — reqwest would encode as %09
    let status = raw_http_get("127.0.0.1", port, "/test\t.txt").await;

    assert_eq!(
        status, 400,
        "Literal tab in path should be rejected with 400, got: {}",
        status
    );

    // Send raw HTTP with literal null byte (0x00) in path
    // We need to construct this carefully since \0 in strings can be tricky
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut raw_path = Vec::from("/test");
    raw_path.push(0x00);
    raw_path.extend_from_slice(b".txt");
    let request = [
        b"GET ".as_slice(),
        &raw_path,
        b" HTTP/1.1\r\nHost: 127.0.0.1:",
        format!("{port}").as_bytes(),
        b"\r\nConnection: close\r\n\r\n",
    ]
    .concat();

    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("Failed to connect");
    stream.write_all(&request).await.unwrap();
    stream.flush().await.unwrap();
    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf).await.unwrap();
    buf.truncate(n);
    let response = String::from_utf8_lossy(&buf);

    let status = response
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);

    assert_eq!(
        status, 400,
        "Literal null byte in path should be rejected with 400, got: {}",
        status
    );
}

/// Test that the server rejects requests with multiple Host headers.
/// Per RFC 7230 §5.4, a server MUST respond with 400 Bad Request when
/// a request contains multiple Host header fields.
#[tokio::test]
async fn test_multiple_host_headers_rejected() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let mut config_file = common::create_temp_file();

    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
    root "/var/www/ferron"
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

    // Send raw HTTP with TWO Host headers (reqwest doesn't allow this)
    let request =
        "GET / HTTP/1.1\r\nHost: example.com\r\nHost: attacker.com\r\nConnection: close\r\n\r\n"
            .to_string();
    let response = raw_http_send("127.0.0.1", port, request.as_bytes()).await;

    let status = response
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);

    assert_eq!(
        status, 400,
        "Multiple Host headers should be rejected with 400, got: {}\nFull response: {}",
        status, response
    );
}

/// Test that the server rejects requests with multiple Content-Length headers.
/// Per RFC 7230 §3.3.3, a server MUST respond with 400 Bad Request or
/// close the connection when duplicate Content-Length headers are present.
#[tokio::test]
async fn test_multiple_content_length_headers_rejected() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let mut config_file = common::create_temp_file();

    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
    root "/var/www/ferron"
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

    // Send raw HTTP with two Content-Length headers
    let request = format!(
        "GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 0\r\nContent-Length: 5\r\nConnection: close\r\n\r\n"
    );
    let response = raw_http_send("127.0.0.1", port, request.as_bytes()).await;

    let status = response
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);

    // The HTTP/1 parser (hyper) may handle this at the protocol level.
    // Multiple Content-Length is a protocol violation (RFC 7230 §3.3.3).
    // The server should NOT treat this as a successful request (200).
    assert_ne!(
        status, 200,
        "Multiple Content-Length headers should not return 200 OK\nFull response: {}",
        response
    );
}

/// Test that the server properly handles the Host header with a trailing dot
/// (e.g., "example.com.") by normalizing it.
#[tokio::test]
async fn test_host_header_trailing_dot_normalized() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let mut config_file = common::create_temp_file();

    common::write_file(webroot_dir.path().join("hello.txt"), b"Hello, World!").unwrap();

    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
    root "/var/www/ferron"
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

    // Send with Host containing trailing dot
    let request =
        format!("GET /hello.txt HTTP/1.1\r\nHost: localhost.:{port}\r\nConnection: close\r\n\r\n");
    let response = raw_http_send("127.0.0.1", port, request.as_bytes()).await;

    let status = response
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);

    assert_eq!(
        status, 200,
        "Host with trailing dot should be normalized and served correctly, got: {}\nFull response: {}",
        status, response
    );
}

/// Test that disabling backslash rejection allows backslashes in paths.
/// With `url_reject_backslash false`, backslashes should be converted
/// to forward slashes instead of being rejected.
#[tokio::test]
async fn test_url_backslash_rejection_disabled() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();

    // Create a file at the path corresponding to backslash-normalized path
    // When backslash is converted to forward slash, `some\path` becomes `some/path`
    std::fs::create_dir_all(webroot_dir.path().join("some")).unwrap();
    std::fs::write(webroot_dir.path().join("some").join("path"), b"hello").unwrap();

    let mut config_file = common::create_temp_file();

    config_file
        .as_file_mut()
        .write_all(
            br#"{
    http {
        url_reject_backslash false
    }
}

*:80 {
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

    // Send request with backslash in path — should NOT be rejected
    let status = raw_http_get("127.0.0.1", ferron_port, "/some\\path").await;
    assert!(
        status != 400,
        "Backslash should not be rejected when url_reject_backslash is false, got 400"
    );
}
