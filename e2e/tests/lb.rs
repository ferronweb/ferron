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
        // No wait strategy here because we want to test availability which might take time due to backends
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
async fn test_lb() {
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

    let network = "e2e-test-lb";

    // Start backends
    let _backend1 = create_backend_container(network, "backend-1")
        .await
        .unwrap();
    let _backend2 = create_backend_container(network, "backend-2")
        .await
        .unwrap();
    let _backend3 = create_backend_container(network, "backend-3")
        .await
        .unwrap();

    // Write Ferron config
    config_file
        .as_file_mut()
        .write_all(
            br#"
ferron-random:80 {
  proxy {
    upstream "http://backend-1:3000"
    upstream "http://backend-2:3000"
    upstream "http://backend-3:3000"
    lb_algorithm "random"
  }
}

ferron-round-robin:80 {
  proxy {
    upstream "http://backend-1:3000"
    upstream "http://backend-2:3000"
    upstream "http://backend-3:3000"
    lb_algorithm "round_robin"
  }
}

ferron-least-conn:80 {
  proxy {
    upstream "http://backend-1:3000"
    upstream "http://backend-2:3000"
    upstream "http://backend-3:3000"
    lb_algorithm "least_conn"
  }
}

ferron-two-random:80 {
  proxy {
    upstream "http://backend-1:3000"
    upstream "http://backend-2:3000"
    upstream "http://backend-3:3000"
    lb_algorithm "two_random"
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

    // Fix test flakiness, maybe caused by networking issues?
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Helper to test an algorithm
    let test_algo = |host: &'static str| {
        let client = client.clone();
        async move {
            for _ in 0..3 {
                let response = client
                    .get(format!("http://localhost:{}/", port))
                    .header("Host", host)
                    .send()
                    .await
                    .unwrap();
                assert_eq!(response.status(), reqwest::StatusCode::OK);
                assert_eq!(response.text().await.unwrap(), "Hello, World!");
            }
        }
    };

    test_algo("ferron-random").await;
    test_algo("ferron-round-robin").await;
    test_algo("ferron-least-conn").await;
    test_algo("ferron-two-random").await;
}
