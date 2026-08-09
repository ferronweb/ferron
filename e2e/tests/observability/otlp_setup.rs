//! Shared helpers for the OTLP e2e tests: container setup, test files, and
//! polling the mock collector's `/received` endpoint.

use std::path::Path;

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};

pub struct TestFiles {
    pub webroot: tempfile::TempDir,
    pub config: tempfile::NamedTempFile,
}

/// Create the temporary webroot directory and config file used by the tests.
pub fn create_test_files() -> TestFiles {
    let webroot = crate::common::create_temp_dir();
    let config = crate::common::create_temp_file();
    TestFiles { webroot, config }
}

pub async fn create_ferron_container(
    network: &str,
    webroot_dir: &Path,
    config_file: &Path,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let ferron_image = crate::common::build_ferron_image().await?;
    ferron_image
        .with_exposed_port(ContainerPort::Tcp(80))
        .with_wait_for(WaitFor::Http(Box::new(
            HttpWaitStrategy::new("/")
                .with_port(ContainerPort::Tcp(80))
                .with_response_matcher(|_| true),
        )))
        .with_network(network)
        .with_mount(Mount::bind_mount(
            webroot_dir.to_string_lossy(),
            "/var/www/ferron",
        ))
        .with_mount(Mount::bind_mount(
            config_file.to_string_lossy(),
            "/etc/ferron.conf",
        ))
        .start()
        .await
}

pub async fn create_otlp_container(
    network: &str,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let mut attempts = 0;
    loop {
        attempts += 1;
        let otlp_image = crate::common::build_otlp_image().await?;
        let start_res = otlp_image
            .with_exposed_port(ContainerPort::Tcp(4318))
            .with_exposed_port(ContainerPort::Tcp(4317))
            // short fixed wait; test will poll the mock collector endpoint for received payloads
            .with_wait_for(WaitFor::seconds(2))
            .with_network(network)
            .with_hostname("otlp")
            .start()
            .await;

        match start_res {
            Ok(container) => return Ok(container),
            Err(err) => {
                if attempts >= 3 {
                    return Err(err);
                }
                eprintln!(
                    "otlp container start attempt {} failed: {:?}, retrying...",
                    attempts, err
                );
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        }
    }
}

/// Poll the mock collector's `/received` endpoint until `predicate` matches
/// the decoded payload, then return it. Returns `None` on timeout.
pub async fn poll_received(
    client: &reqwest::Client,
    received_url: &str,
    mut predicate: impl FnMut(&serde_json::Value) -> bool,
) -> Option<serde_json::Value> {
    for _ in 0..150 {
        if let Ok(resp) = client.get(received_url).send().await
            && resp.status().is_success()
            && let Ok(json) = resp.json::<serde_json::Value>().await
            && predicate(&json)
        {
            return Some(json);
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    None
}

/// Helpers to find a decoded metric by name in the `/received` payload.
pub fn find_metric<'a>(
    payload: &'a serde_json::Value,
    name: &str,
) -> Option<&'a serde_json::Value> {
    payload.get("metrics").and_then(|metrics| {
        metrics
            .as_array()
            .and_then(|metrics| metrics.iter().find(|metric| metric["name"] == name))
    })
}
