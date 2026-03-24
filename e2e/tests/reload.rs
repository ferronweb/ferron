#[cfg(unix)]
use std::{fs::Permissions, os::unix::fs::PermissionsExt};
use std::{io::Write, path::Path};

use testcontainers::{
  ContainerAsync, GenericImage, ImageExt, TestcontainersError,
  bollard::query_parameters::KillContainerOptionsBuilder,
  core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
  runners::AsyncRunner,
};

mod common;

async fn create_ferron_container(
  webroot_dir: &Path,
  webroot_dir2: &Path,
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
    .with_network("bridge")
    .with_mount(Mount::bind_mount(webroot_dir.to_string_lossy(), "/var/www/ferron"))
    .with_mount(Mount::bind_mount(webroot_dir2.to_string_lossy(), "/var/www/ferron2"))
    .with_mount(Mount::bind_mount(config_file.to_string_lossy(), "/etc/ferron.kdl"))
    .start()
    .await
}

#[tokio::test]
async fn test_config_reload() {
  let _ = rustls::crypto::ring::default_provider().install_default();

  #[cfg(unix)]
  nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

  #[cfg(unix)]
  let webroot_dir = tempfile::Builder::new()
    .permissions(Permissions::from_mode(0o777))
    .tempdir()
    .unwrap();
  #[cfg(unix)]
  let webroot_dir2 = tempfile::Builder::new()
    .permissions(Permissions::from_mode(0o777))
    .tempdir()
    .unwrap();
  #[cfg(unix)]
  let mut config_file = tempfile::Builder::new()
    .permissions(Permissions::from_mode(0o666))
    .tempfile()
    .unwrap();
  #[cfg(not(unix))]
  let webroot_dir = tempfile::tempdir().unwrap();
  #[cfg(not(unix))]
  let webroot_dir2 = tempfile::tempdir().unwrap();
  #[cfg(not(unix))]
  let mut config_file = tempfile::NamedTempFile::new().unwrap();

  // Initial configuration
  config_file
    .as_file_mut()
    .write_all(
      r#"
:80 {
  root "/var/www/ferron"
}
"#
      .as_bytes(),
    )
    .unwrap();

  self::common::write_file(webroot_dir.path().join("test.txt"), b"before reload").unwrap();
  self::common::write_file(webroot_dir2.path().join("test.txt"), b"after reload").unwrap();

  let container = create_ferron_container(webroot_dir.path(), webroot_dir2.path(), config_file.path())
    .await
    .unwrap();

  let port = container.get_host_port_ipv4(ContainerPort::Tcp(80)).await.unwrap();
  let client = reqwest::Client::new();

  // Test before reload
  let response = client
    .get(format!("http://localhost:{}/test.txt", port))
    .send()
    .await
    .unwrap();

  assert_eq!(response.status(), reqwest::StatusCode::OK);
  assert_eq!(response.text().await.unwrap(), "before reload");

  // Update configuration to point to new webroot
  config_file
    .as_file_mut()
    .write_all(
      r#"
:80 {
  root "/var/www/ferron2"
}
"#
      .as_bytes(),
    )
    .unwrap();

  // Send SIGHUP to reload configuration
  testcontainers::bollard::Docker::connect_with_local_defaults()
    .unwrap()
    .kill_container(
      container.id(),
      Some(KillContainerOptionsBuilder::new().signal("SIGHUP").build()),
    )
    .await
    .unwrap();

  // Wait for reload to complete
  tokio::time::sleep(std::time::Duration::from_secs(1)).await;

  // Test after reload
  let response = client
    .get(format!("http://localhost:{}/test.txt", port))
    .send()
    .await
    .unwrap();

  assert_eq!(response.status(), reqwest::StatusCode::OK);
  assert_eq!(response.text().await.unwrap(), "after reload");

  container.stop().await.unwrap();
}
