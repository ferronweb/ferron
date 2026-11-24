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

#[tokio::test]
async fn test_circuit_breaker_trips_on_slow_upstream() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let mut config_file = common::create_temp_file();

    let network = "e2e-test-cb-latency-trip";

    let _backend_slow = create_backend_container(network, "backend-slow", "backend-slow")
        .await
        .unwrap();
    let _backend_fast = create_backend_container(network, "backend-fast", "backend-fast")
        .await
        .unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
  proxy {
    upstream "http://backend-slow:3000"
    upstream "http://backend-fast:3000"

    algorithm round_robin
    retry_connection false

    circuit_breaker {
      max_fails 1
      window "30s"
      open_duration "30s"
      consecutive_passes 2
      latency_threshold "0.1s"
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

    let response1 = client
        .get(format!("http://localhost:{}/unstable?sleep=500", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response1.status(), reqwest::StatusCode::OK);
    assert_eq!(response1.text().await.unwrap(), "backend-slow");

    // Give the circuit breaker a moment to register the failure
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Second request should go to backend-fast because backend-slow's circuit is open.
    let response2 = client
        .get(format!("http://localhost:{}/whoami", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response2.status(), reqwest::StatusCode::OK);
    assert_eq!(response2.text().await.unwrap(), "backend-fast");

    ferron.stop().await.unwrap();
}

#[tokio::test]
async fn test_circuit_breaker_does_not_trip_without_latency_threshold() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let mut config_file = common::create_temp_file();

    let network = "e2e-test-cb-latency-notrip";

    let _backend = create_backend_container(network, "backend-slow", "backend-slow")
        .await
        .unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
  proxy {
    upstream "http://backend-slow:3000"

    algorithm round_robin

    circuit_breaker {
      max_fails 1
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

    // Send multiple slow requests, without latency_threshold, the circuit should never trip.
    for _ in 0..3 {
        let response = client
            .get(format!("http://localhost:{}/unstable?sleep=200", port))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert_eq!(response.text().await.unwrap(), "backend-slow");
    }

    ferron.stop().await.unwrap();
}

#[tokio::test]
async fn test_circuit_breaker_latency_half_open_recovery() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let mut config_file = common::create_temp_file();

    let network = "e2e-test-cb-latency-halfopen";

    let _backend_slow = create_backend_container(network, "backend-slow", "backend-slow")
        .await
        .unwrap();
    let _backend_fast = create_backend_container(network, "backend-fast", "backend-fast")
        .await
        .unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
  proxy {
    upstream "http://backend-slow:3000"
    upstream "http://backend-fast:3000"

    algorithm round_robin
    retry_connection false

    circuit_breaker {
      max_fails 1
      window "30s"
      open_duration "2s"
      consecutive_passes 1
      latency_threshold "0.1s"
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

    let response = client
        .get(format!("http://localhost:{}/unstable?sleep=500", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), "backend-slow");

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let response = client
        .get(format!("http://localhost:{}/whoami", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), "backend-fast");

    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    let response = client
        .get(format!("http://localhost:{}/whoami", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), "backend-slow"); // backend-slow just recovered

    ferron.stop().await.unwrap();
}
