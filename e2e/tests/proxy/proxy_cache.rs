//! Proxy caching tests for Ferron reverse proxy.
//!
//! These tests are inspired by the nginx-tests `proxy_cache.t` test file,
//! which verifies correct proxy caching behavior including cache hit/miss,
//! stale serving, revalidation, and cache control. The original nginx tests
//! cover cache_valid, cache_min_uses, cache_use_stale, and more.
//!
//! See: https://github.com/nginx/nginx-tests/blob/master/proxy_cache.t

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

/// Test basic cache hit/miss behavior.
///
/// Inspired by nginx-tests proxy_cache.t — verifies that responses are cached
/// and subsequent requests are served from cache.
#[tokio::test]
async fn test_proxy_cache_hit_miss() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = self::common::create_temp_file();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-cache-hitmiss";

    let _backend = create_backend_container(network).await.unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
  cache true
  proxy "http://backend:3000"
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

    // First request — should hit backend
    let response1 = client
        .get(format!("http://localhost:{}/cache-etag?id=hitmiss", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response1.status(), reqwest::StatusCode::OK);
    let body1 = response1.text().await.unwrap();
    assert_eq!(body1, "v1");

    // Second request — should be served from cache
    let response2 = client
        .get(format!("http://localhost:{}/cache-etag?id=hitmiss", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response2.status(), reqwest::StatusCode::OK);
    let body2 = response2.text().await.unwrap();
    assert_eq!(body2, "v1");

    ferron.stop().await.unwrap();
}

/// Test cache stale-while-revalidate behavior.
///
/// Inspired by nginx-tests proxy_cache_use_stale.t — when a cached response
/// expires but stale-while-revalidate is set, the stale response should be
/// served while revalidation happens in the background.
#[tokio::test]
async fn test_proxy_cache_stale_while_revalidate() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = self::common::create_temp_file();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-cache-swr";

    let _backend = create_backend_container(network).await.unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
  cache true
  proxy "http://backend:3000"
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

    // First request — populate cache (max-age=1, stale-while-revalidate=60)
    let response1 = client
        .get(format!("http://localhost:{}/cache-swr?id=swr1", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response1.status(), reqwest::StatusCode::OK);
    assert_eq!(response1.text().await.unwrap(), "swr-v1");

    // Wait for cache to expire (max-age=1s)
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Request after expiry — should still get stale response (swr-v1)
    // because stale-while-revalidate=60 allows stale serving
    let response2 = client
        .get(format!("http://localhost:{}/cache-swr?id=swr1", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response2.status(), reqwest::StatusCode::OK);
    let body2 = response2.text().await.unwrap();
    // Should be stale v1 content (not a 5xx error)
    assert!(
        body2.contains("swr-v"),
        "Expected cached stale response, got: {}",
        body2
    );

    ferron.stop().await.unwrap();
}

/// Test cache stale-if-error behavior.
///
/// Inspired by nginx-tests proxy_cache_use_stale.t — when the upstream returns
/// a 5xx error during revalidation, the stale cached response should be served.
#[tokio::test]
async fn test_proxy_cache_stale_if_error() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = self::common::create_temp_file();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-cache-sie";

    let _backend = create_backend_container(network).await.unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
  cache true
  proxy "http://backend:3000"
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

    // First request — populate cache
    let response1 = client
        .get(format!("http://localhost:{}/cache-sie?id=sie1", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response1.status(), reqwest::StatusCode::OK);
    assert_eq!(response1.text().await.unwrap(), "sie-v1");

    // Tell backend to return 503 for subsequent requests
    let _ = client
        .post(format!("http://localhost:{}/cache-sie/error?id=sie1", port))
        .send()
        .await;

    // Wait a moment for the backend to start returning errors
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Request while backend is erroring — should serve stale from cache
    // (stale-if-error=60 allows this)
    let response2 = client
        .get(format!("http://localhost:{}/cache-sie?id=sie1", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response2.status(), reqwest::StatusCode::OK);
    let body2 = response2.text().await.unwrap();
    // Should be stale v1 content, not a 503 error
    assert!(
        body2.contains("sie-v"),
        "Expected stale cached response, got: {}",
        body2
    );

    ferron.stop().await.unwrap();
}

/// Test cache PURGE method invalidation.
///
/// This is a Ferron-specific feature — inspired by nginx-tests proxy_cache.t
/// but extended with Ferron's PURGE method support.
#[tokio::test]
async fn test_proxy_cache_purge() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = self::common::create_temp_file();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-cache-purge";

    let _backend = create_backend_container(network).await.unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
  cache {
    purge_method
    purge_allowed_ips "0.0.0.0/0"
  }
  proxy "http://backend:3000"
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

    // First request — populate cache
    let response1 = client
        .get(format!("http://localhost:{}/cache-purge?id=purge1", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response1.status(), reqwest::StatusCode::OK);
    assert_eq!(response1.text().await.unwrap(), "purge-v1");

    // Update the backend version
    let _ = client
        .post(format!(
            "http://localhost:{}/cache-purge/update?id=purge1",
            port
        ))
        .send()
        .await;

    // Request should still be cached (v1)
    let response2 = client
        .get(format!("http://localhost:{}/cache-purge?id=purge1", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response2.text().await.unwrap(), "purge-v1");

    // PURGE the cached response
    let response_purge = client
        .request(
            reqwest::Method::from_bytes(b"PURGE").unwrap(),
            format!("http://localhost:{}/cache-purge?id=purge1", port),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response_purge.status(), reqwest::StatusCode::OK);

    // Request after purge — should get fresh v2 from backend
    let response3 = client
        .get(format!("http://localhost:{}/cache-purge?id=purge1", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response3.status(), reqwest::StatusCode::OK);
    assert_eq!(response3.text().await.unwrap(), "purge-v2");
}
