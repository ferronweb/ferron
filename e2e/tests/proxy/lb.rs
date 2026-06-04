#[cfg(unix)]
use std::{io::Write, path::Path};

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};


async fn create_backend_container(
    network: &str,
    alias: &str,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let backend_image = crate::common::build_backend_image().await?;
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
    let ferron_image = crate::common::build_ferron_image().await?;
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
async fn test_lb() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = crate::common::create_temp_file();
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
    algorithm "random"
  }
}

ferron-round-robin:80 {
  proxy {
    upstream "http://backend-1:3000"
    upstream "http://backend-2:3000"
    upstream "http://backend-3:3000"
    algorithm "round_robin"
  }
}

ferron-least-conn:80 {
  proxy {
    upstream "http://backend-1:3000"
    upstream "http://backend-2:3000"
    upstream "http://backend-3:3000"
    algorithm "least_conn"
  }
}

ferron-two-random:80 {
  proxy {
    upstream "http://backend-1:3000"
    upstream "http://backend-2:3000"
    upstream "http://backend-3:3000"
    algorithm "two_random"
  }
}

ferron-p2c-ewma:80 {
  proxy {
    upstream "http://backend-1:3000"
    upstream "http://backend-2:3000"
    upstream "http://backend-3:3000"
    algorithm "p2c_ewma"
  }
}

ferron-weighted-round-robin:80 {
  proxy {
    upstream "http://backend-1:3000" {
      weight 2
    }
    upstream "http://backend-2:3000" {
      weight 2
    }
    upstream "http://backend-3:3000" {
      weight 1
    }
    algorithm "round_robin"
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
    let test_algo = |host: &'static str, count: usize| {
        let client = client.clone();
        async move {
            for _ in 0..count {
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

    test_algo("ferron-random", 3).await;
    test_algo("ferron-round-robin", 3).await;
    test_algo("ferron-least-conn", 3).await;
    test_algo("ferron-two-random", 3).await;
    test_algo("ferron-p2c-ewma", 33).await; // 10 samples per backend would be warmup...
    test_algo("ferron-weighted-round-robin", 3).await;
}

#[tokio::test]
async fn test_lb_retry_connection() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = crate::common::create_temp_file();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-lb-retry";

    // One healthy backend, one closed port
    let _backend = create_backend_container(network, "backend-ok")
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
    let url = format!("http://localhost:{port}/whoami");

    // Send multiple requests. They should all succeed because of retries,
    // even though half of the initial choices will hit the failing backend.
    for _ in 0..10 {
        let resp = client.get(&url).send().await.unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::OK);
        assert_eq!(resp.text().await.unwrap(), "backend");
    }

    ferron.stop().await.unwrap();
}
