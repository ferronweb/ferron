use std::io::Write;
#[cfg(unix)]
use std::{fs::Permissions, os::unix::fs::PermissionsExt};

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

async fn raw_http_get_with_header(
    addr: &str,
    port: u16,
    path: &str,
    extra_headers: &[(&str, &str)],
) -> u16 {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut request =
        format!("GET {path} HTTP/1.1\r\nHost: {addr}:{port}\r\nConnection: close\r\n");
    for (name, value) in extra_headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");

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

/// Test that `client_ip_from_header` with `trusted_proxy` correctly
/// rewrites the client IP from X-Forwarded-For for IP access control.
#[tokio::test]
async fn test_client_ip_from_x_forwarded_for() {
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
            br#"{
    client_ip_from_header x-forwarded-for {
        trusted_proxy "0.0.0.0/0"
    }
}

*:80 {
    block "1.2.3.4"
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

    // Request without X-Forwarded-For: client IP is the actual Docker bridge IP
    // which is NOT 1.2.3.4 — so request succeeds
    let status = raw_http_get_with_header("127.0.0.1", ferron_port, "/index.html", &[]).await;
    assert_eq!(
        status, 200,
        "Expected 200 without X-Forwarded-For (client IP is not blocked), got {status}"
    );

    // Request with X-Forwarded-For: 1.2.3.4: client IP is rewritten to 1.2.3.4
    // which IS blocked — so request returns 403
    let status = raw_http_get_with_header(
        "127.0.0.1",
        ferron_port,
        "/index.html",
        &[("X-Forwarded-For", "1.2.3.4")],
    )
    .await;
    assert_eq!(
        status, 403,
        "Expected 403 with X-Forwarded-For: 1.2.3.4 (client IP should be blocked), got {status}"
    );

    // Request with X-Forwarded-For: 5.6.7.8: client IP is rewritten to 5.6.7.8
    // which is NOT blocked — so request succeeds
    let status = raw_http_get_with_header(
        "127.0.0.1",
        ferron_port,
        "/index.html",
        &[("X-Forwarded-For", "5.6.7.8")],
    )
    .await;
    assert_eq!(
        status, 200,
        "Expected 200 with X-Forwarded-For: 5.6.7.8 (client IP is not blocked), got {status}"
    );
}

/// Test that `client_ip_from_header` with `forwarded` header works.
#[tokio::test]
async fn test_client_ip_from_forwarded_header() {
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
            br#"{
    client_ip_from_header forwarded {
        trusted_proxy "0.0.0.0/0"
    }
}

*:80 {
    block "10.0.0.1"
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

    // Request with Forwarded: for=10.0.0.1: client IP is rewritten to 10.0.0.1
    // which IS blocked — so request returns 403
    let status = raw_http_get_with_header(
        "127.0.0.1",
        ferron_port,
        "/index.html",
        &[("Forwarded", "for=10.0.0.1")],
    )
    .await;
    assert_eq!(
        status, 403,
        "Expected 403 with Forwarded: for=10.0.0.1 (client IP should be blocked), got {status}"
    );

    // Request with Forwarded: for=192.168.1.1: client IP is rewritten to 192.168.1.1
    // which is NOT blocked — so request succeeds
    let status = raw_http_get_with_header(
        "127.0.0.1",
        ferron_port,
        "/index.html",
        &[("Forwarded", "for=192.168.1.1")],
    )
    .await;
    assert_eq!(
        status, 200,
        "Expected 200 with Forwarded: for=192.168.1.1 (client IP is not blocked), got {status}"
    );
}

/// Test that untrusted proxies are ignored: when connecting peer is not
/// in the trusted_proxy list, X-Forwarded-For header is NOT trusted.
#[tokio::test]
async fn test_client_ip_untrusted_proxy_ignored() {
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
            br#"{
    client_ip_from_header x-forwarded-for {
        trusted_proxy "192.168.0.0/16"
    }
}

*:80 {
    block "1.2.3.4"
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

    // The connecting peer is the Docker bridge IP (e.g., 172.17.0.1), which is NOT
    // in the trusted_proxy range (192.168.0.0/16). So X-Forwarded-For should be ignored.
    // The client IP remains the Docker bridge IP, which is NOT 1.2.3.4, so request succeeds.
    let status = raw_http_get_with_header(
        "127.0.0.1",
        ferron_port,
        "/index.html",
        &[("X-Forwarded-For", "1.2.3.4")],
    )
    .await;
    assert_eq!(
        status, 200,
        "Expected 200 when connecting peer is not a trusted proxy (X-Forwarded-For ignored), got {status}"
    );
}
