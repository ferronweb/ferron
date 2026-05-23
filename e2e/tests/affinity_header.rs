use std::io::Write;

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};

mod common;

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

#[tokio::test]
async fn test_affinity_header() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = common::create_temp_file();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-affinity-header";

    let _backend1 = create_backend_container(network, "backend-1", "A")
        .await
        .unwrap();
    let _backend2 = create_backend_container(network, "backend-2", "B")
        .await
        .unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
  proxy {
    upstream "http://backend-1:3000"
    upstream "http://backend-2:3000"
    algorithm "consistent_hash"
    affinity header {
      name "X-Sticky"
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
    let url = format!("http://localhost:{port}/whoami");

    // Test sticky routing for user-1
    let mut responses_user1 = Vec::new();
    for _ in 0..5 {
        let resp = client
            .get(&url)
            .header("X-Sticky", "user-1")
            .send()
            .await
            .unwrap();
        responses_user1.push(resp.text().await.unwrap());
    }
    let first1 = &responses_user1[0];
    for r in &responses_user1 {
        assert_eq!(r, first1, "User-1 should always route to the same backend");
    }

    // Test sticky routing for user-2 (should be consistent, hopefully different from user-1)
    let mut responses_user2 = Vec::new();
    for _ in 0..5 {
        let resp = client
            .get(&url)
            .header("X-Sticky", "user-2")
            .send()
            .await
            .unwrap();
        responses_user2.push(resp.text().await.unwrap());
    }
    let first2 = &responses_user2[0];
    for r in &responses_user2 {
        assert_eq!(r, first2, "User-2 should always route to the same backend");
    }

    ferron.stop().await.unwrap();
}
