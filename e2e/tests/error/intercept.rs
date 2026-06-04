use std::io::Write;
use std::path::Path;
use std::time::Duration;

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};

use crate::common;

async fn create_backend_container(
    network: &str,
    hostname: &str,
    unstable_fails: u32,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let backend_image = common::build_backend_image().await?;
    backend_image
        .with_exposed_port(ContainerPort::Tcp(3000))
        .with_wait_for(WaitFor::Http(Box::new(
            HttpWaitStrategy::new("/")
                .with_port(ContainerPort::Tcp(3000))
                .with_response_matcher(|_| true),
        )))
        .with_network(network)
        .with_hostname(hostname)
        .with_env_var("BACKEND_NAME", "backend")
        .with_env_var("UNSTABLE_FAILS", unstable_fails.to_string())
        .start()
        .await
}

async fn create_ferron_container(
    network: &str,
    config_file: &Path,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let ferron_image = common::build_ferron_image().await?;
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
            config_file.to_string_lossy().to_string(),
            "/etc/ferron.conf",
        ))
        .start()
        .await
}

struct InterceptTestContext {
    _backend: ContainerAsync<GenericImage>,
    _ferron: ContainerAsync<GenericImage>,
    base_url: String,
    client: reqwest::Client,
    _network: String,
    _config_file: tempfile::NamedTempFile,
}

impl InterceptTestContext {
    async fn new(test_name: &str, config_body: &[u8], unstable_fails: u32) -> Self {
        let _ = rustls::crypto::ring::default_provider().install_default();

        let mut config_file = common::create_temp_file();

        let network = format!("e2e-test-intercept-{test_name}");

        let backend = create_backend_container(&network, "backend", unstable_fails)
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

#[tokio::test]
async fn test_intercept_errors_true() {
    let config = br#"
*:80 {
    proxy "http://backend:3000" {
        intercept_errors true
    }
}
"#;

    let ctx = InterceptTestContext::new("true", config, 1).await;

    let resp = ctx
        .client
        .get(format!("{}/unstable", ctx.base_url))
        .send()
        .await
        .expect("request failed");

    assert_eq!(resp.status(), 503, "Expected 503 Service Unavailable");
    let body = resp.text().await.unwrap_or_default();
    assert!(
        !body.contains("unstable"),
        "With intercept_errors true, body should NOT contain backend content ('unstable'), got: {body}"
    );
    assert!(
        body.contains("503") || body.contains("Service Unavailable") || body.contains("<!DOCTYPE"),
        "With intercept_errors true, body should be Ferron's built-in error page, got: {body}"
    );

    let resp = ctx
        .client
        .get(format!("{}/nonexistent", ctx.base_url))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 404, "Expected 404 Not Found");
    let body = resp.text().await.unwrap_or_default();
    assert!(
        !body.contains("Cannot GET"),
        "With intercept_errors true, body should NOT contain Express 404 body, got: {body}"
    );
}

#[tokio::test]
async fn test_intercept_errors_default_passthrough() {
    let config = br#"
*:80 {
    proxy "http://backend:3000"
}
"#;

    let ctx = InterceptTestContext::new("default", config, 1).await;

    let resp = ctx
        .client
        .get(format!("{}/unstable", ctx.base_url))
        .send()
        .await
        .expect("request failed");

    assert_eq!(resp.status(), 503, "Expected 503 Service Unavailable");
    let body = resp.text().await.unwrap_or_default();
    assert!(
        body.contains("unstable"),
        "Without intercept_errors, body should contain backend content ('unstable'), got: {body}"
    );

    let resp = ctx
        .client
        .get(format!("{}/nonexistent", ctx.base_url))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 404, "Expected 404 Not Found");
    let body = resp.text().await.unwrap_or_default();
    assert!(
        body.contains("Cannot GET"),
        "Without intercept_errors, body should contain Express 404 body ('Cannot GET'), got: {body}"
    );
}

#[tokio::test]
async fn test_intercept_errors_false_explicit() {
    let config = br#"
*:80 {
    proxy "http://backend:3000" {
        intercept_errors false
    }
}
"#;

    let ctx = InterceptTestContext::new("false", config, 1).await;

    let resp = ctx
        .client
        .get(format!("{}/unstable", ctx.base_url))
        .send()
        .await
        .expect("request failed");

    assert_eq!(resp.status(), 503, "Expected 503 Service Unavailable");
    let body = resp.text().await.unwrap_or_default();
    assert!(
        body.contains("unstable"),
        "With intercept_errors false, body should contain backend content ('unstable'), got: {body}"
    );
}

#[tokio::test]
async fn test_intercept_errors_successful_requests_unaffected() {
    let config = br#"
*:80 {
    proxy "http://backend:3000" {
        intercept_errors true
    }
}
"#;

    let ctx = InterceptTestContext::new("success", config, 0).await;

    let resp = ctx
        .client
        .get(format!("{}/", ctx.base_url))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200, "Expected 200 OK");
    let body = resp.text().await.unwrap_or_default();
    assert_eq!(
        body.trim(),
        "Hello, World!",
        "Successful upstream response should pass through unchanged, got: {body}"
    );

    let resp = ctx
        .client
        .get(format!("{}/whoami", ctx.base_url))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), 200, "Expected 200 OK");
    let body = resp.text().await.unwrap_or_default();
    assert_eq!(
        body.trim(),
        "backend",
        "/whoami should return backend name, got: {body}"
    );
}
