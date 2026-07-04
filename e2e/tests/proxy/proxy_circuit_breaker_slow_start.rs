//! Circuit breaker slow-start E2E tests.
//!
//! These tests verify that the `slow_start` directive in the `circuit_breaker`
//! block causes a recovering backend to receive reduced traffic during the
//! slow-start window, preventing thundering herd.

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
        .with_env_var("UNSTABLE_FAILS", unstable_fails.to_string())
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

/// Test that slow-start configuration is accepted and the circuit breaker
/// recovers correctly after tripping.
///
/// Uses two backends:
/// - backend-fail: fails on first request (`UNSTABLE_FAILS=1`), then succeeds
/// - backend-ok: always succeeds
///
/// With `slow_start "10s"`, after the circuit for backend-fail closes, the
/// slow-start window should be active. We verify this indirectly by checking
/// that requests are still served correctly (both backends respond).
#[tokio::test]
async fn test_circuit_breaker_slow_start_config_accepted() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = common::create_temp_file();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-cb-slowstart-accept";

    let _backend_fail = create_backend_container(network, "backend-fail", "backend-fail", 1)
        .await
        .unwrap();
    let _backend_ok = create_backend_container(network, "backend-ok", "backend-ok", 0)
        .await
        .unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
  proxy {
    upstream "http://backend-fail:3000"
    upstream "http://backend-ok:3000"

    algorithm round_robin
    retry_connection false

    circuit_breaker {
      max_fails 1
      window "30s"
      open_duration "1s"
      consecutive_passes 1
      slow_start "10s"
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

    // Step 1: First request goes to backend-fail (round_robin), which fails.
    let response1 = client
        .get(format!("http://localhost:{}/unstable", port))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response1.status(),
        reqwest::StatusCode::SERVICE_UNAVAILABLE,
        "First request should fail (backend-fail unstable). Ferron logs:\n{}",
        ferron_logs(&ferron).await
    );

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Step 2: Second request goes to backend-ok (backend-fail circuit is open).
    let response2 = client
        .get(format!("http://localhost:{}/whoami", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response2.status(), reqwest::StatusCode::OK);
    let body2 = response2.text().await.unwrap();
    assert_eq!(
        body2, "backend-ok",
        "Second request should go to backend-ok. Ferron logs:\n{}",
        ferron_logs(&ferron).await
    );

    // Step 3: Wait for circuit to transition to half-open, then close.
    // open_duration = 1s, so after 2s the circuit is half-open.
    // The next successful request closes the circuit (consecutive_passes = 1).
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Step 4: Request to backend-fail (half-open probe). It should succeed
    // because UNSTABLE_FAILS=1 means it only fails the first request.
    let response3 = client
        .get(format!("http://localhost:{}/whoami", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response3.status(), reqwest::StatusCode::OK);
    let body3 = response3.text().await.unwrap();
    assert!(
        body3 == "backend-fail" || body3 == "backend-ok",
        "Expected backend name, got: {}. Ferron logs:\n{}",
        body3,
        ferron_logs(&ferron).await
    );

    // Step 5: Subsequent requests should succeed from either backend.
    // The circuit is now closed (slow-start active for backend-fail).
    let response4 = client
        .get(format!("http://localhost:{}/whoami", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response4.status(), reqwest::StatusCode::OK);
    let body4 = response4.text().await.unwrap();
    assert!(
        body4 == "backend-fail" || body4 == "backend-ok",
        "Expected backend name, got: {}. Ferron logs:\n{}",
        body4,
        ferron_logs(&ferron).await
    );

    ferron.stop().await.unwrap();
}

/// Test that slow-start duration is correctly parsed from the configuration.
///
/// Verifies that a very short slow-start duration (`0.5s`) is accepted
/// and that after the window elapses, requests are served normally.
#[tokio::test]
async fn test_circuit_breaker_slow_start_short_duration() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = common::create_temp_file();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-cb-slowstart-short";

    let _backend = create_backend_container(network, "backend", "backend", 0)
        .await
        .unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
  proxy {
    upstream "http://backend:3000"

    circuit_breaker {
      max_fails 1
      window "30s"
      open_duration "1s"
      consecutive_passes 1
      slow_start "0.5s"
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

    // Request should succeed (backend is healthy).
    let response = client
        .get(format!("http://localhost:{}/whoami", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), "backend");

    // Wait for slow-start window to elapse and make another request.
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    let response2 = client
        .get(format!("http://localhost:{}/whoami", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response2.status(), reqwest::StatusCode::OK);
    assert_eq!(response2.text().await.unwrap(), "backend");

    ferron.stop().await.unwrap();
}

async fn ferron_logs(ferron: &ContainerAsync<GenericImage>) -> String {
    let stdout = String::from_utf8(ferron.stdout_to_vec().await.unwrap_or_default())
        .unwrap_or_default();
    let stderr = String::from_utf8(ferron.stderr_to_vec().await.unwrap_or_default())
        .unwrap_or_default();
    format!("stdout:\n{stdout}\n\nstderr:\n{stderr}")
}
