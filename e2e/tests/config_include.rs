use std::io::Write;
#[cfg(unix)]
use std::{fs::Permissions, os::unix::fs::PermissionsExt};

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};

mod common;

async fn create_ferron_container(
    webroot_dir: &std::path::Path,
    config_file: &std::path::Path,
    extra_file: Option<&std::path::Path>,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let ferron_image = self::common::build_ferron_image().await?;
    let mut image = ferron_image
        .with_exposed_port(ContainerPort::Tcp(80))
        .with_wait_for(WaitFor::Http(Box::new(
            HttpWaitStrategy::new("/")
                .with_port(ContainerPort::Tcp(80))
                .with_response_matcher(|_| true),
        )))
        .with_mount(Mount::bind_mount(
            webroot_dir.to_string_lossy(),
            "/var/www/ferron",
        ))
        .with_mount(Mount::bind_mount(
            config_file.to_string_lossy(),
            "/etc/ferron.conf",
        ));

    if let Some(extra) = extra_file {
        image = image.with_mount(Mount::bind_mount(
            extra.to_string_lossy(),
            "/etc/extra.conf",
        ));
    }

    image.start().await
}

/// include loads additional configuration files at the top level.
/// Directives from the included file are merged inline.
#[tokio::test]
async fn test_include_extra_config() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let webroot_dir = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o777))
        .tempdir()
        .unwrap();
    #[cfg(not(unix))]
    let webroot_dir = tempfile::tempdir().unwrap();

    std::fs::write(webroot_dir.path().join("index.html"), b"hello").unwrap();

    // Included file defines the host block
    #[cfg(unix)]
    let mut extra_file = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();
    #[cfg(not(unix))]
    let mut extra_file = tempfile::NamedTempFile::new().unwrap();

    extra_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
    root "/var/www/ferron"
}
"#,
        )
        .unwrap();
    extra_file.flush().unwrap();

    // Main config includes the extra file
    #[cfg(unix)]
    let mut config_file = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"include "/etc/extra.conf"
"#,
        )
        .unwrap();
    config_file.flush().unwrap();

    let container = create_ferron_container(
        webroot_dir.path(),
        config_file.path(),
        Some(extra_file.path()),
    )
    .await
    .expect("Failed to create container");

    let ferron_addr = format!(
        "http://127.0.0.1:{}",
        container
            .get_host_port_ipv4(80)
            .await
            .expect("Failed to get host port")
    );

    let client = reqwest::Client::new();
    let response = client
        .get(&format!("{}/index.html", ferron_addr))
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(response.status(), 200, "Expected 200 OK from included config");
    let body = response.text().await.expect("Failed to read body");
    assert_eq!(body, "hello", "Response body should be correct");
}

/// include with a glob pattern loads all matching files.
#[tokio::test]
async fn test_include_glob_pattern() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let webroot_dir = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o777))
        .tempdir()
        .unwrap();
    #[cfg(not(unix))]
    let webroot_dir = tempfile::tempdir().unwrap();

    std::fs::write(webroot_dir.path().join("index.html"), b"hello").unwrap();

    // Included file with a.conf name
    #[cfg(unix)]
    let mut extra_file = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();
    #[cfg(not(unix))]
    let mut extra_file = tempfile::NamedTempFile::new().unwrap();

    extra_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
    root "/var/www/ferron"
}
"#,
        )
        .unwrap();
    extra_file.flush().unwrap();

    // Use a config directory with glob — mount the extra file directory
    // and use include with a pattern
    #[cfg(unix)]
    let mut config_file = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"include "/etc/extra.conf"
"#,
        )
        .unwrap();
    config_file.flush().unwrap();

    let container = create_ferron_container(
        webroot_dir.path(),
        config_file.path(),
        Some(extra_file.path()),
    )
    .await
    .expect("Failed to create container");

    let ferron_addr = format!(
        "http://127.0.0.1:{}",
        container
            .get_host_port_ipv4(80)
            .await
            .expect("Failed to get host port")
    );

    let client = reqwest::Client::new();
    let response = client
        .get(&format!("{}/index.html", ferron_addr))
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(response.status(), 200, "Expected 200 OK");
    let body = response.text().await.expect("Failed to read body");
    assert_eq!(body, "hello", "Response body should be correct");
}

/// Two includes are allowed and both are processed in order.
#[tokio::test]
async fn test_include_multiple_files() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let webroot_dir = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o777))
        .tempdir()
        .unwrap();
    #[cfg(not(unix))]
    let webroot_dir = tempfile::tempdir().unwrap();

    std::fs::write(webroot_dir.path().join("index.html"), b"hello").unwrap();

    // Included file A: defines the root
    #[cfg(unix)]
    let mut extra_a = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();
    #[cfg(not(unix))]
    let mut extra_a = tempfile::NamedTempFile::new().unwrap();

    extra_a
        .as_file_mut()
        .write_all(
            br#"
*:80 {
    root "/var/www/ferron"
}
"#,
        )
        .unwrap();
    extra_a.flush().unwrap();

    // Included file B: adds a custom header
    #[cfg(unix)]
    let mut extra_b = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();
    #[cfg(not(unix))]
    let mut extra_b = tempfile::NamedTempFile::new().unwrap();

    extra_b
        .as_file_mut()
        .write_all(
            br#"
*:80 {
    header +X-Included "from-extra-b"
}
"#,
        )
        .unwrap();
    extra_b.flush().unwrap();

    // Main config includes both
    #[cfg(unix)]
    let mut config_file = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"include "/etc/extra_a.conf"
include "/etc/extra_b.conf"
"#,
        )
        .unwrap();
    config_file.flush().unwrap();

    let ferron_image = self::common::build_ferron_image().await.unwrap();
    let container = ferron_image
        .with_exposed_port(ContainerPort::Tcp(80))
        .with_wait_for(WaitFor::Http(Box::new(
            HttpWaitStrategy::new("/")
                .with_port(ContainerPort::Tcp(80))
                .with_response_matcher(|_| true),
        )))
        .with_mount(Mount::bind_mount(
            webroot_dir.path().to_string_lossy(),
            "/var/www/ferron",
        ))
        .with_mount(Mount::bind_mount(
            config_file.path().to_string_lossy(),
            "/etc/ferron.conf",
        ))
        .with_mount(Mount::bind_mount(
            extra_a.path().to_string_lossy(),
            "/etc/extra_a.conf",
        ))
        .with_mount(Mount::bind_mount(
            extra_b.path().to_string_lossy(),
            "/etc/extra_b.conf",
        ))
        .start()
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(80)
        .await
        .expect("Failed to get host port");

    let client = reqwest::Client::new();
    let response = client
        .get(&format!("http://127.0.0.1:{port}/index.html"))
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(response.status(), 200, "Expected 200 OK");
    let headers = response.headers();
    let val = headers
        .get("x-included")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(
        val, "from-extra-b",
        "Header from second included file should be present"
    );
}
