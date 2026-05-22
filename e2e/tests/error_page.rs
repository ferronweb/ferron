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
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let ferron_image = self::common::build_ferron_image().await?;
    ferron_image
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
        ))
        .start()
        .await
}

/// error_page serves a custom file for 404 Not Found responses.
#[tokio::test]
async fn test_error_page_custom_404() {
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

    std::fs::create_dir_all(webroot_dir.path().join("custom")).unwrap();
    std::fs::write(
        webroot_dir.path().join("custom").join("404.html"),
        b"custom 404 page",
    )
    .unwrap();

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
            br#"
*:80 {
    root "/var/www/ferron"
    error_page 404 /var/www/ferron/custom/404.html
}
"#,
        )
        .unwrap();
    config_file.flush().unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
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

    // Request a non-existent file — should return custom 404 page
    let response = client
        .get(&format!("{}/nonexistent.html", ferron_addr))
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(response.status(), 404, "Expected 404 Not Found");
    let body = response.text().await.expect("Failed to read body");
    assert!(
        body.contains("custom 404 page"),
        "Response body should contain custom 404 page content, got: {body}"
    );
}

/// A request to an existing file returns normally; error_page does not interfere.
#[tokio::test]
async fn test_error_page_normal_request_unaffected() {
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

    std::fs::create_dir_all(webroot_dir.path().join("custom")).unwrap();
    std::fs::write(
        webroot_dir.path().join("custom").join("404.html"),
        b"custom 404 page",
    )
    .unwrap();
    std::fs::write(webroot_dir.path().join("index.html"), b"hello world").unwrap();

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
            br#"
*:80 {
    root "/var/www/ferron"
    error_page 404 /var/www/ferron/custom/404.html
}
"#,
        )
        .unwrap();
    config_file.flush().unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
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

    // Request an existing file — should return 200 with its content
    let response = client
        .get(&format!("{}/index.html", ferron_addr))
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(response.status(), 200, "Expected 200 OK");
    let body = response.text().await.expect("Failed to read body");
    assert_eq!(body, "hello world", "Response body should be the normal file");
}

/// error_page supports multiple status codes mapped to the same file.
#[tokio::test]
async fn test_error_page_multiple_codes() {
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

    std::fs::create_dir_all(webroot_dir.path().join("custom")).unwrap();
    std::fs::write(
        webroot_dir.path().join("custom").join("50x.html"),
        b"custom server error page",
    )
    .unwrap();

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
            br#"
*:80 {
    root "/var/www/ferron"
    error_page 500 502 503 504 /var/www/ferron/custom/50x.html
}
"#,
        )
        .unwrap();
    config_file.flush().unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
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

    // Request a non-existent file — returns 404, not 50x, so the custom page
    // should NOT be served. This validates that codes are matched specifically.
    let response = client
        .get(&format!("{}/nonexistent.html", ferron_addr))
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(response.status(), 404, "Expected 404 Not Found");
    let body = response.text().await.expect("Failed to read body");
    assert!(
        !body.contains("custom server error page"),
        "404 should not use the 50x error page"
    );
}
