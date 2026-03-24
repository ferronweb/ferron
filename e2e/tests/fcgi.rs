use std::io::Write;
use std::path::Path;
#[cfg(unix)]
use std::{fs::Permissions, os::unix::fs::PermissionsExt};

use testcontainers::{
  ContainerAsync, GenericImage, ImageExt, TestcontainersError,
  core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
  runners::AsyncRunner,
};

mod common;

async fn create_php_fpm_container(
  network: &str,
  wwwroot_dir: &Path,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
  GenericImage::new("php", "8.4-fpm")
    .with_exposed_port(ContainerPort::Tcp(9000))
    .with_wait_for(WaitFor::seconds(3))
    .with_network(network)
    .with_mount(Mount::bind_mount(wwwroot_dir.to_string_lossy(), "/var/www/html"))
    .with_hostname("php-fpm")
    .start()
    .await
}

async fn create_fcgiwrap_container(
  network: &str,
  wwwroot_dir: &Path,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
  let fcgiwrap_image = self::common::build_fcgiwrap_image().await?;
  fcgiwrap_image
    .with_exposed_port(ContainerPort::Tcp(9000))
    .with_wait_for(WaitFor::seconds(2))
    .with_network(network)
    .with_mount(Mount::bind_mount(wwwroot_dir.to_string_lossy(), "/var/www/html"))
    .with_hostname("fcgiwrap")
    .start()
    .await
}

async fn create_ferron_container(
  network: &str,
  wwwroot_dir: &Path,
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
    .with_mount(Mount::bind_mount(wwwroot_dir.to_string_lossy(), "/var/www/html"))
    .with_mount(Mount::bind_mount(config_file.to_string_lossy(), "/etc/ferron.kdl"))
    .start()
    .await
}

#[tokio::test]
async fn test_fcgi_php_hello_world() {
  let _ = rustls::crypto::ring::default_provider().install_default();

  // Set umask to 000 to ensure that the directories are accessible to the container.
  #[cfg(unix)]
  nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

  // Prepare directories
  #[cfg(unix)]
  let wwwroot_dir = tempfile::Builder::new()
    .permissions(Permissions::from_mode(0o777))
    .tempdir()
    .unwrap();
  #[cfg(not(unix))]
  let wwwroot_dir = tempfile::tempdir().unwrap();

  // Prepare config file
  #[cfg(unix)]
  let mut config_file = tempfile::Builder::new()
    .permissions(Permissions::from_mode(0o666))
    .tempfile()
    .unwrap();
  #[cfg(not(unix))]
  let mut config_file = tempfile::NamedTempFile::new().unwrap();

  // Write PHP file
  self::common::write_file(
    wwwroot_dir.path().join("index.php"),
    br#"<?php
echo "Hello, World!";
"#,
  )
  .unwrap();

  // Write Ferron config
  config_file
    .as_file_mut()
    .write_all(
      r#"
:80 {
  root "/var/www/html"
  fcgi_php "tcp://php-fpm:9000/"
}
"#
      .as_bytes(),
    )
    .unwrap();

  let network = "e2e-test-fcgi";

  // Start PHP-FPM container
  let _php_fpm = create_php_fpm_container(&network, wwwroot_dir.path()).await.unwrap();

  // Start Ferron container
  let ferron = create_ferron_container(&network, wwwroot_dir.path(), config_file.path())
    .await
    .unwrap();

  // Test the PHP script
  let port = ferron.get_host_port_ipv4(ContainerPort::Tcp(80)).await.unwrap();
  let response = reqwest::get(format!("http://localhost:{}/", port)).await.unwrap();

  assert_eq!(response.status(), reqwest::StatusCode::OK);
  let body = response.text().await.unwrap();
  assert_eq!(body.trim(), "Hello, World!");

  ferron.stop().await.unwrap();
}

#[tokio::test]
async fn test_fcgiwrap_cgi_hello_world() {
  let _ = rustls::crypto::ring::default_provider().install_default();

  // Set umask to 000 to ensure that the directories are accessible to the container.
  #[cfg(unix)]
  nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

  // Prepare directories
  #[cfg(unix)]
  let wwwroot_dir = tempfile::Builder::new()
    .permissions(Permissions::from_mode(0o777))
    .tempdir()
    .unwrap();
  #[cfg(not(unix))]
  let wwwroot_dir = tempfile::tempdir().unwrap();

  // Prepare config file
  #[cfg(unix)]
  let mut config_file = tempfile::Builder::new()
    .permissions(Permissions::from_mode(0o666))
    .tempfile()
    .unwrap();
  #[cfg(not(unix))]
  let mut config_file = tempfile::NamedTempFile::new().unwrap();

  // Write CGI script
  let _ = self::common::create_dir(wwwroot_dir.path().join("cgi-bin"));
  self::common::write_file(
    wwwroot_dir.path().join("cgi-bin/index.cgi"),
    br#"#!/bin/sh
echo "Content-Type: text/plain"
echo ""
echo "Hello, World!"
"#,
  )
  .unwrap();
  #[cfg(unix)]
  let _ = nix::sys::stat::fchmod(
    std::fs::File::open(wwwroot_dir.path().join("cgi-bin/index.cgi")).unwrap(),
    nix::sys::stat::Mode::from_bits(0o777).unwrap(),
  ); // CGI must be executable

  // Write Ferron config
  config_file
    .as_file_mut()
    .write_all(
      r#"
:80 {
  root "/var/www/html"

  location "/cgi-bin" {
    fcgi "tcp://fcgiwrap:9000/" pass=#false
    fcgi_extension ".cgi"
  }
}
"#
      .as_bytes(),
    )
    .unwrap();

  let network = "e2e-test-fcgiwrap";

  // Start fcgiwrap container
  let _fcgiwrap = create_fcgiwrap_container(&network, wwwroot_dir.path()).await.unwrap();

  // Start Ferron container
  let ferron = create_ferron_container(&network, wwwroot_dir.path(), config_file.path())
    .await
    .unwrap();

  // Test the CGI script via fcgiwrap
  let port = ferron.get_host_port_ipv4(ContainerPort::Tcp(80)).await.unwrap();
  let response = reqwest::get(format!("http://localhost:{}/cgi-bin/index.cgi", port))
    .await
    .unwrap();

  assert_eq!(response.status(), reqwest::StatusCode::OK);
  let body = response.text().await.unwrap();
  assert_eq!(body.trim(), "Hello, World!");

  ferron.stop().await.unwrap();
}
