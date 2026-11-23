//! Inspired by nginx-tests http_keepalive.t.

use std::io::Write;
use std::time::Duration;

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};

use crate::common;

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
        .with_exposed_port(ContainerPort::Tcp(8889))
        .with_wait_for(WaitFor::Http(Box::new(
            HttpWaitStrategy::new("/%")
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

struct KeepaliveTestContext {
    _backend: ContainerAsync<GenericImage>,
    _ferron: ContainerAsync<GenericImage>,
    base_url: String,
    metrics_url: String,
    client: reqwest::Client,
    _config_file: tempfile::NamedTempFile,
}

impl KeepaliveTestContext {
    async fn new(test_name: &str, config: &[u8]) -> Self {
        let _ = rustls::crypto::ring::default_provider().install_default();

        let mut config_file = self::common::create_temp_file();

        let network = format!("e2e-test-keepalive-{}", test_name);

        let backend = create_backend_container(&network).await.unwrap();

        config_file.as_file_mut().write_all(config).unwrap();

        let ferron = create_ferron_container(&network, config_file.path())
            .await
            .unwrap();

        let port = ferron
            .get_host_port_ipv4(ContainerPort::Tcp(80))
            .await
            .unwrap();

        let metrics_port = ferron
            .get_host_port_ipv4(ContainerPort::Tcp(8889))
            .await
            .unwrap();

        let client = reqwest::Client::new();
        let base_url = format!("http://localhost:{}", port);
        let metrics_url = format!("http://localhost:{}/metrics", metrics_port);

        Self {
            _backend: backend,
            _ferron: ferron,
            base_url,
            metrics_url,
            client,
            _config_file: config_file,
        }
    }

    async fn wait_for_metrics(&self, condition: impl Fn(&str) -> bool) -> String {
        let mut last_body = String::new();
        for _ in 0..60 {
            if let Ok(resp) = self.client.get(&self.metrics_url).send().await
                && resp.status().is_success()
                && let Ok(body) = resp.text().await
            {
                last_body = body.clone();
                if condition(&body) {
                    return body;
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        last_body
    }
}

#[tokio::test]
async fn test_keepalive_reuses_connection() {
    let ctx = KeepaliveTestContext::new(
        "reuse",
        br#"
*:80 {
  observability {
    provider prometheus
    endpoint_listen "0.0.0.0:8889"
  }
  proxy "http://backend:3000" {
    keepalive true
  }
}
"#,
    )
    .await;

    let resp1 = ctx
        .client
        .get(format!("{}/whoami", ctx.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp1.status(), 200);

    let resp2 = ctx
        .client
        .get(format!("{}/whoami", ctx.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp2.status(), 200);

    let metrics_body = ctx
        .wait_for_metrics(|body| {
            body.contains("ferron_proxy_requests")
                && body.contains("ferron_proxy_connection_reused=\"1\"")
        })
        .await;

    assert!(
        metrics_body.contains("ferron_proxy_connection_reused=\"1\""),
        "Connection was not reused according to metrics. Metrics body: {}",
        metrics_body
    );
}

#[tokio::test]
async fn test_keepalive_disabled_creates_separate_connections() {
    let ctx = KeepaliveTestContext::new(
        "disabled",
        br#"
*:80 {
  observability {
    provider prometheus
    endpoint_listen "0.0.0.0:8889"
  }
  proxy "http://backend:3000" {
    keepalive false
  }
}
"#,
    )
    .await;

    let resp1 = ctx
        .client
        .get(format!("{}/whoami", ctx.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp1.status(), 200);

    let resp2 = ctx
        .client
        .get(format!("{}/whoami", ctx.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp2.status(), 200);

    // Wait for metrics — all connections should show connection_reused="0"
    let metrics_body = ctx
        .wait_for_metrics(|body| body.contains("ferron_proxy_requests"))
        .await;

    let reused_count = metrics_body
        .lines()
        .filter(|line| line.contains("ferron_proxy_connection_reused=\"1\""))
        .count();

    assert_eq!(
        reused_count, 0,
        "Expected no connection reuse when keepalive is disabled. Metrics:\n{}",
        metrics_body
    );
}
