//! Inspired by nginx-tests http_error_page.t: https://github.com/nginx/nginx-tests/blob/master/http_error_page.t

use std::io::Write;

use testcontainers::core::ContainerPort;

use crate::common;

#[tokio::test]
async fn test_error_page_cascading() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let mut config_file = common::create_temp_file();

    common::write_file(
        webroot_dir.path().join("fallback.html"),
        b"fallback content",
    )
    .unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
    root "/var/www/ferron"

    handle_error 404 {
        status 302 {
            location "/fallback.html"
        }
    }
}
"#,
        )
        .unwrap();

    let container = common::create_ferron_container(webroot_dir.path(), config_file.path())
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
        .get(format!("http://localhost:{}/nonexistent.html", port))
        .send()
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        reqwest::StatusCode::FOUND,
        "Should redirect to fallback"
    );

    let location = response
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(location, "/fallback.html");

    // Follow the redirect, it should get the fallback content
    let response = client
        .get(format!("http://localhost:{}/fallback.html", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), "fallback content");

    container.stop().await.unwrap();
}
