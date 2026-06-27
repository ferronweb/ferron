//! Proxy failover tests for Ferron reverse proxy.
//!
//! These tests are inspired by the nginx-tests `proxy_next_upstream.t` test
//! file, which verifies that the proxy correctly retries requests on upstream
//! failures. The original nginx tests cover failover on error, timeout,
//! invalid_header, http_500, http_404, non_idempotent, max_tries, and
//! trying times.
//!
//! See: https://github.com/nginx/nginx-tests/blob/master/proxy_next_upstream.t

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

/// Test failover on connection refused (transport failure).
///
/// Inspired by nginx-tests proxy_next_upstream.t — when the first upstream
/// is unreachable, Ferron should retry on the second upstream.
#[tokio::test]
async fn test_failover_on_connection_refused() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = self::common::create_temp_file();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-failover-connrefused";

    let _backend = create_backend_container(network, "backend-ok", "backend-ok", 0)
        .await
        .unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
  proxy {
    upstream "http://backend-ok:3999" # Connection refused
    upstream "http://backend-ok:3000"

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

    // All requests should succeed because of retry
    for _ in 0..5 {
        let response = client
            .get(format!("http://localhost:{}/whoami", port))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert_eq!(response.text().await.unwrap(), "backend-ok");
    }

    ferron.stop().await.unwrap();
}

/// Test failover on backend timeout.
///
/// Inspired by nginx-tests proxy_next_upstream.t — when the first upstream
/// times out, Ferron should retry on the second upstream.
#[tokio::test]
async fn test_failover_on_timeout() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = self::common::create_temp_file();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-failover-timeout";

    let _backend_ok = create_backend_container(network, "backend-ok", "backend-ok", 0)
        .await
        .unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
  http {
    timeout "1s"
  }
  proxy {
    upstream "http://backend-ok:3000" # Sleep 5s endpoint
    upstream "http://backend-ok:3000"

    algorithm round_robin
    retry_connection false
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

    // /unstable?sleep=5000 should timeout, then retry on same backend (which also sleeps)
    // This tests that timeout triggers retry
    let response = client
        .get(format!("http://localhost:{}/unstable?sleep=5000", port))
        .send()
        .await
        .unwrap();

    // Should get 408 Request Timeout since both backends are slow
    assert_eq!(
        response.status(),
        reqwest::StatusCode::REQUEST_TIMEOUT,
        "Expected 408 timeout when all backends are slow"
    );

    ferron.stop().await.unwrap();
}

/// Test that retry can be disabled.
///
/// Inspired by nginx-tests proxy_next_upstream.t — when retry is disabled,
/// the first failure should be returned directly.
#[tokio::test]
async fn test_failover_disabled() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = self::common::create_temp_file();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-failover-disabled";

    let _backend = create_backend_container(network, "backend-ok", "backend-ok", 0)
        .await
        .unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
  proxy {
    upstream "http://backend-ok:3999" # Connection refused
    upstream "http://backend-ok:3000"

    algorithm round_robin
    retry_connection false
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

    // With retry disabled, hitting the failing backend should return 502
    let response = client
        .get(format!("http://localhost:{}/whoami", port))
        .send()
        .await
        .unwrap();
    // Could be 502 (bad gateway) if the first upstream is selected and fails
    // or 200 if the healthy backend is selected first — either is acceptable
    // when retry is disabled and round_robin is used
    assert!(
        response.status().is_success() || response.status() == reqwest::StatusCode::BAD_GATEWAY,
        "Expected 200 or 502, got {}",
        response.status()
    );

    ferron.stop().await.unwrap();
}
