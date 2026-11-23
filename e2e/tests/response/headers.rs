use std::io::Write;

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};

async fn create_ferron_container(
    webroot_dir: &std::path::Path,
    config_file: &std::path::Path,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let ferron_image = crate::common::build_ferron_image().await?;
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

#[tokio::test]
async fn test_header_add_custom() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = crate::common::create_temp_dir();

    std::fs::write(webroot_dir.path().join("index.html"), b"hello").unwrap();

    let mut config_file = crate::common::create_temp_file();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
    header +X-Test "hello"
    root "/var/www/ferron"
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
    let response = client
        .get(format!("{}/index.html", ferron_addr))
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(response.status(), 200, "Expected 200 OK");

    let headers = response.headers();
    let test_val = headers
        .get("x-test")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(
        test_val, "hello",
        "Expected X-Test: hello in response headers"
    );
}

#[tokio::test]
async fn test_header_add_then_remove() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = crate::common::create_temp_dir();

    std::fs::write(webroot_dir.path().join("index.html"), b"hello").unwrap();

    let mut config_file = crate::common::create_temp_file();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
    header +X-Temp "temp-value"
    header -X-Temp
    root "/var/www/ferron"
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
    let response = client
        .get(format!("{}/index.html", ferron_addr))
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(response.status(), 200, "Expected 200 OK");

    // X-Temp should be removed by the subsequent -X-Temp directive
    assert!(
        !response.headers().contains_key("x-temp"),
        "X-Temp header should have been removed by header -X-Temp"
    );
}

#[tokio::test]
async fn test_header_replace() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = crate::common::create_temp_dir();

    std::fs::write(webroot_dir.path().join("index.html"), b"hello").unwrap();

    let mut config_file = crate::common::create_temp_file();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
    header X-Powered-By "Ferron Test"
    root "/var/www/ferron"
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
    let response = client
        .get(format!("{}/index.html", ferron_addr))
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(response.status(), 200, "Expected 200 OK");

    let headers = response.headers();
    let powered_by = headers
        .get("x-powered-by")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(
        powered_by, "Ferron Test",
        "Expected X-Powered-By: Ferron Test in response headers"
    );
}
