use std::io::Write;
#[cfg(unix)]
use std::{fs::Permissions, os::unix::fs::PermissionsExt};

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor},
    runners::AsyncRunner,
};

mod common;

async fn create_ferron_container_proxy(
    network: &str,
    hostname: &str,
    config_file: &std::path::Path,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let ferron_image = self::common::build_ferron_image().await?;
    ferron_image
        .with_exposed_port(ContainerPort::Tcp(80))
        .with_wait_for(WaitFor::seconds(3))
        .with_network(network)
        .with_hostname(hostname)
        .with_mount(Mount::bind_mount(
            config_file.to_string_lossy(),
            "/etc/ferron.conf",
        ))
        .start()
        .await
}

/*#[tokio::test]
async fn test_proxy_header_v1() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_a = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();
    #[cfg(unix)]
    let mut config_b = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();

    #[cfg(not(unix))]
    let mut config_a = tempfile::NamedTempFile::new().unwrap();
    #[cfg(not(unix))]
    let mut config_b = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-proxy-header";

    // Ferron B: Backend, accepts PROXY protocol, blocks 1.2.3.4
    config_b
        .as_file_mut()
        .write_all(
            br#"{
    protocol_proxy true
}
*:80 {
    block "1.2.3.4"
    status 200 {
        body "Success from B"
    }
}
"#,
        )
        .unwrap();

    let _ferron_b = create_ferron_container_proxy(network, "ferron-b", config_b.path())
        .await
        .unwrap();

    // Ferron A: Proxy, sends PROXY protocol v1
    config_a
        .as_file_mut()
        .write_all(
            br#"
*:80 {
    proxy "http://ferron-b:80" {
        proxy_header v1
    }
}
"#,
        )
        .unwrap();

    let ferron_a = create_ferron_container_proxy(network, "ferron-a", config_a.path())
        .await
        .unwrap();

    let port_a = ferron_a
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();

    // Send raw HTTP request with X-Forwarded-For to simulate client IP
    // Wait, proxy_header uses the ACTUAL client IP from the socket.
    // To test this properly, we need Ferron A to think the client is 1.2.3.4.
    // We can use protocol_proxy on Ferron A too!
}*/

#[tokio::test]
async fn test_proxy_header_end_to_end() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_a = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();
    #[cfg(unix)]
    let mut config_b = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();

    #[cfg(not(unix))]
    let mut config_a = tempfile::NamedTempFile::new().unwrap();
    #[cfg(not(unix))]
    let mut config_b = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-proxy-header-e2e";

    // Ferron B: Backend, accepts PROXY protocol, blocks 1.2.3.4
    config_b
        .as_file_mut()
        .write_all(
            br#"{
    http {
        protocol_proxy true
    }
}
*:80 {
    block "1.2.3.4"
    status 200 {
        body "Success from B"
    }
}
"#,
        )
        .unwrap();

    let _ferron_b = create_ferron_container_proxy(network, "ferron-b", config_b.path())
        .await
        .unwrap();

    // Ferron A: Proxy, accepts PROXY protocol, sends PROXY protocol v1
    config_a
        .as_file_mut()
        .write_all(
            br#"{
    http {
        protocol_proxy true
    }
}
*:80 {
    proxy "http://ferron-b:80" {
        proxy_header v1
    }
}
"#,
        )
        .unwrap();

    let ferron_a = create_ferron_container_proxy(network, "ferron-a", config_a.path())
        .await
        .unwrap();

    let port_a = ferron_a
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();

    // Step 1: Send request to A with PROXY v1 header claiming client is 1.2.3.4
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port_a))
        .await
        .unwrap();
    let proxy_header = b"PROXY TCP4 1.2.3.4 127.0.0.1 12345 80\r\n";
    let http_request = b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";

    stream.write_all(proxy_header).await.unwrap();
    stream.write_all(http_request).await.unwrap();
    stream.flush().await.unwrap();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let response = String::from_utf8_lossy(&buf);

    // Ferron A should have received 1.2.3.4, then sent it to Ferron B via PROXY v1.
    // Ferron B should have blocked it.
    if !response.contains("403") {
        let stdout = String::from_utf8(ferron_a.stdout_to_vec().await.unwrap_or_default())
            .unwrap_or_default();
        let stderr = String::from_utf8(ferron_a.stderr_to_vec().await.unwrap_or_default())
            .unwrap_or_default();
        println!(
            "--- Ferron A stdout ---\n{}\n--- Ferron A stderr ---\n{}\n---",
            stdout, stderr
        );

        let stdout_b = String::from_utf8(_ferron_b.stdout_to_vec().await.unwrap_or_default())
            .unwrap_or_default();
        let stderr_b = String::from_utf8(_ferron_b.stderr_to_vec().await.unwrap_or_default())
            .unwrap_or_default();
        println!(
            "--- Ferron B stdout ---\n{}\n--- Ferron B stderr ---\n{}\n---",
            stdout_b, stderr_b
        );
    }
    assert!(
        response.contains("403"),
        "Should be blocked by Ferron B, got: {}",
        response
    );
}
