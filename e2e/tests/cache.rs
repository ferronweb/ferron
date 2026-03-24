#[cfg(unix)]
use std::{fs::Permissions, os::unix::fs::PermissionsExt};
use std::{io::Write, path::Path};

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
    .with_wait_for(WaitFor::Http(Box::new(
      HttpWaitStrategy::new("/")
        .with_port(ContainerPort::Tcp(80))
        .with_response_matcher(|_| true),
    )))
    .with_network("bridge")
    .with_mount(Mount::bind_mount(webroot_dir.to_string_lossy(), "/var/www/ferron"))
    .with_mount(Mount::bind_mount(config_file.to_string_lossy(), "/etc/ferron.kdl"))
    .start()
    .await
}

#[tokio::test]
async fn test_cache_hit() {
  let _ = rustls::crypto::ring::default_provider().install_default();

  // Set umask to 000 to ensure that the webroot directory is accessible to the container.
  #[cfg(unix)]
  nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

  #[cfg(unix)]
  let webroot_dir = tempfile::Builder::new()
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
  let mut config_file = tempfile::NamedTempFile::new().unwrap();

  config_file
    .as_file_mut()
    .write_all(
      r#"
      :80 {
        root "/var/www/ferron"
        file_cache_control "public, max-age=60"
        cache
      }
  "#
      .as_bytes(),
    )
    .unwrap();

  let container = create_ferron_container(webroot_dir.path(), config_file.path())
    .await
    .unwrap();

  self::common::write_file(webroot_dir.path().join("test.txt"), "v1".as_bytes()).unwrap();
  let response = reqwest::get(format!(
    "http://localhost:{}/test.txt",
    container.get_host_port_ipv4(ContainerPort::Tcp(80)).await.unwrap()
  ))
  .await
  .unwrap();

  assert_eq!(response.status(), reqwest::StatusCode::OK);
  assert_eq!(
    response.headers().get("X-Ferron-Cache"),
    Some(&reqwest::header::HeaderValue::from_static("MISS"))
  );
  assert_eq!(&*response.bytes().await.unwrap(), b"v1");

  self::common::write_file(webroot_dir.path().join("test.txt"), "v2".as_bytes()).unwrap();
  let response = reqwest::get(format!(
    "http://localhost:{}/test.txt",
    container.get_host_port_ipv4(ContainerPort::Tcp(80)).await.unwrap()
  ))
  .await
  .unwrap();

  assert_eq!(response.status(), reqwest::StatusCode::OK);
  assert_eq!(
    response.headers().get("X-Ferron-Cache"),
    Some(&reqwest::header::HeaderValue::from_static("HIT"))
  );
  assert_eq!(&*response.bytes().await.unwrap(), b"v1");

  container.stop().await.unwrap();
}

#[tokio::test]
async fn test_cache_expiry() {
  let _ = rustls::crypto::ring::default_provider().install_default();

  // Set umask to 000 to ensure that the webroot directory is accessible to the container.
  #[cfg(unix)]
  nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

  #[cfg(unix)]
  let webroot_dir = tempfile::Builder::new()
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
  let mut config_file = tempfile::NamedTempFile::new().unwrap();

  config_file
    .as_file_mut()
    .write_all(
      r#"
      :80 {
        root "/var/www/ferron"
        file_cache_control "public, max-age=2"
        cache
      }
  "#
      .as_bytes(),
    )
    .unwrap();

  let container = create_ferron_container(webroot_dir.path(), config_file.path())
    .await
    .unwrap();

  self::common::write_file(webroot_dir.path().join("test.txt"), "v1".as_bytes()).unwrap();
  let response = reqwest::get(format!(
    "http://localhost:{}/test.txt",
    container.get_host_port_ipv4(ContainerPort::Tcp(80)).await.unwrap()
  ))
  .await
  .unwrap();

  assert_eq!(response.status(), reqwest::StatusCode::OK);
  assert_eq!(
    response.headers().get("X-Ferron-Cache"),
    Some(&reqwest::header::HeaderValue::from_static("MISS"))
  );
  assert_eq!(&*response.bytes().await.unwrap(), b"v1");

  tokio::time::sleep(std::time::Duration::from_secs(3)).await;

  self::common::write_file(webroot_dir.path().join("test.txt"), "v2".as_bytes()).unwrap();
  let response = reqwest::get(format!(
    "http://localhost:{}/test.txt",
    container.get_host_port_ipv4(ContainerPort::Tcp(80)).await.unwrap()
  ))
  .await
  .unwrap();

  assert_eq!(response.status(), reqwest::StatusCode::OK);
  assert_eq!(
    response.headers().get("X-Ferron-Cache"),
    Some(&reqwest::header::HeaderValue::from_static("MISS"))
  );
  assert_eq!(&*response.bytes().await.unwrap(), b"v2");

  container.stop().await.unwrap();
}

#[tokio::test]
async fn test_cache_vary() {
  let _ = rustls::crypto::ring::default_provider().install_default();

  // Set umask to 000 to ensure that the webroot directory is accessible to the container.
  #[cfg(unix)]
  nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

  #[cfg(unix)]
  let webroot_dir = tempfile::Builder::new()
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
  let mut config_file = tempfile::NamedTempFile::new().unwrap();

  config_file
    .as_file_mut()
    .write_all(
      r#"
      :80 {
        root "/var/www/ferron"
        file_cache_control "public, max-age=2"
        cache
        cache_vary "X-Test-Header"
      }
  "#
      .as_bytes(),
    )
    .unwrap();

  let container = create_ferron_container(webroot_dir.path(), config_file.path())
    .await
    .unwrap();

  self::common::write_file(webroot_dir.path().join("test.txt"), "v1".as_bytes()).unwrap();
  let response = reqwest::Client::new()
    .get(format!(
      "http://localhost:{}/test.txt",
      container.get_host_port_ipv4(ContainerPort::Tcp(80)).await.unwrap()
    ))
    .header("X-Test-Header", "A")
    .send()
    .await
    .unwrap();

  assert_eq!(response.status(), reqwest::StatusCode::OK);
  assert_eq!(
    response.headers().get("X-Ferron-Cache"),
    Some(&reqwest::header::HeaderValue::from_static("MISS"))
  );
  assert_eq!(&*response.bytes().await.unwrap(), b"v1");

  self::common::write_file(webroot_dir.path().join("test.txt"), "v2".as_bytes()).unwrap();
  let response = reqwest::Client::new()
    .get(format!(
      "http://localhost:{}/test.txt",
      container.get_host_port_ipv4(ContainerPort::Tcp(80)).await.unwrap()
    ))
    .header("X-Test-Header", "B")
    .send()
    .await
    .unwrap();

  assert_eq!(response.status(), reqwest::StatusCode::OK);
  assert_eq!(
    response.headers().get("X-Ferron-Cache"),
    Some(&reqwest::header::HeaderValue::from_static("MISS"))
  );
  assert_eq!(&*response.bytes().await.unwrap(), b"v2");

  let response = reqwest::Client::new()
    .get(format!(
      "http://localhost:{}/test.txt",
      container.get_host_port_ipv4(ContainerPort::Tcp(80)).await.unwrap()
    ))
    .header("X-Test-Header", "A")
    .send()
    .await
    .unwrap();

  assert_eq!(response.status(), reqwest::StatusCode::OK);
  assert_eq!(
    response.headers().get("X-Ferron-Cache"),
    Some(&reqwest::header::HeaderValue::from_static("HIT"))
  );
  assert_eq!(&*response.bytes().await.unwrap(), b"v1");

  container.stop().await.unwrap();
}
