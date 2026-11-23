#[cfg(unix)]
use std::{io::Write, path::Path};

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};

async fn create_ferron_container(
    webroot_dir: &Path,
    config_file: &Path,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let ferron_image = crate::common::build_ferron_image().await?;
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
        .start()
        .await
}

#[tokio::test]
async fn test_redirecting() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = crate::common::create_temp_dir();
    let mut config_file = crate::common::create_temp_file();

    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
  root "/var/www/ferron"
  status 301 {
    url "/"
    location "/basic.txt"
  }
}
"#
            .as_bytes(),
        )
        .unwrap();

    crate::common::write_file(webroot_dir.path().join("basic.txt"), b"test content").unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    let response = client
        .get(format!("http://localhost:{}/", port))
        .send()
        .await
        .unwrap();

    assert!(
        matches!(response.status(), reqwest::StatusCode::MOVED_PERMANENTLY),
        "expected 301 Moved Permanently, got {}",
        response.status()
    );

    container.stop().await.unwrap();
}

#[tokio::test]
async fn test_trailing_slash_redirect_disabled() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = crate::common::create_temp_dir();
    let mut config_file = crate::common::create_temp_file();

    crate::common::create_dir(webroot_dir.path().join("subdir")).unwrap();
    crate::common::write_file(
        webroot_dir.path().join("subdir/index.html"),
        b"index file content",
    )
    .unwrap();

    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
    root "/var/www/ferron"
    trailing_slash_redirect false
}
"#
            .as_bytes(),
        )
        .unwrap();
    config_file.flush().unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .expect("Failed to create container");

    let port = container
        .get_host_port_ipv4(80)
        .await
        .expect("Failed to get host port");

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let response = client
        .get(format!("http://localhost:{}/subdir", port))
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();

    assert!(
        status != 301 && status != 308 && status != 302,
        "trailing_slash_redirect false should prevent redirect, got status: {}",
        status
    );

    assert!(
        status == 200 || status == 403,
        "Expected 200 (directory listing/index) or 403 (if disabled), got: {}",
        status
    );
}

#[tokio::test]
async fn test_trailing_slash_redirect_noindex() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = crate::common::create_temp_dir();
    let mut config_file = crate::common::create_temp_file();

    crate::common::create_dir(webroot_dir.path().join("subdir")).unwrap();
    crate::common::write_file(
        webroot_dir.path().join("subdir/index.html"),
        b"index file content",
    )
    .unwrap();

    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
    root "/var/www/ferron"
}
"#
            .as_bytes(),
        )
        .unwrap();
    config_file.flush().unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .expect("Failed to create container");

    let port = container
        .get_host_port_ipv4(80)
        .await
        .expect("Failed to get host port");

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let response = client
        .get(format!("http://localhost:{}/subdir", port))
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();

    assert!(
        status == 301 || status == 308 || status == 302 || status == 307,
        "Expected a redirect (301/308/302/307), got status: {}",
        status
    );
}

#[tokio::test]
async fn test_trailing_slash_redirect_index() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = crate::common::create_temp_dir();
    let mut config_file = crate::common::create_temp_file();

    crate::common::create_dir(webroot_dir.path().join("subdir")).unwrap();

    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
    root "/var/www/ferron"
}
"#
            .as_bytes(),
        )
        .unwrap();
    config_file.flush().unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .expect("Failed to create container");

    let port = container
        .get_host_port_ipv4(80)
        .await
        .expect("Failed to get host port");

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let response = client
        .get(format!("http://localhost:{}/subdir", port))
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();

    assert!(
        status == 301 || status == 308 || status == 302 || status == 307,
        "Expected a redirect (301/308/302/307), got status: {}",
        status,
    );
}
