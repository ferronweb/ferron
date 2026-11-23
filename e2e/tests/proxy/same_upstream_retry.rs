//! Same-upstream retry tests for Ferron reverse proxy.

use std::io::Write;

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};

use crate::common;

async fn create_backend_container(
    network: &str,
    alias: &str,
    backend_name: &str,
    unstable_fails: u32,
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
        .with_hostname(alias)
        .with_env_var("BACKEND_NAME", backend_name)
        .with_env_var("UNSTABLE_FAILS", unstable_fails.to_string())
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

#[tokio::test]
async fn test_same_upstream_retry_default_succeeds() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let mut config_file = self::common::create_temp_file();

    let network = "e2e-test-same-retry-default";

    // One backend that destroys the socket on the first request to /unstable?unsafe=true
    let _backend = create_backend_container(network, "backend", "backend", 1)
        .await
        .unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
  proxy {
    upstream "http://backend:3000"
    algorithm round_robin
  }
}
"#,
        )
        .unwrap();

    let ferron = create_ferron_container(network, config_file.path())
        .await
        .unwrap();

    let port = ferron
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let response = client
        .get(format!("http://localhost:{}/unstable?unsafe=true", port))
        .send()
        .await
        .unwrap();

    // With default max_retries_per_upstream 1, the second attempt succeeds.
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), "backend");

    ferron.stop().await.unwrap();
}

#[tokio::test]
async fn test_same_upstream_retry_disabled_fails() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let mut config_file = self::common::create_temp_file();

    let network = "e2e-test-same-retry-disabled";

    let _backend = create_backend_container(network, "backend", "backend", 1)
        .await
        .unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
  proxy {
    upstream "http://backend:3000"
    algorithm round_robin
    max_retries_per_upstream 0
  }
}
"#,
        )
        .unwrap();

    let ferron = create_ferron_container(network, config_file.path())
        .await
        .unwrap();

    let port = ferron
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let response = client
        .get(format!("http://localhost:{}/unstable?unsafe=true", port))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::BAD_GATEWAY);

    ferron.stop().await.unwrap();
}

#[tokio::test]
async fn test_same_upstream_retry_two_recovers() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let mut config_file = self::common::create_temp_file();

    let network = "e2e-test-same-retry-two";

    let _backend = create_backend_container(network, "backend", "backend", 2)
        .await
        .unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
  proxy {
    upstream "http://backend:3000"
    algorithm round_robin
    max_retries_per_upstream 2
  }
}
"#,
        )
        .unwrap();

    let ferron = create_ferron_container(network, config_file.path())
        .await
        .unwrap();

    let port = ferron
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let response = client
        .get(format!("http://localhost:{}/unstable?unsafe=true", port))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), "backend");

    ferron.stop().await.unwrap();
}

/// POST (non-idempotent) must not be retried on the same upstream.
#[tokio::test]
async fn test_same_upstream_retry_non_idempotent_no_retry() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let mut config_file = self::common::create_temp_file();

    let network = "e2e-test-same-retry-post";

    let _backend = create_backend_container(network, "backend", "backend", 5)
        .await
        .unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
  proxy {
    upstream "http://backend:3000"
    algorithm round_robin
    max_retries_per_upstream 2
  }
}
"#,
        )
        .unwrap();

    let ferron = create_ferron_container(network, config_file.path())
        .await
        .unwrap();

    let port = ferron
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();

    let client = reqwest::Client::new();
    // POST to /unsafe always destroys the socket; non-idempotent must not be retried
    let response = client
        .post(format!("http://localhost:{}/unsafe", port))
        .body("hello")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::BAD_GATEWAY);

    ferron.stop().await.unwrap();
}
