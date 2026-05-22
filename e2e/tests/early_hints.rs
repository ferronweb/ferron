use std::io::Write;
use std::time::Duration;
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
async fn test_early_hints_h1() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    #[cfg(unix)]
    let webroot_dir = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o777))
        .tempdir()
        .unwrap();
    #[cfg(not(unix))]
    let webroot_dir = tempfile::tempdir().unwrap();

    std::fs::write(webroot_dir.path().join("index.html"), b"Main Response").unwrap();

    config_file
        .as_file_mut()
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

    let ferron = create_ferron_container(webroot_dir.path(), config_file.path()).await.unwrap();

    let port = ferron
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();

    // Use raw TCP to see the 103 response
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
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
    println!("--- RAW RESPONSE ---\n{}\n--- END RAW RESPONSE ---", response);

    // Should see both 103 and 200
    assert!(response.contains("HTTP/1.1 103 Early Hints"), "Should contain 103 Early Hints, got: {}", response);
    assert!(response.to_lowercase().contains("link: </style.css>; rel=preload; as=style"), "Should contain Link header in 103 response");
    assert!(response.contains("HTTP/1.1 200 OK"), "Should contain 200 OK after 103");
    assert!(response.contains("Main Response"), "Should contain main response body");

    ferron.stop().await.unwrap();
}
