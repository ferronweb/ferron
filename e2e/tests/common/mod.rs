#![allow(dead_code)]

#[cfg(unix)]
use std::{
    io::Write,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
};
use std::{path::PathBuf, sync::LazyLock};

use testcontainers::{
    GenericBuildableImage, GenericImage, TestcontainersError, core::BuildImageOptions,
    runners::AsyncBuilder,
};
use tokio::sync::Mutex;

static FERRON_IMAGE: std::sync::LazyLock<Mutex<Option<GenericImage>>> =
    LazyLock::new(|| Mutex::new(None));
static BACKEND_IMAGE: std::sync::LazyLock<Mutex<Option<GenericImage>>> =
    LazyLock::new(|| Mutex::new(None));
static BACKEND_GRPC_IMAGE: std::sync::LazyLock<Mutex<Option<GenericImage>>> =
    LazyLock::new(|| Mutex::new(None));
static CGI_IMAGE: std::sync::LazyLock<Mutex<Option<GenericImage>>> =
    LazyLock::new(|| Mutex::new(None));
static FCGIWRAP_IMAGE: std::sync::LazyLock<Mutex<Option<GenericImage>>> =
    LazyLock::new(|| Mutex::new(None));
static OTLP_IMAGE: std::sync::LazyLock<Mutex<Option<GenericImage>>> =
    LazyLock::new(|| Mutex::new(None));
static OCSP_IMAGE: std::sync::LazyLock<Mutex<Option<GenericImage>>> =
    LazyLock::new(|| Mutex::new(None));
static BIND9_IMAGE: std::sync::LazyLock<Mutex<Option<GenericImage>>> =
    LazyLock::new(|| Mutex::new(None));
static AUTH_BACKEND_IMAGE: std::sync::LazyLock<Mutex<Option<GenericImage>>> =
    LazyLock::new(|| Mutex::new(None));
static LSCACHE_BACKEND_IMAGE: std::sync::LazyLock<Mutex<Option<GenericImage>>> =
    LazyLock::new(|| Mutex::new(None));

pub async fn build_ferron_image() -> Result<GenericImage, TestcontainersError> {
    let mut ferron_image = FERRON_IMAGE.lock().await;
    if let Some(image) = ferron_image.as_ref() {
        return Ok(image.clone());
    }
    let mut builder = GenericBuildableImage::new("e2e-test-ferron", "latest")
        .with_dockerfile(concat!(env!("CARGO_MANIFEST_DIR"), "/Dockerfile.test"));
    for entry in glob::glob(concat!(env!("CARGO_MANIFEST_DIR"), "/../*")).unwrap() {
        let entry = entry.unwrap();
        let dest = entry.file_name().unwrap().to_str().unwrap().to_string();
        if dest != "target"
            && dest != "e2e"
            && dest != ".git"
            && dest != "Dockerfile"
            && !dest.starts_with("Dockerfile.")
        {
            builder = builder.with_file(entry, format!("./{dest}"));
        }
    }
    let ferron_image_built = builder
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
    let backend_image_built = GenericBuildableImage::new("e2e-test-backend-v2", "latest")
        .with_dockerfile(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/images/backend/Dockerfile"
        ))
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
        .with_dockerfile(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/images/backend-grpc/Dockerfile"
        ))
        .with_file(
            concat!(env!("CARGO_MANIFEST_DIR"), "/images/backend-grpc"),
            ".",
        )
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
        .with_dockerfile(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/images/cgi/Dockerfile"
        ))
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
        .with_dockerfile(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/images/fcgiwrap/Dockerfile"
        ))
        .with_file(concat!(env!("CARGO_MANIFEST_DIR"), "/images/fcgiwrap"), ".")
        .build_image()
        .await?;
    fcgiwrap_image.replace(fcgiwrap_image_built.clone());
    Ok(fcgiwrap_image_built)
}

pub async fn build_otlp_image() -> Result<GenericImage, TestcontainersError> {
    let mut otlp_image = OTLP_IMAGE.lock().await;
    if let Some(image) = otlp_image.as_ref() {
        return Ok(image.clone());
    }
    let otlp_image_built = GenericBuildableImage::new("e2e-test-otlp", "latest")
        .with_dockerfile(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/images/otlp/Dockerfile"
        ))
        .with_file(concat!(env!("CARGO_MANIFEST_DIR"), "/images/otlp"), ".")
        .build_image()
        .await?;
    otlp_image.replace(otlp_image_built.clone());
    Ok(otlp_image_built)
}

pub async fn build_ocsp_image() -> Result<GenericImage, TestcontainersError> {
    let mut ocsp_image = OCSP_IMAGE.lock().await;
    if let Some(image) = ocsp_image.as_ref() {
        return Ok(image.clone());
    }
    let image = GenericBuildableImage::new("e2e-test-ocsp", "latest")
        .with_dockerfile(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/images/ocsp/Dockerfile"
        ))
        .with_file(concat!(env!("CARGO_MANIFEST_DIR"), "/images/ocsp"), ".")
        .build_image()
        .await?;
    ocsp_image.replace(image.clone());
    Ok(image)
}

pub async fn build_bind9_image() -> Result<GenericImage, TestcontainersError> {
    let mut bind9_image = BIND9_IMAGE.lock().await;
    if let Some(image) = bind9_image.as_ref() {
        return Ok(image.clone());
    }
    let bind9_image_built = GenericBuildableImage::new("e2e-test-bind9", "latest")
        .with_dockerfile(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/images/bind9/Dockerfile"
        ))
        .with_file(
            concat!(env!("CARGO_MANIFEST_DIR"), "/images/bind9/entrypoint.sh"),
            "/entrypoint.sh",
        )
        .build_image()
        .await?;
    bind9_image.replace(bind9_image_built.clone());
    Ok(bind9_image_built)
}

pub async fn build_auth_backend_image() -> Result<GenericImage, TestcontainersError> {
    let mut auth_backend_image = AUTH_BACKEND_IMAGE.lock().await;
    if let Some(image) = auth_backend_image.as_ref() {
        return Ok(image.clone());
    }
    let auth_backend_image_built = GenericBuildableImage::new("e2e-test-auth-backend", "latest")
        .with_dockerfile(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/images/auth-backend/Dockerfile"
        ))
        .with_file(
            concat!(env!("CARGO_MANIFEST_DIR"), "/images/auth-backend"),
            ".",
        )
        .build_image()
        .await?;
    auth_backend_image.replace(auth_backend_image_built.clone());
    Ok(auth_backend_image_built)
}

pub async fn build_lscache_backend_image() -> Result<GenericImage, TestcontainersError> {
    let mut lscache_backend_image = LSCACHE_BACKEND_IMAGE.lock().await;
    if let Some(image) = lscache_backend_image.as_ref() {
        return Ok(image.clone());
    }
    let lscache_backend_image_built =
        GenericBuildableImage::new("e2e-test-lscache-backend", "latest")
            .with_dockerfile(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/images/lscache-backend/Dockerfile"
            ))
            .with_file(
                concat!(env!("CARGO_MANIFEST_DIR"), "/images/lscache-backend"),
                ".",
            )
            .build_image()
            .await?;
    lscache_backend_image.replace(lscache_backend_image_built.clone());
    Ok(lscache_backend_image_built)
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
