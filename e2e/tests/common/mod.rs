#![allow(dead_code)]

#[cfg(unix)]
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::LazyLock,
};

use testcontainers::{
    ContainerAsync, GenericBuildableImage, GenericImage, ImageExt, TestcontainersError,
    core::{BuildImageOptions, ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::{AsyncBuilder, AsyncRunner},
};
use tokio::sync::Mutex;

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};

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
static STATSD_IMAGE: std::sync::LazyLock<Mutex<Option<GenericImage>>> =
    LazyLock::new(|| Mutex::new(None));
static OCSP_IMAGE: std::sync::LazyLock<Mutex<Option<GenericImage>>> =
    LazyLock::new(|| Mutex::new(None));
static BIND9_IMAGE: std::sync::LazyLock<Mutex<Option<GenericImage>>> =
    LazyLock::new(|| Mutex::new(None));
static AUTH_BACKEND_IMAGE: std::sync::LazyLock<Mutex<Option<GenericImage>>> =
    LazyLock::new(|| Mutex::new(None));
static LSCACHE_BACKEND_IMAGE: std::sync::LazyLock<Mutex<Option<GenericImage>>> =
    LazyLock::new(|| Mutex::new(None));
static MOCK_CONTROL_PLANE_IMAGE: std::sync::LazyLock<Mutex<Option<GenericImage>>> =
    LazyLock::new(|| Mutex::new(None));

pub async fn create_ferron_container(
    webroot_dir: &Path,
    config_file: &Path,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let ferron_image = build_ferron_image().await?;
    ferron_image
        .with_exposed_port(ContainerPort::Tcp(80))
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
        .with_env_var("FERRON_ROOT", "/var/www/ferron") // For some tests...
        .start()
        .await
}

#[cfg(unix)]
pub fn create_temp_dir() -> tempfile::TempDir {
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());
    tempfile::Builder::new()
        .permissions(std::os::unix::fs::PermissionsExt::from_mode(0o777))
        .tempdir()
        .unwrap()
}

#[cfg(not(unix))]
pub fn create_temp_dir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

#[cfg(unix)]
pub fn create_temp_file() -> tempfile::NamedTempFile {
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());
    tempfile::Builder::new()
        .permissions(std::os::unix::fs::PermissionsExt::from_mode(0o666))
        .tempfile()
        .unwrap()
}

#[cfg(not(unix))]
pub fn create_temp_file() -> tempfile::NamedTempFile {
    tempfile::NamedTempFile::new().unwrap()
}

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
            && dest != "fuzz"
            && dest != "e2e"
            && dest != ".git"
            && dest != "Dockerfile"
            && dest != "cross-build"
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

async fn build_cached_image(
    cache: &std::sync::LazyLock<Mutex<Option<GenericImage>>>,
    image_name: &str,
    dockerfile: &str,
    files: &[(&str, &str)],
) -> Result<GenericImage, TestcontainersError> {
    let mut cached = cache.lock().await;
    if let Some(image) = cached.as_ref() {
        return Ok(image.clone());
    }
    let mut builder = GenericBuildableImage::new(image_name, "latest")
        .with_dockerfile(format!("{}{dockerfile}", env!("CARGO_MANIFEST_DIR")));
    for (local, dest) in files {
        builder = builder.with_file(format!("{}{local}", env!("CARGO_MANIFEST_DIR")), *dest);
    }
    let built = builder.build_image().await?;
    cached.replace(built.clone());
    Ok(built)
}

pub async fn build_backend_image() -> Result<GenericImage, TestcontainersError> {
    build_cached_image(
        &BACKEND_IMAGE,
        "e2e-test-backend-v2",
        "/images/backend/Dockerfile",
        &[("/images/backend", ".")],
    )
    .await
}

pub async fn build_backend_grpc_image() -> Result<GenericImage, TestcontainersError> {
    build_cached_image(
        &BACKEND_GRPC_IMAGE,
        "e2e-test-backend-grpc",
        "/images/backend-grpc/Dockerfile",
        &[("/images/backend-grpc", ".")],
    )
    .await
}

pub async fn build_cgi_image() -> Result<GenericImage, TestcontainersError> {
    build_cached_image(
        &CGI_IMAGE,
        "e2e-test-cgi",
        "/images/cgi/Dockerfile",
        &[("/images/cgi", ".")],
    )
    .await
}

pub async fn build_fcgiwrap_image() -> Result<GenericImage, TestcontainersError> {
    build_cached_image(
        &FCGIWRAP_IMAGE,
        "e2e-test-fcgiwrap",
        "/images/fcgiwrap/Dockerfile",
        &[("/images/fcgiwrap", ".")],
    )
    .await
}

pub async fn build_otlp_image() -> Result<GenericImage, TestcontainersError> {
    build_cached_image(
        &OTLP_IMAGE,
        "e2e-test-otlp",
        "/images/otlp/Dockerfile",
        &[("/images/otlp", ".")],
    )
    .await
}

pub async fn build_statsd_image() -> Result<GenericImage, TestcontainersError> {
    build_cached_image(
        &STATSD_IMAGE,
        "e2e-test-statsd",
        "/images/statsd/Dockerfile",
        &[("/images/statsd", ".")],
    )
    .await
}

pub async fn build_ocsp_image() -> Result<GenericImage, TestcontainersError> {
    build_cached_image(
        &OCSP_IMAGE,
        "e2e-test-ocsp",
        "/images/ocsp/Dockerfile",
        &[("/images/ocsp", ".")],
    )
    .await
}

pub async fn build_bind9_image() -> Result<GenericImage, TestcontainersError> {
    build_cached_image(
        &BIND9_IMAGE,
        "e2e-test-bind9",
        "/images/bind9/Dockerfile",
        &[("/images/bind9/entrypoint.sh", "/entrypoint.sh")],
    )
    .await
}

pub async fn build_auth_backend_image() -> Result<GenericImage, TestcontainersError> {
    build_cached_image(
        &AUTH_BACKEND_IMAGE,
        "e2e-test-auth-backend",
        "/images/auth-backend/Dockerfile",
        &[("/images/auth-backend", ".")],
    )
    .await
}

pub async fn build_lscache_backend_image() -> Result<GenericImage, TestcontainersError> {
    build_cached_image(
        &LSCACHE_BACKEND_IMAGE,
        "e2e-test-lscache-backend",
        "/images/lscache-backend/Dockerfile",
        &[("/images/lscache-backend", ".")],
    )
    .await
}

pub async fn build_mock_control_plane_image() -> Result<GenericImage, TestcontainersError> {
    build_cached_image(
        &MOCK_CONTROL_PLANE_IMAGE,
        "e2e-test-mock-control-plane",
        "/images/mock-control-plane/Dockerfile",
        &[("/images/mock-control-plane", ".")],
    )
    .await
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
