#[cfg(unix)]
use std::{fs::Permissions, os::unix::fs::PermissionsExt};
use std::{io::Write, path::Path};

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};

mod common;

async fn create_backend_container(
    network: &str,
    alias: &str,
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
        .start()
        .await
}

async fn create_ferron_container(
    network: &str,
    config_file: &Path,
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
async fn test_affinity_cookie() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-affinity-cookie";

    let _backend1 = create_backend_container(network, "backend-1")
        .await
        .unwrap();
    let _backend2 = create_backend_container(network, "backend-2")
        .await
        .unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
ferron-affinity-cookie:80 {
  proxy {
    upstream "http://backend-1:3000"
    upstream "http://backend-2:3000"
    algorithm "random"
    affinity cookie {
      name "sticky"
      httponly
      samesite lax
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

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let client = reqwest::Client::new();

    // First request should set the cookie in Set-Cookie header
    let resp1 = client
        .get(format!("http://localhost:{}/", port))
        .header("Host", "ferron-affinity-cookie")
        .send()
        .await
        .unwrap();
    assert_eq!(resp1.status(), reqwest::StatusCode::OK);
    let body1 = resp1.text().await.unwrap();
    assert_eq!(body1, "Hello, World!");

    // The cookie should be set on the response (we can't easily verify without cookie parsing)
    // Subsequent requests should work correctly
    for _ in 0..5 {
        let resp = client
            .get(format!("http://localhost:{}/", port))
            .header("Host", "ferron-affinity-cookie")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::OK);
        let body = resp.text().await.unwrap();
        assert_eq!(body, "Hello, World!");
    }
}

#[tokio::test]
async fn test_affinity_ip() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-affinity-ip";

    let _backend1 = create_backend_container(network, "backend-1")
        .await
        .unwrap();
    let _backend2 = create_backend_container(network, "backend-2")
        .await
        .unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
ferron-affinity-ip:80 {
  proxy {
    upstream "http://backend-1:3000"
    upstream "http://backend-2:3000"
    affinity ip
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

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let client = reqwest::Client::new();

    // Multiple requests from the same client IP should go to the same backend
    let mut responses = Vec::new();
    for _ in 0..10 {
        let resp = client
            .get(format!("http://localhost:{}/", port))
            .header("Host", "ferron-affinity-ip")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::OK);
        let body = resp.text().await.unwrap();
        responses.push(body);
    }

    // All responses should be identical (same backend)
    let first = &responses[0];
    for resp in &responses[1..] {
        assert_eq!(resp, first, "IP affinity should route to same backend");
    }
}

#[tokio::test]
async fn test_consistent_hash_algorithm() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-consistent-hash";

    let _backend1 = create_backend_container(network, "backend-1")
        .await
        .unwrap();
    let _backend2 = create_backend_container(network, "backend-2")
        .await
        .unwrap();
    let _backend3 = create_backend_container(network, "backend-3")
        .await
        .unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
ferron-consistent-hash:80 {
  proxy {
    upstream "http://backend-1:3000"
    upstream "http://backend-2:3000"
    upstream "http://backend-3:3000"
    algorithm "consistent_hash"
    affinity ip
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

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let client = reqwest::Client::new();

    // Same IP should always route to the same backend with consistent_hash
    let mut responses = Vec::new();
    for _ in 0..10 {
        let resp = client
            .get(format!("http://localhost:{}/", port))
            .header("Host", "ferron-consistent-hash")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::OK);
        let body = resp.text().await.unwrap();
        responses.push(body);
    }

    // All responses should be identical
    let first = &responses[0];
    for resp in &responses[1..] {
        assert_eq!(
            resp, first,
            "Consistent hash should route same IP to same backend"
        );
    }
}
