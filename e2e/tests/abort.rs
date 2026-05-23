mod common;

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

    let webroot_dir = common::create_temp_dir();

    common::write_file(webroot_dir.path().join("index.html"), b"hello").unwrap();

    let config_file = common::create_temp_file();

    common::write_file(
        config_file.path().to_path_buf(),
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

    let container = common::create_ferron_container(webroot_dir.path(), config_file.path())
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

    let webroot_dir = common::create_temp_dir();

    common::write_file(webroot_dir.path().join("index.html"), b"hello").unwrap();

    let config_file = common::create_temp_file();

    common::write_file(
        config_file.path().to_path_buf(),
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

    let container = common::create_ferron_container(webroot_dir.path(), config_file.path())
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

    let webroot_dir = common::create_temp_dir();

    common::write_file(webroot_dir.path().join("index.html"), b"hello").unwrap();

    let config_file = common::create_temp_file();

    common::write_file(
        config_file.path().to_path_buf(),
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

    let container = common::create_ferron_container(webroot_dir.path(), config_file.path())
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
