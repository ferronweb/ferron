use std::io::Write;
use std::path::Path;

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};

mod common;

async fn create_ferron_container(
    network: &str,
    webroot_dir: &Path,
    config_file: &Path,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let ferron_image = self::common::build_ferron_image().await?;
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

async fn create_otlp_container(
    network: &str,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    use std::time::Duration;

    let mut attempts = 0;
    loop {
        attempts += 1;
        let otlp_image = self::common::build_otlp_image().await?;
        let start_res = otlp_image
            .with_exposed_port(ContainerPort::Tcp(4318))
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
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

#[tokio::test]
async fn test_otlp_traces_exported() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let webroot_dir = tempfile::Builder::new()
        .permissions(std::os::unix::fs::PermissionsExt::from_mode(0o777))
        .tempdir()
        .unwrap();
    #[cfg(unix)]
    let mut config_file = tempfile::Builder::new()
        .permissions(std::os::unix::fs::PermissionsExt::from_mode(0o666))
        .tempfile()
        .unwrap();

    #[cfg(not(unix))]
    let webroot_dir = tempfile::tempdir().unwrap();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    config_file
        .as_file_mut()
        .write_all(
            r#"*:80 {
  root "/var/www/ferron"
  observability {
    provider otlp
    service_name "e2e-otlp"
    traces "http://otlp:4318/v1/traces" {
      protocol "http/protobuf"
    }
  }
}
"#
            .as_bytes(),
        )
        .unwrap();

    common::write_file(webroot_dir.path().join("basic.txt"), b"hello").unwrap();

    let network = "e2e-test-otlp";

    let otlp = create_otlp_container(network).await.unwrap();
    let ferron = create_ferron_container(network, webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let http_port = ferron
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let otlp_port = otlp
        .get_host_port_ipv4(ContainerPort::Tcp(4318))
        .await
        .unwrap();

    let client = reqwest::Client::new();

    // Trigger a request to produce a trace
    let _ = client
        .get(format!("http://localhost:{}/basic.txt", http_port))
        .send()
        .await
        .unwrap();

    // Poll the OTLP mock collector until it reports at least one received payload
    let received_url = format!("http://localhost:{}/received", otlp_port);
    let mut found = false;
    for _ in 0..60 {
        if let Ok(resp) = client.get(&received_url).send().await
            && resp.status().is_success()
            && let Ok(json) = resp.json::<serde_json::Value>().await
            && json.get("count").and_then(|v| v.as_u64()).unwrap_or(0) > 0
        {
            found = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }

    assert!(found, "OTLP collector did not receive traces");

    ferron.stop().await.unwrap();
    otlp.stop().await.unwrap();
}
