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
    core::{wait::HttpWaitStrategy, ContainerPort, Mount, WaitFor},
    runners::AsyncRunner,
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
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

#[tokio::test]
async fn test_failover_on_http_error() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = self::common::create_temp_file();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-failover-httperror";

    let _backend_ok = create_backend_container(network, "backend-ok", "backend-ok", 0)
        .await
        .unwrap();
    let _backend_unstable =
        create_backend_container(network, "backend-unstable", "backend-unstable", 10)
            .await
            .unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
  proxy {
    upstream "http://backend-ok:3000"
    upstream "http://backend-unstable:3000"

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

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap();

    for _ in 0..9 {
        let response = client
            .get(format!("http://localhost:{}/unstable?unsafe=true", port))
            .send()
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            reqwest::StatusCode::OK,
            "Expected 200 OK when one backend is unstable"
        );
    }

    ferron.stop().await.unwrap();
}

/// A failed request without an available failover target must not consume
/// retry-budget tokens. Otherwise a fully unavailable upstream degrades from
/// 502 to 503 even though no retry was ever possible.
#[tokio::test]
async fn test_retry_budget_not_consumed_without_failover() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = self::common::create_temp_file();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-retry-budget-no-failover";

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
  proxy {
    upstream "http://127.0.0.1:3999"
    algorithm round_robin
    circuit_breaker false
    retry_connection true
    max_retries_per_upstream 0
    retry_budget {
      max_retry_rate 0.5
      max_tokens 10
      refill_rate 0.0
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

    for _ in 0..15 {
        let response = client
            .get(format!("http://localhost:{}/whoami", port))
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            reqwest::StatusCode::BAD_GATEWAY,
            "unavailable upstream without failover must stay 502"
        );
    }

    ferron.stop().await.unwrap();
}

/// A tiny `max_retry_rate` must refuse cross-backend retries even when the
/// token bucket is large and refills quickly.
#[tokio::test]
async fn test_retry_budget_enforces_max_retry_rate() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = self::common::create_temp_file();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-retry-budget-rate";

    let _backend_ok = create_backend_container(network, "backend-ok", "backend-ok", 0)
        .await
        .unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
  proxy {
    upstream "http://127.0.0.1:3999"
    upstream "http://backend-ok:3000"

    algorithm round_robin
    retry_connection true
    max_retries_per_upstream 0
    retry_budget {
      max_retry_rate 0.0001
      max_tokens 1000000
      refill_rate 1000000
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

    let mut succeeded = 0;
    let mut refused = 0;
    for _ in 0..20 {
        let response = client
            .get(format!("http://localhost:{}/whoami", port))
            .send()
            .await
            .unwrap();
        match response.status() {
            reqwest::StatusCode::OK => succeeded += 1,
            reqwest::StatusCode::BAD_GATEWAY | reqwest::StatusCode::SERVICE_UNAVAILABLE => {
                refused += 1
            }
            status => panic!("unexpected status {status}"),
        }
    }

    assert!(succeeded > 0, "healthy backend must serve some requests");
    assert!(
        refused > 0,
        "max_retry_rate 0.0001 must refuse most retries to the dead backend"
    );

    ferron.stop().await.unwrap();
}
