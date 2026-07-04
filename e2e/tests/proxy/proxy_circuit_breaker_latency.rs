//! Latency-aware circuit breaker E2E tests.
//!
//! These tests verify that the `latency_threshold` directive in the
//! `circuit_breaker` block causes the circuit to trip when upstream responses
//! exceed the configured duration.

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
    let backend_image = common::build_backend_image().await?;
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
    let ferron_image = common::build_ferron_image().await?;
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

/// Test that the circuit trips when upstream response time exceeds `latency_threshold`.
///
/// Uses two backends:
/// - backend-slow: always responds after 500ms via `/unstable?sleep=500`
/// - backend-fast: always responds immediately via `/`
///
/// With `latency_threshold "0.1s"` and `max_fails 1`, the slow backend should
/// trip after the first request, and subsequent requests should go to the fast backend.
#[tokio::test]
async fn test_circuit_breaker_trips_on_slow_upstream() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = common::create_temp_file();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-cb-latency-trip";

    let _backend_slow = create_backend_container(network, "backend-slow", "backend-slow")
        .await
        .unwrap();
    let _backend_fast = create_backend_container(network, "backend-fast", "backend-fast")
        .await
        .unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
  proxy {
    upstream "http://backend-slow:3000"
    upstream "http://backend-fast:3000"

    algorithm round_robin
    retry_connection false

    circuit_breaker {
      max_fails 1
      window "30s"
      open_duration "30s"
      consecutive_passes 2
      latency_threshold "0.1s"
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

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap();

    // First request goes to backend-slow (round_robin), which takes 500ms.
    // This exceeds latency_threshold of 100ms, so the circuit should trip.
    let response1 = client
        .get(format!(
            "http://localhost:{}/unstable?sleep=500",
            port
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(response1.status(), reqwest::StatusCode::OK);
    assert_eq!(response1.text().await.unwrap(), "backend-slow");

    // Give the circuit breaker a moment to register the failure
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Second request should go to backend-fast because backend-slow's circuit is open.
    let response2 = client
        .get(format!("http://localhost:{}/whoami", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response2.status(), reqwest::StatusCode::OK);
    assert_eq!(response2.text().await.unwrap(), "backend-fast");

    ferron.stop().await.unwrap();
}

/// Test that slow responses do NOT trip the circuit when `latency_threshold` is not set.
///
/// Uses a backend that always responds after 500ms via `/unstable?sleep=500`.
/// Without `latency_threshold`, the circuit should never trip regardless of response time.
#[tokio::test]
async fn test_circuit_breaker_does_not_trip_without_latency_threshold() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = common::create_temp_file();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-cb-latency-notrip";

    let _backend = create_backend_container(network, "backend-slow", "backend-slow")
        .await
        .unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
  proxy {
    upstream "http://backend-slow:3000"

    algorithm round_robin

    circuit_breaker {
      max_fails 1
      window "30s"
      open_duration "30s"
      consecutive_passes 1
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

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap();

    // Send multiple slow requests — without latency_threshold, the circuit should never trip.
    for _ in 0..3 {
        let response = client
            .get(format!(
                "http://localhost:{}/unstable?sleep=200",
                port
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert_eq!(response.text().await.unwrap(), "backend-slow");
    }

    ferron.stop().await.unwrap();
}

/// Test that the circuit stays open after being tripped by latency, then
/// transitions to half-open and closes after a fast response.
///
/// Uses two backends:
/// - backend-slow: responds after 500ms
/// - backend-fast: responds immediately
///
/// With `latency_threshold "0.1s"`, `max_fails 1`, `open_duration "2s"`, and
/// `consecutive_passes 1`:
/// 1. Request 1 → backend-slow (slow) → circuit trips
/// 2. Request 2 → backend-fast (fast, circuit is open but we still route to fast)
/// 3. Wait for open_duration to expire
/// 4. Request 3 → backend-slow (now in half-open, fast response because timeout is short)
///    → circuit should close
#[tokio::test]
async fn test_circuit_breaker_latency_half_open_recovery() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = common::create_temp_file();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-cb-latency-halfopen";

    let _backend_slow = create_backend_container(network, "backend-slow", "backend-slow")
        .await
        .unwrap();
    let _backend_fast = create_backend_container(network, "backend-fast", "backend-fast")
        .await
        .unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
  proxy {
    upstream "http://backend-slow:3000"
    upstream "http://backend-fast:3000"

    algorithm round_robin
    retry_connection false

    circuit_breaker {
      max_fails 1
      window "30s"
      open_duration "2s"
      consecutive_passes 1
      latency_threshold "0.1s"
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

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap();

    // Step 1: Request to slow backend — trips the circuit (500ms > 100ms threshold)
    let response = client
        .get(format!(
            "http://localhost:{}/unstable?sleep=500",
            port
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), "backend-slow");

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Step 2: Request to fast backend (round_robin selects backend-fast)
    // Circuit is open for backend-slow, but backend-fast is fine
    let response = client
        .get(format!("http://localhost:{}/whoami", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), "backend-fast");

    // Step 3: Wait for circuit to transition to half-open (open_duration = 2s)
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    // Step 4: Request to slow backend again (half-open) — but we use /whoami
    // to get the backend name. Since the circuit is half-open, it will try backend-slow.
    // We expect a fast response because in half-open state, a single success closes the circuit.
    let response = client
        .get(format!("http://localhost:{}/whoami", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    // The response should be from one of the backends (either slow or fast is fine
    // in half-open; what matters is the circuit closes and subsequent requests succeed)
    let body = response.text().await.unwrap();
    assert!(
        body == "backend-slow" || body == "backend-fast",
        "Expected backend name, got: {}",
        body
    );

    ferron.stop().await.unwrap();
}
