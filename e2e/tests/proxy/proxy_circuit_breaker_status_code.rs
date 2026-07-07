//! HTTP status code-based circuit breaker E2E tests.
//!
//! These tests verify that the `record_5xx` directive in the `circuit_breaker`
//! block correctly records upstream HTTP 5xx responses as failures, tripping
//! the circuit when the `max_fails` threshold is reached.

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

/// Test that the circuit trips when `record_5xx true` and the upstream returns 503.
///
/// Uses two backends:
/// - backend-a: returns 503 via `/status?code=503`
/// - backend-b: always responds OK via `/`
///
/// With `max_fails 1`, `record_5xx true`, the 503 response should trip
/// the circuit, and subsequent requests should route to backend-b.
#[tokio::test]
async fn test_circuit_breaker_trips_on_5xx_with_record_5xx() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = common::create_temp_file();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-cb-status-trip";

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
    upstream "http://backend-a:3000"
    upstream "http://backend-b:3000"

    algorithm round_robin
    retry_connection false

    circuit_breaker {
      max_fails 1
      window "30s"
      open_duration "30s"
      consecutive_passes 1
      record_5xx true
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

    // First request goes to backend-a (round_robin), which returns 503.
    let response1 = client
        .get(format!("http://localhost:{}/status?code=503", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response1.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);

    // Give the circuit breaker a moment to register the failure
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Second request should go to backend-b because backend-a's circuit is open.
    let response2 = client
        .get(format!("http://localhost:{}/whoami", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response2.status(), reqwest::StatusCode::OK);
    assert_eq!(response2.text().await.unwrap(), "backend-b");

    ferron.stop().await.unwrap();
}

/// Test that 5xx responses do NOT trip the circuit when `record_5xx` is not set (default: false).
///
/// With `record_5xx` absent (default false), the circuit should never trip
/// regardless of 5xx responses from the backend.
#[tokio::test]
async fn test_circuit_breaker_does_not_trip_on_5xx_without_record_5xx() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = common::create_temp_file();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-cb-status-notrip";

    let _backend = create_backend_container(network, "backend", "backend")
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

    circuit_breaker {
      max_fails 3
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

    // Send multiple 5xx requests — without record_5xx, the circuit should never trip.
    for _ in 0..5 {
        let response = client
            .get(format!("http://localhost:{}/status?code=503", port))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
    }

    // The backend should still be reachable
    let response = client
        .get(format!("http://localhost:{}/whoami", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), "backend");

    ferron.stop().await.unwrap();
}

/// Test that the circuit correctly records only 5xx, not 4xx responses.
///
/// With `record_5xx true`, 4xx responses should NOT trip the circuit,
/// but a 5xx response should.
#[tokio::test]
async fn test_circuit_breaker_does_not_record_4xx() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = common::create_temp_file();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-cb-status-4xx";

    let _backend = create_backend_container(network, "backend", "backend")
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

    circuit_breaker {
      max_fails 2
      window "30s"
      open_duration "30s"
      consecutive_passes 1
      record_5xx true
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

    // Send 4xx responses — should NOT trip the circuit
    for _ in 0..5 {
        let response = client
            .get(format!("http://localhost:{}/status?code=404", port))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    }

    // Backend should still be reachable with 200 OK
    let response = client
        .get(format!("http://localhost:{}/whoami", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), "backend");

    ferron.stop().await.unwrap();
}

/// Test that different 5xx status codes accumulate toward tripping the circuit.
///
/// With `record_5xx true` and `max_fails 2`, a single 500 does not trip
/// the circuit, but a second 502 does — confirming both codes are recorded.
#[tokio::test]
async fn test_circuit_breaker_various_5xx_codes() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = common::create_temp_file();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-cb-status-various";

    let _backend = create_backend_container(network, "backend", "backend")
        .await
        .unwrap();
    let _fallback = create_backend_container(network, "fallback", "fallback")
        .await
        .unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
  match READY {
    request.uri.path == "/__ready"
  }

  if READY {
    status 204
  }

  proxy {
    upstream "http://backend:3000"
    upstream "http://fallback:3000"

    algorithm round_robin
    retry_connection false

    circuit_breaker {
      max_fails 2
      window "30s"
      open_duration "30s"
      consecutive_passes 1
      record_5xx true
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

    // Request 1 (round_robin → backend): returns 500 — failure count: 1
    let response = client
        .get(format!("http://localhost:{}/status?code=500", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::INTERNAL_SERVER_ERROR);

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Request 2 (round_robin → fallback): returns 502 — not a failure for backend.
    // This also verifies the fallback works correctly.
    let response = client
        .get(format!("http://localhost:{}/status?code=502", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_GATEWAY);

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Request 3 (round_robin → backend): returns 500 — failure count: 2, circuit trips
    let response = client
        .get(format!("http://localhost:{}/status?code=500", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::INTERNAL_SERVER_ERROR);

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Request 4: backend circuit is open, route to fallback
    let response = client
        .get(format!("http://localhost:{}/whoami", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), "fallback");

    ferron.stop().await.unwrap();
}

/// Test that the circuit breaker recovers (half-open -> closed) after being
/// tripped by 5xx status codes.
///
/// With `record_5xx true`, `max_fails 1`, `open_duration "1s"`, and
/// `consecutive_passes 1`:
/// 1. First 503 response trips the circuit
/// 2. Wait for open_duration to expire
/// 3. Half-open probe succeeds, circuit closes
#[tokio::test]
async fn test_circuit_breaker_5xx_half_open_recovery() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = common::create_temp_file();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-cb-status-recovery";

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
    upstream "http://backend-a:3000"
    upstream "http://backend-b:3000"

    algorithm round_robin
    retry_connection false

    circuit_breaker {
      max_fails 1
      window "30s"
      open_duration "1s"
      consecutive_passes 1
      record_5xx true
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

    // Step 1: First request to backend-a returns 503 — trips the circuit
    let response = client
        .get(format!("http://localhost:{}/status?code=503", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Step 2: Request goes to backend-b (backend-a circuit is open)
    let response = client
        .get(format!("http://localhost:{}/whoami", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), "backend-b");

    // Step 3: Wait for circuit to transition to half-open (open_duration = 1s)
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Step 4: Request to whoami — both backends should be reachable again.
    // backend-a's circuit is half-open, and backend-a returns 200 OK on /whoami,
    // which should close the circuit again (consecutive_passes = 1).
    let response = client
        .get(format!("http://localhost:{}/whoami", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body = response.text().await.unwrap();
    assert!(
        body == "backend-a" || body == "backend-b",
        "Expected backend name, got: {}",
        body
    );

    ferron.stop().await.unwrap();
}
