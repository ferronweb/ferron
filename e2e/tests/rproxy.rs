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
        .with_hostname("backend")
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
        let backend = create_backend_container(&network, cert_dir.path())
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

        // Fix test flakiness, maybe caused by networking issues?
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

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
