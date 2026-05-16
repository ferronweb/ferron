use std::io::Write;
use std::path::Path;
use std::time::Duration;

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
            "/var/www/html",
        ))
        .with_mount(Mount::bind_mount(
            config_file.to_string_lossy(),
            "/etc/ferron.conf",
        ))
        .start()
        .await
}

#[tokio::test]
async fn test_fcgi_backend_reset_midrequest() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let wwwroot_dir = tempfile::Builder::new()
        .permissions(std::os::unix::fs::PermissionsExt::from_mode(0o777))
        .tempdir()
        .unwrap();
    #[cfg(not(unix))]
    let wwwroot_dir = tempfile::tempdir().unwrap();

    #[cfg(unix)]
    let mut config_file = tempfile::Builder::new()
        .permissions(std::os::unix::fs::PermissionsExt::from_mode(0o666))
        .tempfile()
        .unwrap();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    // Long-running PHP script so we can kill the backend mid-request
    self::common::write_file(
        wwwroot_dir.path().join("index.php"),
        br#"<?php
sleep(10);
echo "Hello after sleep";
"#,
    )
    .unwrap();

    config_file
        .as_file_mut()
        .write_all(
            r#"*:80 {
  root "/var/www/html"
  fcgi_php "tcp://php-fpm:9000/"
}
"#
            .as_bytes(),
        )
        .unwrap();

    let network = "e2e-test-fcgi-reset";

    // Start PHP-FPM container
    let php_fpm = GenericImage::new("php", "8.4-fpm")
        .with_exposed_port(ContainerPort::Tcp(9000))
        .with_wait_for(WaitFor::seconds(3))
        .with_network(network)
        .with_mount(Mount::bind_mount(
            wwwroot_dir.path().to_string_lossy().to_string(),
            "/var/www/html",
        ))
        .with_hostname("php-fpm")
        .start()
        .await
        .unwrap();

    // Start Ferron container
    let ferron = create_ferron_container(&network, wwwroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let port = ferron
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap();

    // Start the request in a background task
    let client_clone = client.clone();
    let req_future = tokio::spawn(async move {
        client_clone.get(format!("http://localhost:{}/", port)).send().await
    });

    // Give the request a short moment to start, then stop the backend to simulate a mid-request reset
    tokio::time::sleep(Duration::from_millis(200)).await;
    php_fpm.stop().await.unwrap();

    // Wait for the request to complete or fail
    match tokio::time::timeout(Duration::from_secs(10), req_future).await {
        Ok(join_res) => match join_res.unwrap() {
            Ok(resp) => {
                // Ferron should surface an upstream error; allow either a 502/5xx or client error
                assert!(resp.status().is_server_error() || resp.status() == reqwest::StatusCode::BAD_GATEWAY);
            }
            Err(_) => {
                // request failed at the client level -> acceptable
            }
        },
        Err(_) => {
            // timed out -> accept as failure mode
        }
    }

    ferron.stop().await.unwrap();
}
