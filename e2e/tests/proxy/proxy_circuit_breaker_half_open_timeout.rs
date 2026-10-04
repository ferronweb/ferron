//! Circuit breaker half-open trial timeout E2E tests.
//!
//! These tests verify that when a half-open trial request is aborted before
//! response headers arrive, Ferron releases the single half-open trial slot
//! instead of leaving the backend permanently stuck in half-open.
//!
//! The abort is produced by the `timeout` pipeline directive: an upstream that
//! accepts the connection but withholds response headers past the timeout makes
//! the pipeline cancel `execute_proxy`, so neither the success nor the failure
//! recording path runs.

use std::io::Write;

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};

use crate::common;

async fn create_backend_container(
    network: &str,
    backend_name: &str,
    unstable_fails: &str,
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
        .with_hostname("backend")
        .with_env_var("BACKEND_NAME", backend_name)
        .with_env_var("UNSTABLE_FAILS", unstable_fails)
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
        .with_exposed_port(ContainerPort::Tcp(8889))
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

/// Read a counter sample for `family` from the Prometheus endpoint.
async fn scrape_counter(ferron: &ContainerAsync<GenericImage>, family: &str) -> Option<f64> {
    let port = ferron
        .get_host_port_ipv4(ContainerPort::Tcp(8889))
        .await
        .ok()?;
    let body = reqwest::get(format!("http://localhost:{port}/metrics"))
        .await
        .ok()?
        .text()
        .await
        .ok()?;
    body.lines()
        .filter(|line| line.starts_with(family) && !line.starts_with('#'))
        .filter_map(|line| line.rsplit_once(' '))
        .filter_map(|(_, value)| value.trim().parse::<f64>().ok())
        .sum::<f64>()
        .into()
}

async fn ferron_logs(ferron: &ContainerAsync<GenericImage>) -> String {
    String::from_utf8(ferron.stdout_to_vec().await.unwrap_or_default()).unwrap_or_default()
}

/// Test that a half-open trial aborted by the pipeline timeout does not
/// permanently eject the backend.
///
/// The backend fails its first `/unstable` request with a 503, which trips the
/// circuit because `record_5xx` is enabled and `max_fails` is 1. After
/// `open_duration` elapses the next request becomes the half-open trial. That
/// trial targets `/unstable?sleep=5000`, so the `timeout "1s"` pipeline
/// directive cancels it before response headers arrive. Ferron must then
/// release the trial slot so the following request can recover the circuit.
#[tokio::test]
async fn test_half_open_trial_timeout_releases_slot() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = common::create_temp_file();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-cb-half-open-timeout";

    // `UNSTABLE_FAILS=1` makes the first `/unstable` call answer 503 and every
    // later call answer 200, so the backend is healthy again once the circuit
    // allows another trial through.
    let _backend = create_backend_container(network, "backend", "1")
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

  observability {
    provider prometheus
    endpoint_listen "0.0.0.0:8889"
  }

  proxy {
    upstream "http://backend:3000"
    retry_connection false

    circuit_breaker {
      max_fails 1
      window "30s"
      open_duration "1s"
      consecutive_passes 1
      record_5xx true
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
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .unwrap();

    let base = format!("http://localhost:{port}");

    // The first request trips the circuit: the backend answers 503 and
    // `record_5xx` counts it as a failure.
    let tripped = client
        .get(format!("{base}/unstable"))
        .send()
        .await
        .unwrap();
    assert_eq!(tripped.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);

    // Wait out `open_duration` so the next request may take the half-open slot.
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

    // The trial targets a slow endpoint, so the 1s pipeline timeout cancels it
    // before any response header arrives.
    let trial = client
        .get(format!("{base}/unstable?sleep=5000"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        trial.status(),
        reqwest::StatusCode::REQUEST_TIMEOUT,
        "the half-open trial should have been cancelled by the pipeline timeout"
    );

    // The backend is healthy, so this request recovers the circuit. Without the
    // trial slot being released it would keep answering 503 instead.
    let mut recovered = false;
    for _ in 0..10 {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let response = client.get(format!("{base}/unstable")).send().await.unwrap();
        if response.status() == reqwest::StatusCode::OK {
            assert_eq!(response.text().await.unwrap(), "backend");
            recovered = true;
            break;
        }
    }
    assert!(
        recovered,
        "backend stayed stuck in half-open after its trial request timed out; ferron logs:\n{}",
        ferron_logs(&ferron).await
    );

    // The guard also counts the abandoned trial so operators can see how often
    // upstreams hang past the pipeline timeout.
    let timeouts = scrape_counter(&ferron, "ferron_proxy_circuit_half_open_timeouts_total").await;
    assert!(
        timeouts.unwrap_or(0.0) >= 1.0,
        "expected at least one `ferron.proxy.circuit.half_open_timeouts` sample, got {timeouts:?}"
    );

    ferron.stop().await.unwrap();
}