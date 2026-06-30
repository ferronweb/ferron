//! Priority-based failover E2E tests.
//!
//! These tests verify that priority grouping works end-to-end:
//! requests route to the highest-priority tier first, and fail
//! over to lower tiers when the higher tier is unavailable.

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

/// Test that requests route to the highest-priority tier when available.
#[tokio::test]
async fn test_priority_failover_basic() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = self::common::create_temp_file();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-priority-basic";

    let _backend_primary = create_backend_container(network, "backend-primary", "backend-primary")
        .await
        .unwrap();
    let _backend_secondary =
        create_backend_container(network, "backend-secondary", "backend-secondary")
            .await
            .unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
  proxy {
    upstream "http://backend-primary:3000" {
      priority 0
    }
    upstream "http://backend-secondary:3000" {
      priority 1
    }

    algorithm round_robin
    retry_connection true
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

    // All requests should go to the primary backend (priority 0)
    for _ in 0..10 {
        let response = client
            .get(format!("http://localhost:{}/whoami", port))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert_eq!(response.text().await.unwrap(), "backend-primary");
    }

    ferron.stop().await.unwrap();
}

/// Test that requests distribute across backends within the same priority tier.
#[tokio::test]
async fn test_priority_within_tier() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = self::common::create_temp_file();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-priority-within-tier";

    let _backend_a = create_backend_container(network, "backend-a", "backend-a")
        .await
        .unwrap();
    let _backend_b = create_backend_container(network, "backend-b", "backend-b")
        .await
        .unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
  proxy {
    upstream "http://backend-a:3000" {
      priority 0
    }
    upstream "http://backend-b:3000" {
      priority 0
    }

    algorithm round_robin
    retry_connection true
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

    // With round_robin, requests should distribute across both backends
    let mut seen_a = false;
    let mut seen_b = false;
    for _ in 0..10 {
        let response = client
            .get(format!("http://localhost:{}/whoami", port))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let body = response.text().await.unwrap();
        match body.as_str() {
            "backend-a" => seen_a = true,
            "backend-b" => seen_b = true,
            other => panic!("Unexpected backend: {other}"),
        }
    }
    assert!(
        seen_a && seen_b,
        "Expected both backends to be hit, seen_a={seen_a}, seen_b={seen_b}"
    );

    ferron.stop().await.unwrap();
}

/// Test that requests fail over to a lower-priority tier when the higher tier is unavailable.
#[tokio::test]
async fn test_priority_failover_across_tiers() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = self::common::create_temp_file();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-priority-across-tiers";

    let _backend_secondary =
        create_backend_container(network, "backend-secondary", "backend-secondary")
            .await
            .unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
  proxy {
    upstream "http://backend-secondary:3999" {
      priority 0
    }
    upstream "http://backend-secondary:3000" {
      priority 1
    }

    algorithm round_robin
    retry_connection true
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

    // Primary is unreachable (port 3999), should fail over to secondary on port 3000
    for _ in 0..10 {
        let response = client
            .get(format!("http://localhost:{}/whoami", port))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert_eq!(response.text().await.unwrap(), "backend-secondary");
    }

    ferron.stop().await.unwrap();
}

/// Test priority failover with circuit breaker: when the higher-priority
/// backend trips the circuit breaker, requests fall through to the next tier.
#[tokio::test]
async fn test_priority_failover_circuit_breaker() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = self::common::create_temp_file();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-priority-circuit-breaker";

    let _backend_secondary =
        create_backend_container(network, "backend-secondary", "backend-secondary")
            .await
            .unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
  proxy {
    upstream "http://backend-secondary:3999" {
      priority 0
    }
    upstream "http://backend-secondary:3000" {
      priority 1
    }

    algorithm round_robin
    retry_connection true

    circuit_breaker {
      enabled true
      window_size 1
      min_requests 1
      failure_rate_threshold 1.0
      half_open_max_connections 1
    }
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

    // First request trips circuit breaker on primary (port 3999), then fails over
    // to secondary (port 3000). Subsequent requests go directly to secondary.
    for _ in 0..5 {
        let response = client
            .get(format!("http://localhost:{}/whoami", port))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert_eq!(response.text().await.unwrap(), "backend-secondary");
    }

    ferron.stop().await.unwrap();
}
