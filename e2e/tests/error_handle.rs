mod common;

/// handle_error 404 with a redirect sends a 302 to the fallback page.
#[tokio::test]
async fn test_handle_error_redirect_on_404() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    std::fs::write(webroot_dir.path().join("basic.txt"), b"fallback page").unwrap();

    let config_file = common::create_temp_file();
    std::fs::write(
        config_file.path(),
        br#"
*:80 {
    root "/var/www/ferron"

    handle_error 404 {
        status 302 {
            location "/basic.txt"
        }
    }
}
"#,
    )
    .unwrap();

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

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    // Request a non-existent file — should redirect to /basic.txt
    let response = client
        .get(format!("{}/nonexistent.html", ferron_addr))
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(response.status(), 302, "Expected 302 redirect");
    let location = response
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(
        location, "/basic.txt",
        "Should redirect to /basic.txt, got: {location}"
    );
}

/// handle_error without a specific code catches all errors.
#[tokio::test]
async fn test_handle_error_catch_all() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    std::fs::write(webroot_dir.path().join("basic.txt"), b"catch-all fallback").unwrap();

    let config_file = common::create_temp_file();
    std::fs::write(
        config_file.path(),
        br#"
*:80 {
    root "/var/www/ferron"

    handle_error {
        status 302 {
            location "/basic.txt"
        }
    }
}
"#,
    )
    .unwrap();

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

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    // Request a non-existent file — should redirect to /basic.txt
    let response = client
        .get(format!("{}/nonexistent.html", ferron_addr))
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(
        response.status(),
        302,
        "Expected 302 redirect from catch-all"
    );
    let location = response
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(
        location, "/basic.txt",
        "Should redirect to /basic.txt, got: {location}"
    );
}

/// Existing files are served normally even with handle_error configured.
#[tokio::test]
async fn test_handle_error_normal_requests_unaffected() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    std::fs::write(webroot_dir.path().join("index.html"), b"hello world").unwrap();
    std::fs::write(webroot_dir.path().join("basic.txt"), b"fallback").unwrap();

    let config_file = common::create_temp_file();
    std::fs::write(
        config_file.path(),
        br#"
*:80 {
    root "/var/www/ferron"

    handle_error 404 {
        status 302 {
            location "/basic.txt"
        }
    }
}
"#,
    )
    .unwrap();

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
        "Normal requests should return the file"
    );
}
