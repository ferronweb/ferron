use std::io::Write;
use std::time::Duration;
#[cfg(unix)]
use std::{fs::Permissions, os::unix::fs::PermissionsExt};

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor},
    runners::AsyncRunner,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

mod common;

const PROXY_V2_SIG: &[u8] = b"\x0D\x0A\x0D\x0A\x00\x0D\x0A\x51\x55\x49\x54\x0A";

async fn create_ferron_container(
    webroot_dir: &std::path::Path,
    config_file: &std::path::Path,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let ferron_image = self::common::build_ferron_image().await?;
    ferron_image
        .with_exposed_port(ContainerPort::Tcp(80))
        .with_wait_for(WaitFor::seconds(5))
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

async fn wait_until_ready(port: u16, send: &[u8], expected: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        if std::time::Instant::now() > deadline {
            panic!("Timed out waiting for container on port {port}");
        }
        let mut stream = match TcpStream::connect(("127.0.0.1", port)).await {
            Ok(s) => s,
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(500)).await;
                continue;
            }
        };
        if stream.write_all(send).await.is_err() {
            tokio::time::sleep(Duration::from_millis(500)).await;
            continue;
        }
        stream.flush().await.unwrap_or_default();
        let mut buf = vec![0u8; 4096];
        match stream.read(&mut buf).await {
            Ok(0) => {
                tokio::time::sleep(Duration::from_millis(500)).await;
                continue;
            }
            Ok(_) => {
                let resp = String::from_utf8_lossy(&buf);
                if resp.contains(expected) {
                    return;
                }
            }
            Err(_) => {}
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

fn make_v1_header(
    family: &str,
    src_ip: &str,
    dst_ip: &str,
    src_port: u16,
    dst_port: u16,
) -> Vec<u8> {
    format!("PROXY {family} {src_ip} {dst_ip} {src_port} {dst_port}\r\n").into_bytes()
}

fn make_v2_header_tcp4(src_ip: [u8; 4], dst_ip: [u8; 4], src_port: u16, dst_port: u16) -> Vec<u8> {
    let mut header = Vec::with_capacity(28);
    header.extend_from_slice(PROXY_V2_SIG);
    header.extend_from_slice(&[0x21, 0x11]); // v2, PROXY, TCP4
    header.extend_from_slice(&12u16.to_be_bytes()); // addr len
    header.extend_from_slice(&src_ip);
    header.extend_from_slice(&dst_ip);
    header.extend_from_slice(&src_port.to_be_bytes());
    header.extend_from_slice(&dst_port.to_be_bytes());
    header
}

async fn send_proxy_then_http(
    port: u16,
    proxy_header: &[u8],
    http_request: &[u8],
) -> Result<Vec<u8>, String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .map_err(|e| format!("connect: {e}"))?;
    stream
        .write_all(proxy_header)
        .await
        .map_err(|e| format!("write proxy header: {e}"))?;
    stream
        .write_all(http_request)
        .await
        .map_err(|e| format!("write http: {e}"))?;
    stream.flush().await.map_err(|e| format!("flush: {e}"))?;

    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        match stream.read(&mut tmp).await {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
            Err(_) => break,
        }
    }
    Ok(buf)
}

fn parse_status_code(response: &[u8]) -> u16 {
    String::from_utf8_lossy(response)
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

#[tokio::test]
async fn test_proxy_protocol_v1() {
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

    // Create index.html
    std::fs::write(webroot_dir.path().join("index.html"), b"hello proxy").unwrap();

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
            br#"*:80 {
    http {
        protocol_proxy true
    }
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

    let proxy_header = make_v1_header("TCP4", "10.0.0.1", "10.0.0.2", 54321, 80);
    let http_request = b"GET /index.html HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";

    wait_until_ready(
        ferron_port,
        &[&proxy_header[..], &http_request[..]].concat(),
        "200",
    )
    .await;

    let response = send_proxy_then_http(ferron_port, &proxy_header, http_request)
        .await
        .expect("request failed");
    let status = parse_status_code(&response);

    assert_eq!(
        status,
        200,
        "Expected 200, got {status}. Response: {:?}",
        String::from_utf8_lossy(&response)
    );
    assert!(
        String::from_utf8_lossy(&response).contains("hello proxy"),
        "Response body should contain 'hello proxy'"
    );
}

#[tokio::test]
async fn test_proxy_protocol_v2() {
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

    std::fs::write(webroot_dir.path().join("index.html"), b"hello proxy v2").unwrap();

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
            br#"*:80 {
    http {
        protocol_proxy true
    }
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

    let proxy_header = make_v2_header_tcp4([10, 0, 0, 1], [10, 0, 0, 2], 54321, 80);
    let http_request = b"GET /index.html HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";

    wait_until_ready(
        ferron_port,
        &[&proxy_header[..], &http_request[..]].concat(),
        "200",
    )
    .await;

    let response = send_proxy_then_http(ferron_port, &proxy_header, http_request)
        .await
        .expect("request failed");
    let status = parse_status_code(&response);

    assert_eq!(
        status,
        200,
        "Expected 200, got {status}. Response: {:?}",
        String::from_utf8_lossy(&response)
    );
    assert!(
        String::from_utf8_lossy(&response).contains("hello proxy v2"),
        "Response body should contain 'hello proxy v2'"
    );
}

#[tokio::test]
async fn test_proxy_protocol_malformed_rejected() {
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
            br#"*:80 {
    http {
        protocol_proxy true
    }
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

    // Wait for container readiness using a valid PROXY request first
    {
        let proxy_header = make_v1_header("TCP4", "10.0.0.1", "10.0.0.2", 54321, 80);
        let http_request =
            b"GET /index.html HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
        wait_until_ready(
            ferron_port,
            &[&proxy_header[..], &http_request[..]].concat(),
            "200",
        )
        .await;
    }

    // Now send garbage instead of a valid PROXY header
    let mut stream = TcpStream::connect(("127.0.0.1", ferron_port))
        .await
        .expect("connect failed");
    stream
        .write_all(b"GET /index.html HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    stream.flush().await.unwrap();

    let mut buf = [0u8; 1024];
    let result = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buf)).await;

    // When PROXY protocol is enabled and the header is missing/malformed,
    // Ferron should close the connection — read should return 0 or error.
    match result {
        Ok(Ok(0)) => {}  // clean close — expected
        Ok(Err(_)) => {} // connection reset — also expected
        Err(_) => {}     // timeout means connection was NOT dropped — fail
        Ok(Ok(n)) => {
            let resp = String::from_utf8_lossy(&buf[..n]);
            panic!(
                "Expected connection to be dropped, but got response ({} bytes): {}",
                n, resp
            );
        }
    }
}
