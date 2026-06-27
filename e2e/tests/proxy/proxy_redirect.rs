//! Proxy redirect rewriting tests for Ferron.
//!
//! These tests are inspired by the nginx-tests `proxy_redirect.t` test file,
//! which verifies that the proxy correctly rewrites Location headers in
//! redirect responses from upstream backends.
//!
//! See: https://github.com/nginx/nginx-tests/blob/master/proxy_redirect.t

use std::io::Write;

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

/// Test that redirect responses from backend are correctly proxied.
///
/// Inspired by nginx-tests proxy_redirect.t — verifies that Location headers
/// in redirect responses from the upstream are passed through correctly.
#[tokio::test]
async fn test_proxy_redirect_passthrough() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = self::common::create_temp_file();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-proxy-redirect";

    let _backend = create_backend_container(network).await.unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
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

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    // The backend's /redirect endpoint returns a 302 redirect
    let response = client
        .get(format!(
            "http://localhost:{}/redirect?target=/hello&code=302",
            port
        ))
        .send()
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        reqwest::StatusCode::FOUND,
        "Should get 302 redirect from backend"
    );

    let location = response
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        location.contains("/hello"),
        "Location should contain /hello, got: {}",
        location
    );

    ferron.stop().await.unwrap();
}

/// Test that redirect with 301 status code works correctly.
///
/// Inspired by nginx-tests proxy_redirect.t — verifies different redirect
/// status codes are proxied correctly.
#[tokio::test]
async fn test_proxy_redirect_301() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = self::common::create_temp_file();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-proxy-redirect-301";

    let _backend = create_backend_container(network).await.unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
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

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    let response = client
        .get(format!(
            "http://localhost:{}/redirect?target=/new-location&code=301",
            port
        ))
        .send()
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        reqwest::StatusCode::MOVED_PERMANENTLY,
        "Should get 301 redirect"
    );

    let location = response
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        location.contains("/new-location"),
        "Location should contain /new-location, got: {}",
        location
    );

    ferron.stop().await.unwrap();
}
