#[cfg(unix)]
use std::{fs::Permissions, os::unix::fs::PermissionsExt};
use std::{
  io::{Read, Write},
  path::Path,
};

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
async fn test_dynamic_compression_gzip() {
  let _ = rustls::crypto::ring::default_provider().install_default();

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

  compressed #false
  dynamic_compressed
}
"#
      .as_bytes(),
    )
    .unwrap();

  self::common::write_file(webroot_dir.path().join("small.txt"), b"test content").unwrap();

  let container = create_ferron_container(webroot_dir.path(), config_file.path())
    .await
    .unwrap();

  let port = container.get_host_port_ipv4(ContainerPort::Tcp(80)).await.unwrap();
  let client = reqwest::Client::new();

  // Test gzip compression
  let response = client
    .get(format!("http://localhost:{}/small.txt", port))
    .header("Accept-Encoding", "gzip")
    .send()
    .await
    .unwrap();

  assert_eq!(response.status(), reqwest::StatusCode::OK);
  let bytes = response.bytes().await.unwrap();
  let mut decompressed = Vec::new();
  let _ = flate2::read::GzDecoder::new(bytes.as_ref())
    .read_to_end(&mut decompressed)
    .unwrap();
  assert_eq!(decompressed, b"test content");

  container.stop().await.unwrap();
}

#[tokio::test]
async fn test_dynamic_compression_deflate() {
  let _ = rustls::crypto::ring::default_provider().install_default();

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

  compressed #false
  dynamic_compressed
}
"#
      .as_bytes(),
    )
    .unwrap();

  self::common::write_file(webroot_dir.path().join("small.txt"), b"test content").unwrap();

  let container = create_ferron_container(webroot_dir.path(), config_file.path())
    .await
    .unwrap();

  let port = container.get_host_port_ipv4(ContainerPort::Tcp(80)).await.unwrap();
  let client = reqwest::Client::new();

  // Test gzip compression
  let response = client
    .get(format!("http://localhost:{}/small.txt", port))
    .header("Accept-Encoding", "deflate")
    .send()
    .await
    .unwrap();

  assert_eq!(response.status(), reqwest::StatusCode::OK);
  let bytes = response.bytes().await.unwrap();
  let mut decompressed = Vec::new();
  let _ = flate2::read::DeflateDecoder::new(bytes.as_ref())
    .read_to_end(&mut decompressed)
    .unwrap();
  assert_eq!(decompressed, b"test content");

  container.stop().await.unwrap();
}

#[tokio::test]
async fn test_dynamic_compression_brotli() {
  let _ = rustls::crypto::ring::default_provider().install_default();

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

  compressed #false
  dynamic_compressed
}
"#
      .as_bytes(),
    )
    .unwrap();

  self::common::write_file(webroot_dir.path().join("small.txt"), b"test content").unwrap();

  let container = create_ferron_container(webroot_dir.path(), config_file.path())
    .await
    .unwrap();

  let port = container.get_host_port_ipv4(ContainerPort::Tcp(80)).await.unwrap();
  let client = reqwest::Client::new();

  // Test gzip compression
  let response = client
    .get(format!("http://localhost:{}/small.txt", port))
    .header("Accept-Encoding", "br")
    .send()
    .await
    .unwrap();

  assert_eq!(response.status(), reqwest::StatusCode::OK);
  let mut bytes = std::io::Cursor::new(response.bytes().await.unwrap());
  let mut decompressed = Vec::new();

  brotli::BrotliDecompress(&mut bytes, &mut decompressed).unwrap();
  assert_eq!(decompressed, b"test content");

  container.stop().await.unwrap();
}

#[tokio::test]
async fn test_dynamic_compression_zstd() {
  let _ = rustls::crypto::ring::default_provider().install_default();

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

  compressed #false
  dynamic_compressed
}
"#
      .as_bytes(),
    )
    .unwrap();

  self::common::write_file(webroot_dir.path().join("small.txt"), b"test content").unwrap();

  let container = create_ferron_container(webroot_dir.path(), config_file.path())
    .await
    .unwrap();

  let port = container.get_host_port_ipv4(ContainerPort::Tcp(80)).await.unwrap();
  let client = reqwest::Client::new();

  // Test gzip compression
  let response = client
    .get(format!("http://localhost:{}/small.txt", port))
    .header("Accept-Encoding", "zstd")
    .send()
    .await
    .unwrap();

  assert_eq!(response.status(), reqwest::StatusCode::OK);
  let mut bytes = std::io::Cursor::new(response.bytes().await.unwrap());
  let mut decompressed = Vec::new();

  let mut decoder = zstd::Decoder::new(&mut bytes).unwrap();
  std::io::copy(&mut decoder, &mut decompressed).unwrap();

  assert_eq!(decompressed, b"test content");

  container.stop().await.unwrap();
}
