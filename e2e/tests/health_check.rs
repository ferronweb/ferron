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
        .with_env_var("BACKEND_NAME", "health-backend")
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

struct HealthCheckTestContext {
    _backend: ContainerAsync<GenericImage>,
    _ferron: ContainerAsync<GenericImage>,
    base_url: String,
    client: reqwest::Client,
    _network: String,
    _config_file: tempfile::NamedTempFile,
}

impl HealthCheckTestContext {
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

        let network = format!("e2e-test-health-{test_name}");

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
            if let Ok(resp) = client.get(format!("{base_url}/")).send().await
                && resp.status().is_success()
            {
                break;
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

/// passive_check configuration can be parsed correctly.
#[tokio::test]
async fn test_passive_check_config_accepted() {
    let config = br#"
*:80 {
    proxy {
        upstream http://backend:3000
        passive_check {
            max_fails 1
            window "60s"
        }
    }
}
"#;

    let ctx = HealthCheckTestContext::new("passive-config", config).await;

    // If the configuration was accepted, Ferron should start and respond
    let resp = ctx
        .client
        .get(format!("{}/", ctx.base_url))
        .send()
        .await
        .expect("Request failed");

    assert_eq!(
        resp.status(),
        200,
        "Ferron should accept passive_check configuration and forward requests"
    );
}

/// passive_check with max_fails=0 disables the feature.
#[tokio::test]
async fn test_passive_check_disabled() {
    let config = br#"
*:80 {
    proxy {
        upstream http://backend:3000
        passive_check {
            max_fails 0
            window "60s"
        }
    }
}
"#;

    let ctx = HealthCheckTestContext::new("passive-disabled", config).await;

    // Normal request should succeed
    let resp = ctx
        .client
        .get(format!("{}/whoami", ctx.base_url))
        .send()
        .await
        .expect("Request failed");

    assert_eq!(resp.status(), 200, "Expected 200 OK");
    let body = resp.text().await.unwrap_or_default();
    assert_eq!(
        body.trim(),
        "health-backend",
        "Should reach the health-backend backend"
    );
}
