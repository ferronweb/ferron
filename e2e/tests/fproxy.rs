use std::io::Write;

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};

mod common;

/// Set up a Ferron container with forward proxy configuration and a backend
/// container on the same Docker network.
struct ForwardProxyTestContext {
    _backend: ContainerAsync<GenericImage>,
    _ferron: ContainerAsync<GenericImage>,
    ferron_port: u16,
    _network: String,
}

impl ForwardProxyTestContext {
    async fn new(test_name: &str, ferron_config: &[u8]) -> Self {
        let _ = rustls::crypto::ring::default_provider().install_default();

        #[cfg(unix)]
        nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

        let network = format!("e2e-test-fproxy-{test_name}");

        // Start the HTTP backend on the custom network
        let backend_image = common::build_backend_image().await.unwrap();
        let backend = backend_image
            .with_exposed_port(ContainerPort::Tcp(3000))
            .with_wait_for(WaitFor::Http(Box::new(
                HttpWaitStrategy::new("/")
                    .with_port(ContainerPort::Tcp(3000))
                    .with_response_matcher(|_| true),
            )))
            .with_network(&network)
            .with_hostname("backend")
            .start()
            .await
            .unwrap();

        // Create config file for Ferron
        #[cfg(unix)]
        let mut config_file = common::create_temp_file();
        #[cfg(not(unix))]
        let mut config_file = tempfile::NamedTempFile::new().unwrap();
        config_file.as_file_mut().write_all(ferron_config).unwrap();
        config_file.flush().unwrap();

        // Start Ferron on the same network
        let ferron_image = common::build_ferron_image().await.unwrap();
        let ferron = ferron_image
            .with_exposed_port(ContainerPort::Tcp(80))
            .with_wait_for(WaitFor::Http(Box::new(
                HttpWaitStrategy::new("/__ready")
                    .with_port(ContainerPort::Tcp(80))
                    .with_response_matcher(|_| true),
            )))
            .with_network(&network)
            .with_hostname("ferron")
            .with_mount(Mount::bind_mount(
                config_file.path().to_string_lossy().to_string(),
                "/etc/ferron.conf",
            ))
            .start()
            .await
            .unwrap();

        let ferron_port = ferron
            .get_host_port_ipv4(ContainerPort::Tcp(80))
            .await
            .unwrap();

        // Wait for Ferron to be fully ready
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        Self {
            _backend: backend,
            _ferron: ferron,
            ferron_port,
            _network: network,
        }
    }
}

/// Test forward proxy CONNECT method to port 80 (HTTP tunnel).
/// Establishes a TCP tunnel through Ferron to the backend, then sends
/// an HTTP GET request through the tunnel.
#[tokio::test]
async fn test_forward_proxy_connect_port_80() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let config = br#"
*:80 {
    forward_proxy {
        allow_domains "*"
        allow_ports 3000
        deny_ips "255.255.255.255"
    }
    root "/var/www/ferron"
}
"#;

    let ctx = ForwardProxyTestContext::new("connect-80", config).await;

    // Step 1: Connect to Ferron and send CONNECT request
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", ctx.ferron_port))
        .await
        .unwrap();

    let connect_request = "CONNECT backend:3000 HTTP/1.1\r\nHost: backend:3000\r\n\r\n".to_string();
    stream.write_all(connect_request.as_bytes()).await.unwrap();
    stream.flush().await.unwrap();

    // Step 2: Read the 200 response (tunnel established)
    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf).await.unwrap();
    buf.truncate(n);
    let response = String::from_utf8_lossy(&buf);

    let stderr = String::from_utf8(ctx._ferron.stderr_to_vec().await.unwrap_or_default())
        .unwrap_or_default();
    let stdout = String::from_utf8(ctx._ferron.stdout_to_vec().await.unwrap_or_default())
        .unwrap_or_default();
    eprintln!(
        "--- Ferron stdout:\n{}\n--- Ferron stderr:\n{}\n--- END",
        stdout, stderr
    );

    assert!(
        response.contains("200") || response.contains("200 OK"),
        "CONNECT tunnel should return 200, got: {}",
        response
    );

    // Step 3: Send HTTP GET through the established tunnel
    let http_request = "GET / HTTP/1.1\r\nHost: backend:3000\r\nConnection: close\r\n\r\n";
    stream.write_all(http_request.as_bytes()).await.unwrap();
    stream.flush().await.unwrap();

    // Step 4: Read the HTTP response from the backend through the tunnel
    let mut tunnel_buf = vec![0u8; 4096];
    let n = stream.read(&mut tunnel_buf).await.unwrap();
    tunnel_buf.truncate(n);
    let tunnel_response = String::from_utf8_lossy(&tunnel_buf);

    assert!(
        tunnel_response.contains("200 OK") || tunnel_response.contains("Hello, World!"),
        "Should receive HTTP response from backend through tunnel, got: {}",
        tunnel_response
    );
}

/// Test forward proxy HTTP forwarding with absolute URI.
/// Sends a GET request with absolute URI (http://backend:3000/)
/// which Ferron should forward to the backend.
#[tokio::test]
async fn test_forward_proxy_http_forwarding() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let config = br#"
*:80 {
    forward_proxy {
        allow_domains "*"
        allow_ports 3000
        deny_ips "255.255.255.255"
    }
    root "/var/www/ferron"
}
"#;

    let ctx = ForwardProxyTestContext::new("http-fwd", config).await;

    // Send absolute URI request to Ferron's forward proxy
    let request =
        "GET http://backend:3000/ HTTP/1.1\r\nHost: backend:3000\r\nConnection: close\r\n\r\n"
            .to_string();

    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", ctx.ferron_port))
        .await
        .unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    stream.flush().await.unwrap();

    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf).await.unwrap();
    buf.truncate(n);
    let response = String::from_utf8_lossy(&buf);

    assert!(
        response.contains("200 OK") || response.contains("Hello, World!"),
        "Forward proxy should forward to backend and return response, got: {}",
        response
    );

    let status = response
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);

    assert_eq!(
        status, 200,
        "Expected 200 from forwarded request, got: {}\nFull response: {}",
        status, response
    );
}
