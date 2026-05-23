use std::io::Write;
use std::time::Duration;

mod common;

#[tokio::test]
async fn test_early_hints_h1() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let config_file = common::create_temp_file();
    let webroot_dir = common::create_temp_dir();

    std::fs::write(webroot_dir.path().join("index.html"), b"Main Response").unwrap();

    let mut config = std::fs::File::create(config_file.path()).unwrap();
    config
        .write_all(
            br#"
*:80 {
    http {
        h1_enable_early_hints true
    }
    early_hints {
        link "</style.css>; rel=preload; as=style"
    }
    root "/var/www/ferron"
}
"#,
        )
        .unwrap();
    drop(config);

    let ferron = common::create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let port = ferron
        .get_host_port_ipv4(testcontainers::core::ContainerPort::Tcp(80))
        .await
        .unwrap();

    // Use raw TCP to see the 103 response
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    let request = b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
    stream.write_all(request).await.unwrap();
    stream.flush().await.unwrap();

    let mut buf = vec![0u8; 8192];
    let mut response_bytes = Vec::new();
    loop {
        match tokio::time::timeout(Duration::from_secs(2), stream.read(&mut buf)).await {
            Ok(Ok(0)) => break,
            Ok(Ok(n)) => response_bytes.extend_from_slice(&buf[..n]),
            _ => break,
        }
    }
    let response = String::from_utf8_lossy(&response_bytes);
    println!(
        "--- RAW RESPONSE ---\n{}\n--- END RAW RESPONSE ---",
        response
    );

    // Should see both 103 and 200
    assert!(
        response.contains("HTTP/1.1 103 Early Hints"),
        "Should contain 103 Early Hints, got: {}",
        response
    );
    assert!(
        response
            .to_lowercase()
            .contains("link: </style.css>; rel=preload; as=style"),
        "Should contain Link header in 103 response"
    );
    assert!(
        response.contains("HTTP/1.1 200 OK"),
        "Should contain 200 OK after 103"
    );
    assert!(
        response.contains("Main Response"),
        "Should contain main response body"
    );

    ferron.stop().await.unwrap();
}
