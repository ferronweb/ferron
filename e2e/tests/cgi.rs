#[cfg(unix)]
use std::{fs::Permissions, os::unix::fs::PermissionsExt};
use std::{io::Write, path::Path};

use testcontainers::{
  ContainerAsync, GenericImage, ImageExt, TestcontainersError,
  core::{
    ContainerPort, Mount, WaitFor,
    wait::{ExitWaitStrategy, HttpWaitStrategy},
  },
  runners::AsyncRunner,
};

mod common;

async fn create_cgi_container(
  network: &str,
  cgi_bin_dir: &Path,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
  let cgi_image = self::common::build_cgi_image().await?;
  cgi_image
    .with_wait_for(WaitFor::Exit(ExitWaitStrategy::new()))
    .with_network(network)
    .with_mount(Mount::bind_mount(cgi_bin_dir.to_string_lossy(), "/usr/lib/cgi-bin"))
    .start()
    .await
}

async fn create_ferron_container(
  network: &str,
  cgi_bin_dir: &Path,
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
      cgi_bin_dir.to_string_lossy(),
      "/var/www/ferron/cgi-bin",
    ))
    .with_mount(Mount::bind_mount(config_file.to_string_lossy(), "/etc/ferron.conf"))
    .start()
    .await
}

#[tokio::test]
async fn test_cgi_hello_world() {
  let _ = rustls::crypto::ring::default_provider().install_default();

  // Set umask to 000 to ensure that the directories are accessible to the container.
  #[cfg(unix)]
  nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

  // Prepare directories
  #[cfg(unix)]
  let cgi_bin_dir = tempfile::Builder::new()
    .permissions(Permissions::from_mode(0o777))
    .tempdir()
    .unwrap();
  #[cfg(not(unix))]
  let cgi_bin_dir = tempfile::tempdir().unwrap();

  // Prepare config file
  #[cfg(unix)]
  let mut config_file = tempfile::Builder::new()
    .permissions(Permissions::from_mode(0o666))
    .tempfile()
    .unwrap();
  #[cfg(not(unix))]
  let mut config_file = tempfile::NamedTempFile::new().unwrap();

  // Write Ferron config
  config_file
    .as_file_mut()
    .write_all(
      r#"
*:80 {
  root "/var/www/ferron"
  cgi true
}
"#
      .as_bytes(),
    )
    .unwrap();

  let network = "e2e-test-cgi";

  // Start CGI container to copy the CGI binary
  let _cgi = create_cgi_container(&network, cgi_bin_dir.path()).await.unwrap();

  // Start Ferron container
  let ferron = create_ferron_container(&network, cgi_bin_dir.path(), config_file.path())
    .await
    .unwrap();

  // Test the CGI script
  let port = ferron.get_host_port_ipv4(ContainerPort::Tcp(80)).await.unwrap();
  let response = reqwest::get(format!("http://localhost:{}/cgi-bin/hello.cgi", port))
    .await
    .unwrap();

  assert_eq!(response.status(), reqwest::StatusCode::OK);
  let body = response.text().await.unwrap();
  assert_eq!(body.trim(), "Hello, World!");

  ferron.stop().await.unwrap();
}
