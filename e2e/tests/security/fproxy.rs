use std::io::Write;

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::common;

/// Ferron + HTTP backend on an isolated Docker network, so forward-proxy
/// requests can actually be routed to (or blocked from) a real upstream.
///
/// This mirrors the harness in `e2e/tests/fproxy.rs`: the backend is reachable
/// from Ferron as `backend:<port>`, and the test talks to Ferron over raw TCP
/// so it can issue `CONNECT` and absolute-URI requests that `reqwest` cannot
/// express against the proxy listener itself.
struct ForwardProxySecurityContext {
    _backend: ContainerAsync<GenericImage>,
    _ferron: ContainerAsync<GenericImage>,
    ferron_port: u16,
    _network: String,
}

impl ForwardProxySecurityContext {
    async fn new(test_name: &str, ferron_config: &[u8]) -> Self {
        // Required by testcontainers' HTTP wait strategy (reqwest with
        // rustls-no-provider needs a process-wide crypto provider).
        let _ = rustls::crypto::ring::default_provider().install_default();

        #[cfg(unix)]
        nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

        let network = format!("e2e-test-sec-fproxy-{test_name}");

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

        #[cfg(unix)]
        let mut config_file = common::create_temp_file();
        #[cfg(not(unix))]
        let mut config_file = tempfile::NamedTempFile::new().unwrap();
        config_file.as_file_mut().write_all(ferron_config).unwrap();
        config_file.flush().unwrap();

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

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        Self {
            _backend: backend,
            _ferron: ferron,
            ferron_port,
            _network: network,
        }
    }

    async fn tcp(&self) -> tokio::net::TcpStream {
        tokio::net::TcpStream::connect(("127.0.0.1", self.ferron_port))
            .await
            .unwrap()
    }
}

/// Send one raw HTTP request and read until the response head is complete.
async fn send_raw(stream: &mut tokio::net::TcpStream, request: &str) -> String {
    stream.write_all(request.as_bytes()).await.unwrap();
    stream.flush().await.unwrap();
    read_head(stream).await
}

/// Read until `\r\n\r\n` (end of response head) or EOF, with a timeout so a
/// hung tunnel fails the test instead of hanging it.
async fn read_head(stream: &mut tokio::net::TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = vec![0u8; 4096];
    let read = async {
        loop {
            let n = stream.read(&mut chunk).await.unwrap();
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(10), read)
        .await
        .expect("timed out waiting for Ferron response");
    String::from_utf8_lossy(&buf).into_owned()
}

fn status_code(response_head: &str) -> u16 {
    response_head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0)
}

/// DNS-rebinding / SSRF protection: a hostname (or literal IP) that resolves
/// to a denied IP must be rejected with 403 on both the CONNECT tunnel path
/// and the absolute-URI forwarding path — never tunnelled, forwarded, or
/// silently passed through to the normal request pipeline.
///
/// The backend lives on a Docker network in `172.16.0.0/12`, which is part of
/// the default `deny_ips` list, so with no `deny_ips` override every request
/// to `backend:3000` exercises the "resolved IP is denied" path. Literal and
/// obfuscated loopback forms exercise the `inet_aton` pre-resolution check.
#[tokio::test]
async fn test_forward_proxy_dns_rebinding_protection() {
    let config = br#"
*:80 {
    forward_proxy {
        allow_domains "*"
        allow_ports 3000
    }
    root "/var/www/ferron"
}
"#;

    let ctx = ForwardProxySecurityContext::new("rebind", config).await;

    // CONNECT to a hostname resolving to a denied (RFC 1918) IP must be 403.
    let mut stream = ctx.tcp().await;
    let response = send_raw(
        &mut stream,
        "CONNECT backend:3000 HTTP/1.1\r\nHost: backend:3000\r\n\r\n",
    )
    .await;
    assert_eq!(
        status_code(&response),
        403,
        "CONNECT to rebinding hostname should be denied with 403, got: {response}"
    );

    // Same enforcement on the absolute-URI (plain HTTP forwarding) path.
    let mut stream = ctx.tcp().await;
    let response = send_raw(
        &mut stream,
        "GET http://backend:3000/ HTTP/1.1\r\nHost: backend:3000\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert_eq!(
        status_code(&response),
        403,
        "Forwarded request to rebinding hostname should be denied with 403, got: {response}"
    );

    // Literal loopback IP must be denied even though the domain allowlist is "*".
    let mut stream = ctx.tcp().await;
    let response = send_raw(
        &mut stream,
        "CONNECT 127.0.0.1:3000 HTTP/1.1\r\nHost: 127.0.0.1:3000\r\n\r\n",
    )
    .await;
    assert_eq!(
        status_code(&response),
        403,
        "CONNECT to loopback IP literal should be denied with 403, got: {response}"
    );

    // Obfuscated loopback (hex form parsed by inet_aton) must be denied too.
    let mut stream = ctx.tcp().await;
    let response = send_raw(
        &mut stream,
        "CONNECT 0x7f.0.0.1:3000 HTTP/1.1\r\nHost: 0x7f.0.0.1:3000\r\n\r\n",
    )
    .await;
    assert_eq!(
        status_code(&response),
        403,
        "CONNECT to obfuscated loopback IP should be denied with 403, got: {response}"
    );
}

/// `allow_ports` replaces the built-in default (`80, 443`) instead of adding
/// to it: with only `allow_ports 3000`, port 3000 must tunnel/forward while
/// ports 80 and 443 are rejected with 403.
///
/// `deny_ips` is overridden here so the backend subnet is reachable and the
/// port-3000 requests can actually complete the tunnel.
#[tokio::test]
async fn test_forward_proxy_allow_ports_not_additive() {
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

    let ctx = ForwardProxySecurityContext::new("ports", config).await;

    // Explicitly allowed port: tunnel establishes and relays the backend.
    let mut stream = ctx.tcp().await;
    let response = send_raw(
        &mut stream,
        "CONNECT backend:3000 HTTP/1.1\r\nHost: backend:3000\r\n\r\n",
    )
    .await;
    assert_eq!(
        status_code(&response),
        200,
        "CONNECT to explicitly allowed port 3000 should return 200, got: {response}"
    );
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: backend:3000\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    stream.flush().await.unwrap();
    let tunneled = read_head(&mut stream).await;
    assert!(
        tunneled.contains("200 OK") || tunneled.contains("Hello, World!"),
        "Should receive backend response through tunnel, got: {tunneled}"
    );

    // Non-configured ports must NOT be implicitly allowed.
    for port in [80, 443] {
        let mut stream = ctx.tcp().await;
        let response = send_raw(
            &mut stream,
            &format!("CONNECT backend:{port} HTTP/1.1\r\nHost: backend:{port}\r\n\r\n"),
        )
        .await;
        assert_eq!(
            status_code(&response),
            403,
            "CONNECT to non-configured port {port} should be denied with 403, got: {response}"
        );
    }

    // Same port list applies to absolute-URI forwarding.
    let mut stream = ctx.tcp().await;
    let response = send_raw(
        &mut stream,
        "GET http://backend:3000/ HTTP/1.1\r\nHost: backend:3000\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert_eq!(
        status_code(&response),
        200,
        "Forwarded request to allowed port 3000 should return 200, got: {response}"
    );

    let mut stream = ctx.tcp().await;
    let response = send_raw(
        &mut stream,
        "GET http://backend:80/ HTTP/1.1\r\nHost: backend:80\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert_eq!(
        status_code(&response),
        403,
        "Forwarded request to non-configured port 80 should be denied with 403, got: {response}"
    );
}
