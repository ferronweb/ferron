#[cfg(unix)]
use std::{fs::Permissions, os::unix::fs::PermissionsExt};
use std::{io::Write, path::Path};

use futures_util::{SinkExt, StreamExt};
use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

mod common;

async fn create_backend_container(
    network: &str,
    cert_dir: &Path,
    hostname: &str,
    backend_name: &str,
    unstable_fails: u32,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let backend_image = self::common::build_backend_image().await?;
    backend_image
        .with_exposed_port(ContainerPort::Tcp(3000))
        .with_exposed_port(ContainerPort::Tcp(3001))
        .with_wait_for(WaitFor::Http(Box::new(
            HttpWaitStrategy::new("/")
                .with_port(ContainerPort::Tcp(3000))
                .with_response_matcher(|_| true),
        )))
        .with_network(network)
        .with_hostname(hostname)
        .with_env_var("BACKEND_NAME", backend_name)
        .with_env_var("UNSTABLE_FAILS", unstable_fails.to_string())
        .with_mount(Mount::bind_mount(
            cert_dir.to_string_lossy().to_string(),
            "/etc/certs",
        ))
        .start()
        .await
}

async fn create_ferron_container(
    network: &str,
    config_file: &Path,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let ferron_image = self::common::build_ferron_image().await?;
    ferron_image
        .with_exposed_port(ContainerPort::Tcp(80))
        .with_network(network)
        .with_hostname("ferron")
        .with_mount(Mount::bind_mount(
            config_file.to_string_lossy().to_string(),
            "/etc/ferron.conf",
        ))
        .start()
        .await
}

async fn wait_for_ferron_tcp_ready(port: u16) {
    for _ in 0..20 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return;
        }

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    panic!("Ferron test container did not become reachable on port {}", port);
}

async fn wait_for_ferron_ready_route(port: u16, ferron: &ContainerAsync<GenericImage>) {
    let client = reqwest::Client::new();
    let url = format!("http://localhost:{port}/__ready");

    for _ in 0..100 {
        if let Ok(response) = client.get(&url).send().await {
            if response.status() == reqwest::StatusCode::NO_CONTENT {
                return;
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    let stdout = String::from_utf8(ferron.stdout_to_vec().await.unwrap_or_default())
        .unwrap_or_default();
    let stderr = String::from_utf8(ferron.stderr_to_vec().await.unwrap_or_default())
        .unwrap_or_default();

    panic!(
        "Ferron test container did not become ready at {}\nstdout:\n{}\n\nstderr:\n{}",
        url, stdout, stderr
    );
}

struct RProxyTestContext {
    _backend: ContainerAsync<GenericImage>,
    _ferron: ContainerAsync<GenericImage>,
    base_url: String,
    ws_url: String,
    client: reqwest::Client,
    // Keep these alive to prevent cleanup
    _cert_dir: tempfile::TempDir,
    _config_file: tempfile::NamedTempFile,
}

impl RProxyTestContext {
    async fn new(test_name: &str) -> Self {
        let _ = rustls::crypto::ring::default_provider().install_default();

        #[cfg(unix)]
        nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

        #[cfg(unix)]
        let cert_dir = tempfile::Builder::new()
            .permissions(Permissions::from_mode(0o777))
            .tempdir()
            .unwrap();
        #[cfg(unix)]
        let mut config_file = tempfile::Builder::new()
            .permissions(Permissions::from_mode(0o666))
            .tempfile()
            .unwrap();

        #[cfg(not(unix))]
        let cert_dir = tempfile::tempdir().unwrap();
        #[cfg(not(unix))]
        let mut config_file = tempfile::NamedTempFile::new().unwrap();

        // Generate certs for backend
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        std::fs::write(cert_dir.path().join("server.crt"), cert.cert.pem()).unwrap();
        std::fs::write(
            cert_dir.path().join("server.key"),
            cert.signing_key.serialize_pem(),
        )
        .unwrap();

        let network = format!("e2e-test-rproxy-{}", test_name);

        // Start backend
        let backend = create_backend_container(&network, cert_dir.path(), "backend", "backend", 0)
            .await
            .unwrap();

        // Write Ferron config
        config_file
            .as_file_mut()
            .write_all(
                br#"
*:80 {
  proxy "http://backend:3000"

  match HEADER {
    request.uri.path == "/header"
  }

  match TLS {
    request.uri.path == "/tls"
  }

  if HEADER {
    proxy "http://backend:3000" {
      request_header "X-Some-Header" "something"
    }
  }

  if TLS {
    proxy "https://backend:3001" {
      no_verification true
    }
  }
}
"#,
            )
            .unwrap();

        // Start Ferron
        let ferron = create_ferron_container(&network, config_file.path())
            .await
            .unwrap();

        let port = ferron
            .get_host_port_ipv4(ContainerPort::Tcp(80))
            .await
            .unwrap();
        let client = reqwest::Client::new();
        let base_url = format!("http://localhost:{}", port);
        let ws_url = format!("ws://localhost:{}/echo", port);

        wait_for_ferron_tcp_ready(port).await;

        Self {
            _backend: backend,
            _ferron: ferron,
            base_url,
            ws_url,
            client,
            _cert_dir: cert_dir,
            _config_file: config_file,
        }
    }
}

struct CircuitBreakerTestContext {
    _backends: Vec<ContainerAsync<GenericImage>>,
    _ferron: ContainerAsync<GenericImage>,
    base_url: String,
    client: reqwest::Client,
    _cert_dir: tempfile::TempDir,
    _config_file: tempfile::NamedTempFile,
}

impl CircuitBreakerTestContext {
    async fn new(test_name: &str, config: &[u8], backends: &[(&str, &str, u32)]) -> Self {
        let _ = rustls::crypto::ring::default_provider().install_default();

        #[cfg(unix)]
        nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

        #[cfg(unix)]
        let cert_dir = tempfile::Builder::new()
            .permissions(Permissions::from_mode(0o777))
            .tempdir()
            .unwrap();
        #[cfg(unix)]
        let mut config_file = tempfile::Builder::new()
            .permissions(Permissions::from_mode(0o666))
            .tempfile()
            .unwrap();

        #[cfg(not(unix))]
        let cert_dir = tempfile::tempdir().unwrap();
        #[cfg(not(unix))]
        let mut config_file = tempfile::NamedTempFile::new().unwrap();

        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        std::fs::write(cert_dir.path().join("server.crt"), cert.cert.pem()).unwrap();
        std::fs::write(
            cert_dir.path().join("server.key"),
            cert.signing_key.serialize_pem(),
        )
        .unwrap();

        let network = format!("e2e-test-rproxy-cb-{}", test_name);

        let mut backend_containers = Vec::new();
        for (hostname, backend_name, unstable_fails) in backends {
            backend_containers.push(
                create_backend_container(
                    &network,
                    cert_dir.path(),
                    hostname,
                    backend_name,
                    *unstable_fails,
                )
                .await
                .unwrap(),
            );
        }

        config_file.as_file_mut().write_all(config).unwrap();

        let ferron = create_ferron_container(&network, config_file.path())
            .await
            .unwrap();

        let port = ferron
            .get_host_port_ipv4(ContainerPort::Tcp(80))
            .await
            .unwrap();

        wait_for_ferron_ready_route(port, &ferron).await;

        Self {
            _backends: backend_containers,
            _ferron: ferron,
            base_url: format!("http://localhost:{}", port),
            client: reqwest::Client::new(),
            _cert_dir: cert_dir,
            _config_file: config_file,
        }
    }

    async fn ferron_logs(&self) -> String {
        let stdout = String::from_utf8(self._ferron.stdout_to_vec().await.unwrap_or_default())
            .unwrap_or_default();
        let stderr = String::from_utf8(self._ferron.stderr_to_vec().await.unwrap_or_default())
            .unwrap_or_default();

        format!("stdout:\n{stdout}\n\nstderr:\n{stderr}")
    }

    async fn get(&self, path: &str) -> reqwest::Response {
        let url = format!("{}/{}", self.base_url, path.trim_start_matches('/'));

        for _ in 0..5 {
            match self.client.get(&url).send().await {
                Ok(response) => return response,
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(100)).await,
            }
        }

        panic!(
            "Request to {} failed repeatedly. Ferron logs:\n{}",
            url,
            self.ferron_logs().await
        );
    }
}

#[tokio::test]
async fn test_basic_reverse_proxy() {
    let ctx = RProxyTestContext::new("basic").await;
    let response = ctx
        .client
        .get(format!("{}/", ctx.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), "Hello, World!");
}

#[tokio::test]
async fn test_websocket() {
    let ctx = RProxyTestContext::new("websocket").await;
    let (mut ws_stream, _) = connect_async(&ctx.ws_url).await.expect("Failed to connect");
    ws_stream
        .send(Message::Text("WEBSOCKET TEST".into()))
        .await
        .expect("Failed to send");
    if let Some(msg) = ws_stream.next().await {
        let msg = msg.expect("Failed to receive");
        if let Message::Text(text) = msg {
            assert_eq!(text, "WEBSOCKET TEST");
        } else {
            panic!("Received non-text message");
        }
    } else {
        panic!("Stream ended");
    }
}

#[tokio::test]
async fn test_x_forwarded_for() {
    let ctx = RProxyTestContext::new("xff").await;
    let response = ctx
        .client
        .get(format!("{}/ip", ctx.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let ip = response.text().await.unwrap();
    assert!(!ip.is_empty(), "IP should not be empty");
    assert!(
        ip.parse::<std::net::IpAddr>().is_ok(),
        "Response should be a valid IP address: {}",
        ip
    );
    assert_ne!(
        ip.parse::<std::net::IpAddr>().unwrap(),
        ctx._ferron.get_bridge_ip_address().await.unwrap()
    );
}

#[tokio::test]
async fn test_hostname_forwarding() {
    let ctx = RProxyTestContext::new("hostname").await;
    // If we manually set Host header to "ferron", we should see "ferron".
    // This verifies that Ferron forwards the Host header it receives (when not using TLS for backend).
    let response = ctx
        .client
        .get(format!("{}/hostname", ctx.base_url))
        .header("Host", "ferron")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), "ferron");
}

#[tokio::test]
async fn test_custom_header() {
    let ctx = RProxyTestContext::new("header").await;
    let response = ctx
        .client
        .get(format!("{}/header", ctx.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), "something");
}

#[tokio::test]
async fn test_bad_gateway() {
    let ctx = RProxyTestContext::new("502").await;
    // /unsafe destroys socket immediately, causing a backend error which Ferron sees as network error -> 502
    let response = ctx
        .client
        .get(format!("{}/unsafe", ctx.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn test_tls_backend() {
    let ctx = RProxyTestContext::new("tls").await;
    let response = ctx
        .client
        .get(format!("{}/tls", ctx.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), "Hello, World!");
}

#[tokio::test]
async fn test_circuit_breaker_skips_backend_after_transport_failure() {
    let ctx = CircuitBreakerTestContext::new(
        "transport",
        br#"
*:80 {
  match READY {
    request.uri.path == "/__ready"
  }

  if READY {
    status 204
  }

  proxy {
    upstream "http://backend-ok:3999"
    upstream "http://backend-ok:3000"

    algorithm round_robin
    retry_connection false

    circuit_breaker {
      max_fails 1
      window "30s"
      open_duration "30s"
      consecutive_passes 1
    }
  }
}
"#,
        &[("backend-ok", "backend-ok", 0)],
    )
    .await;

    let first = ctx.get("/whoami").await;
    assert_eq!(
        first.status(),
        reqwest::StatusCode::BAD_GATEWAY,
        "Ferron logs:\n{}",
        ctx.ferron_logs().await
    );

    let second = ctx.get("/whoami").await;
    let second_status = second.status();
    let second_body = second.text().await.unwrap();
    assert_eq!(
        second_status,
        reqwest::StatusCode::OK,
        "Unexpected second response body: {second_body}\nFerron logs:\n{}",
        ctx.ferron_logs().await
    );
    assert_eq!(
        second_body,
        "backend-ok",
        "Ferron logs:\n{}",
        ctx.ferron_logs().await
    );

    let third = ctx.get("/whoami").await;
    let third_status = third.status();
    let third_body = third.text().await.unwrap();
    assert_eq!(
        third_status,
        reqwest::StatusCode::OK,
        "Unexpected third response body: {third_body}\nFerron logs:\n{}",
        ctx.ferron_logs().await
    );
    assert_eq!(
        third_body,
        "backend-ok",
        "Ferron logs:\n{}",
        ctx.ferron_logs().await
    );
}

#[tokio::test]
async fn test_circuit_breaker_half_open_recovery() {
    let ctx = CircuitBreakerTestContext::new(
        "recovery",
        br#"
*:80 {
  match READY {
    request.uri.path == "/__ready"
  }

  if READY {
    status 204
  }

  proxy {
    upstream "http://backend-flaky:3000"
    upstream "http://backend-ok:3000"

    algorithm round_robin
    retry_connection false

    circuit_breaker {
      max_fails 1
      window "30s"
      open_duration "1s"
      consecutive_passes 1
    }
  }
}
"#,
        &[("backend-flaky", "backend-flaky", 1), ("backend-ok", "backend-ok", 0)],
    )
    .await;

    let first = ctx.get("/unstable").await;
    let first_status = first.status();
    let first_body = first.text().await.unwrap();
    assert_eq!(
        first_status,
        reqwest::StatusCode::SERVICE_UNAVAILABLE,
        "Unexpected first response body: {first_body}\nFerron logs:\n{}",
        ctx.ferron_logs().await
    );
    assert_eq!(
        first_body,
        "unstable:backend-flaky",
        "Ferron logs:\n{}",
        ctx.ferron_logs().await
    );

    let second = ctx.get("/unstable").await;
    let second_status = second.status();
    let second_body = second.text().await.unwrap();
    assert_eq!(
        second_status,
        reqwest::StatusCode::OK,
        "Unexpected second response body: {second_body}\nFerron logs:\n{}",
        ctx.ferron_logs().await
    );
    assert_eq!(
        second_body,
        "backend-ok",
        "Ferron logs:\n{}",
        ctx.ferron_logs().await
    );

    let third = ctx.get("/unstable").await;
    let third_status = third.status();
    let third_body = third.text().await.unwrap();
    assert_eq!(
        third_status,
        reqwest::StatusCode::OK,
        "Unexpected third response body: {third_body}\nFerron logs:\n{}",
        ctx.ferron_logs().await
    );
    assert_eq!(
        third_body,
        "backend-ok",
        "Ferron logs:\n{}",
        ctx.ferron_logs().await
    );

    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;

    let fourth = ctx.get("/unstable").await;
    let fourth_status = fourth.status();
    let fourth_body = fourth.text().await.unwrap();
    assert_eq!(
        fourth_status,
        reqwest::StatusCode::OK,
        "Unexpected fourth response body: {fourth_body}\nFerron logs:\n{}",
        ctx.ferron_logs().await
    );
    assert_eq!(
        fourth_body,
        "backend-ok",
        "Ferron logs:\n{}",
        ctx.ferron_logs().await
    );

    let fifth = ctx.get("/unstable").await;
    let fifth_status = fifth.status();
    let fifth_body = fifth.text().await.unwrap();
    assert_eq!(
        fifth_status,
        reqwest::StatusCode::OK,
        "Unexpected fifth response body: {fifth_body}\nFerron logs:\n{}",
        ctx.ferron_logs().await
    );
    assert_eq!(
        fifth_body,
        "backend-flaky",
        "Ferron logs:\n{}",
        ctx.ferron_logs().await
    );
}
