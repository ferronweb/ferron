use std::io::Write;

use testcontainers::core::ContainerPort;

use crate::common;

#[tokio::test]
async fn test_routing_bypass() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let mut config_file = common::create_temp_file();

    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
    root "/var/www/ferron"
    location /private {
        status 403
    }
}
"#
            .as_bytes(),
        )
        .unwrap();

    common::write_file(webroot_dir.path().join("test.txt"), b"test content").unwrap();
    common::create_dir(webroot_dir.path().join("private")).unwrap();
    common::write_file(
        webroot_dir.path().join("private").join("test.txt"),
        b"private content",
    )
    .unwrap();

    let container = common::create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();

    let client = reqwest::Client::new();

    // Smoke test
    let response = client
        .get(format!("http://localhost:{}/test.txt", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    // Normal case
    let response = client
        .get(format!("http://localhost:{}/private/test.txt", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);

    // Potential bypass edge cases
    let response = client
        .get(format!("http://localhost:{}/private%2Ftest.txt", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
    let response = client
        .get(format!("http://localhost:{}/private%2F..%2Ftest.txt", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);

    container.stop().await.unwrap();
}
