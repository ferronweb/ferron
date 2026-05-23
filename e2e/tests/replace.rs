use std::io::Write;
use testcontainers::core::ContainerPort;

mod common;

#[tokio::test]
async fn test_string_replacement() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let mut config_file = common::create_temp_file();

    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
  root "/var/www/ferron"
  replace "World" "Ferron"
  replace_filter_types "*"
}
"#
            .as_bytes(),
        )
        .unwrap();

    common::write_file(webroot_dir.path().join("test.txt"), b"Hello, World!").unwrap();

    let container = common::create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::new();

    let response = client
        .get(format!("http://localhost:{}/test.txt", port))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), "Hello, Ferron!");

    container.stop().await.unwrap();
}
