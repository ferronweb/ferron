use std::io::Write;
use std::path::Path;

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};

mod common;

async fn create_ferron_container(
    webroot_dir: &Path,
    config_file: &Path,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let ferron_image = self::common::build_ferron_image().await?;
    ferron_image
        .with_exposed_port(ContainerPort::Tcp(80))
        .with_exposed_port(ContainerPort::Tcp(8889))
        .with_wait_for(WaitFor::Http(Box::new(
            HttpWaitStrategy::new("/")
                .with_port(ContainerPort::Tcp(80))
                .with_response_matcher(|_| true),
        )))
        .with_network("bridge")
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

#[tokio::test]
async fn test_prometheus_metrics_exposed() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let webroot_dir = common::create_temp_dir();
    #[cfg(unix)]
    let mut config_file = common::create_temp_file();

    #[cfg(not(unix))]
    let webroot_dir = tempfile::tempdir().unwrap();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    // Minimal config enabling Prometheus observability and binding metrics to all interfaces
    config_file
        .as_file_mut()
        .write_all(
            r#"*:80 {
  root "/var/www/ferron"
  observability {
    provider prometheus
    endpoint_listen "0.0.0.0:8889"
  }
}
"#
            .as_bytes(),
        )
        .unwrap();

    // Basic content to generate metrics when requested
    common::write_file(webroot_dir.path().join("basic.txt"), b"hello").unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let http_port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let metrics_port = container
        .get_host_port_ipv4(ContainerPort::Tcp(8889))
        .await
        .unwrap();

    let client = reqwest::Client::new();

    // Trigger a request to cause metrics to be emitted and the Prometheus endpoint to start
    let _ = client
        .get(format!("http://localhost:{}/basic.txt", http_port))
        .send()
        .await
        .unwrap();

    // Poll the metrics endpoint until some metrics are present
    let metrics_url = format!("http://localhost:{}/metrics", metrics_port);
    let mut found = false;
    for _ in 0..30 {
        if let Ok(resp) = client.get(&metrics_url).send().await
            && resp.status().is_success()
            && let Ok(body) = resp.text().await
            && !body.trim().is_empty()
            && (body.contains("ferron") || body.contains("http_server") || body.contains("request"))
        {
            found = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }

    assert!(found, "Prometheus /metrics did not expose any metrics");

    container.stop().await.unwrap();
}
