use std::io::Write;

use crate::common;

#[tokio::test]
async fn test_error_page_custom_404() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    std::fs::create_dir_all(webroot_dir.path().join("custom")).unwrap();
    std::fs::write(
        webroot_dir.path().join("custom").join("404.html"),
        b"custom 404 page",
    )
    .unwrap();

    let mut config_file = common::create_temp_file();

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

    let container = common::create_ferron_container(webroot_dir.path(), config_file.path())
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
        .get(format!("{}/nonexistent.html", ferron_addr))
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

#[tokio::test]
async fn test_error_page_normal_request_unaffected() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    std::fs::create_dir_all(webroot_dir.path().join("custom")).unwrap();
    std::fs::write(
        webroot_dir.path().join("custom").join("404.html"),
        b"custom 404 page",
    )
    .unwrap();
    std::fs::write(webroot_dir.path().join("index.html"), b"hello world").unwrap();

    let mut config_file = common::create_temp_file();

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

    let container = common::create_ferron_container(webroot_dir.path(), config_file.path())
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
    let body = response.text().await.expect("Failed to read body");
    assert_eq!(
        body, "hello world",
        "Response body should be the normal file"
    );
}

#[tokio::test]
async fn test_error_page_multiple_codes() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    std::fs::create_dir_all(webroot_dir.path().join("custom")).unwrap();
    std::fs::write(
        webroot_dir.path().join("custom").join("50x.html"),
        b"custom server error page",
    )
    .unwrap();

    let mut config_file = common::create_temp_file();

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

    let container = common::create_ferron_container(webroot_dir.path(), config_file.path())
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
        .get(format!("{}/nonexistent.html", ferron_addr))
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
