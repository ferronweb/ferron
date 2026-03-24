#![allow(dead_code)]

#[cfg(unix)]
use std::{
  io::Write,
  os::unix::fs::{DirBuilderExt, OpenOptionsExt},
};
use std::{path::PathBuf, sync::LazyLock};

use testcontainers::{
  GenericBuildableImage, GenericImage, TestcontainersError, core::BuildImageOptions, runners::AsyncBuilder,
};
use tokio::sync::Mutex;

static FERRON_IMAGE: std::sync::LazyLock<Mutex<Option<GenericImage>>> = LazyLock::new(|| Mutex::new(None));
static BACKEND_IMAGE: std::sync::LazyLock<Mutex<Option<GenericImage>>> = LazyLock::new(|| Mutex::new(None));
static BACKEND_GRPC_IMAGE: std::sync::LazyLock<Mutex<Option<GenericImage>>> = LazyLock::new(|| Mutex::new(None));
static CGI_IMAGE: std::sync::LazyLock<Mutex<Option<GenericImage>>> = LazyLock::new(|| Mutex::new(None));
static FCGIWRAP_IMAGE: std::sync::LazyLock<Mutex<Option<GenericImage>>> = LazyLock::new(|| Mutex::new(None));

pub async fn build_ferron_image() -> Result<GenericImage, TestcontainersError> {
  let mut ferron_image = FERRON_IMAGE.lock().await;
  if let Some(image) = ferron_image.as_ref() {
    return Ok(image.clone());
  }
  let ferron_image_built = GenericBuildableImage::new("e2e-test-ferron", "latest")
    .with_dockerfile(concat!(env!("CARGO_MANIFEST_DIR"), "/Dockerfile.test"))
    .with_file(concat!(env!("CARGO_MANIFEST_DIR"), "/.."), ".")
    .build_image_with(BuildImageOptions::new().with_skip_if_exists(true))
    .await?;
  ferron_image.replace(ferron_image_built.clone());
  Ok(ferron_image_built)
}

pub async fn build_backend_image() -> Result<GenericImage, TestcontainersError> {
  let mut backend_image = BACKEND_IMAGE.lock().await;
  if let Some(image) = backend_image.as_ref() {
    return Ok(image.clone());
  }
  let backend_image_built = GenericBuildableImage::new("e2e-test-backend", "latest")
    .with_dockerfile(concat!(env!("CARGO_MANIFEST_DIR"), "/images/backend/Dockerfile"))
    .with_file(concat!(env!("CARGO_MANIFEST_DIR"), "/images/backend"), ".")
    .build_image()
    .await?;
  backend_image.replace(backend_image_built.clone());
  Ok(backend_image_built)
}

pub async fn build_backend_grpc_image() -> Result<GenericImage, TestcontainersError> {
  let mut backend_grpc_image = BACKEND_GRPC_IMAGE.lock().await;
  if let Some(image) = backend_grpc_image.as_ref() {
    return Ok(image.clone());
  }
  let backend_grpc_image_built = GenericBuildableImage::new("e2e-test-backend-grpc", "latest")
    .with_dockerfile(concat!(env!("CARGO_MANIFEST_DIR"), "/images/backend-grpc/Dockerfile"))
    .with_file(concat!(env!("CARGO_MANIFEST_DIR"), "/images/backend-grpc"), ".")
    .build_image()
    .await?;
  backend_grpc_image.replace(backend_grpc_image_built.clone());
  Ok(backend_grpc_image_built)
}

pub async fn build_cgi_image() -> Result<GenericImage, TestcontainersError> {
  let mut cgi_image = CGI_IMAGE.lock().await;
  if let Some(image) = cgi_image.as_ref() {
    return Ok(image.clone());
  }
  let cgi_image_built = GenericBuildableImage::new("e2e-test-cgi", "latest")
    .with_dockerfile(concat!(env!("CARGO_MANIFEST_DIR"), "/images/cgi/Dockerfile"))
    .with_file(concat!(env!("CARGO_MANIFEST_DIR"), "/images/cgi"), ".")
    .build_image()
    .await?;
  cgi_image.replace(cgi_image_built.clone());
  Ok(cgi_image_built)
}

pub async fn build_fcgiwrap_image() -> Result<GenericImage, TestcontainersError> {
  let mut fcgiwrap_image = FCGIWRAP_IMAGE.lock().await;
  if let Some(image) = fcgiwrap_image.as_ref() {
    return Ok(image.clone());
  }
  let fcgiwrap_image_built = GenericBuildableImage::new("e2e-test-fcgiwrap", "latest")
    .with_dockerfile(concat!(env!("CARGO_MANIFEST_DIR"), "/images/fcgiwrap/Dockerfile"))
    .with_file(concat!(env!("CARGO_MANIFEST_DIR"), "/images/fcgiwrap"), ".")
    .build_image()
    .await?;
  fcgiwrap_image.replace(fcgiwrap_image_built.clone());
  Ok(fcgiwrap_image_built)
}

pub fn write_file(path: PathBuf, content: &[u8]) -> Result<(), std::io::Error> {
  #[cfg(unix)]
  let file = std::fs::OpenOptions::new()
    .write(true)
    .create(true)
    .truncate(true)
    .mode(0o666)
    .open(path);
  #[cfg(unix)]
  let result = file.and_then(|mut file| file.write_all(content));
  #[cfg(not(unix))]
  let result = std::fs::write(path, content);

  result
}

pub fn create_dir(path: PathBuf) -> Result<(), std::io::Error> {
  #[cfg(unix)]
  let result = std::fs::DirBuilder::new().mode(0o777).create(path);
  #[cfg(not(unix))]
  let result = std::fs::create_dir(path);

  result
}
