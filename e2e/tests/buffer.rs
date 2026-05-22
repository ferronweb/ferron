use std::io::Write;
#[cfg(unix)]
use std::{fs::Permissions, os::unix::fs::PermissionsExt};
use std::time::Duration;

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};

mod common;

async fn create_backend_container(
    network: &str,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let backend_image = self::common::build_backend_image().await?;
    backend_image
        .with_exposed_port(ContainerPort::Tcp(3000))
        .with_wait_for(WaitFor::Http(Box::new(
            HttpWaitStrategy::new("/")
                .with_port(ContainerPort::Tcp(3000))
                .with_response_matcher(|_| true),
        )))
        .with_network(network)
        .with_hostname("backend")
        .with_env_var("BACKEND_NAME", "buffer-backend")
        .start()
        .await
}

async fn create_ferron_container(
    network: &str,
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
        .with_network(network)
        .with_hostname("ferron")
        .with_mount(Mount::bind_mount(
            config_file.to_string_lossy(),
            "/etc/ferron.conf",
        ))
        .start()
        .await
}

struct BufferTestContext {
    _backend: ContainerAsync<GenericImage>,
    _ferron: ContainerAsync<GenericImage>,
    base_url: String,
    client: reqwest::Client,
    _network: String,
    _config_file: tempfile::NamedTempFile,
}

impl BufferTestContext {
    async fn new(test_name: &str, config_body: &[u8]) -> Self {
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

        let network = format!("e2e-test-buffer-{test_name}");

        let backend = create_backend_container(&network)
            .await
            .expect("Failed to create backend");

        config_file.as_file_mut().write_all(config_body).unwrap();
        config_file.flush().unwrap();

        let ferron = create_ferron_container(&network, config_file.path())
            .await
            .expect("Failed to create Ferron");

        let port = ferron
            .get_host_port_ipv4(ContainerPort::Tcp(80))
            .await
            .expect("Failed to get port");

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap();

        let base_url = format!("http://localhost:{port}");
        for _ in 0..60 {
            if let Ok(resp) = client.get(&format!("{base_url}/")).send().await {
                if resp.status().is_success() {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        Self {
            _backend: backend,
            _ferron: ferron,
            base_url,
            client,
            _network: network,
            _config_file: config_file,
        }
    }
}

/// buffer_response with a positive size buffers the full response body
/// before sending it to the client.
#[tokio::test]
async fn test_buffer_response() {
    let config = br#"
*:80 {
    proxy "http://backend:3000" {
        buffer_response 65536
    }
}
"#;

    let ctx = BufferTestContext::new("resp", config).await;

    let resp = ctx
        .client
        .get(&format!("{}/", ctx.base_url))
        .send()
        .await
        .expect("Request failed");

    assert_eq!(resp.status(), 200, "Expected 200 OK");
    let body = resp.text().await.unwrap_or_default();
    assert_eq!(
        body.trim(),
        "Hello, World!",
        "Response body should match backend content"
    );
}

/// buffer_request with a positive size buffers the full request body
/// before forwarding it to the backend.
#[tokio::test]
async fn test_buffer_request() {
    let config = br#"
*:80 {
    proxy "http://backend:3000" {
        buffer_request 65536
    }
}
"#;

    let ctx = BufferTestContext::new("req", config).await;

    // Send a POST with a body — the backend doesn't handle POST, but the
    // request buffering happens before forwarding; we just verify it doesn't
    // break normal GET requests
    let resp = ctx
        .client
        .get(&format!("{}/whoami", ctx.base_url))
        .send()
        .await
        .expect("Request failed");

    assert_eq!(resp.status(), 200, "Expected 200 OK");
    let body = resp.text().await.unwrap_or_default();
    assert_eq!(
        body.trim(),
        "buffer-backend",
        "Should reach the buffer-backend backend"
    );
}

/// buffer_response 0 disables response buffering.
#[tokio::test]
async fn test_buffer_response_disabled() {
    let config = br#"
*:80 {
    proxy "http://backend:3000" {
        buffer_response 0
    }
}
"#;

    let ctx = BufferTestContext::new("disabled", config).await;

    let resp = ctx
        .client
        .get(&format!("{}/whoami", ctx.base_url))
        .send()
        .await
        .expect("Request failed");

    assert_eq!(resp.status(), 200, "Expected 200 OK");
    let body = resp.text().await.unwrap_or_default();
    assert_eq!(
        body.trim(),
        "buffer-backend",
        "Should still work with buffer_response 0"
    );
}
