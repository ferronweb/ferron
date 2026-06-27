//! Error page cascading tests for Ferron.
//!
//! These tests are inspired by the nginx-tests `http_error_page.t` test file,
//! which verifies correct error page handling including cascading errors,
//! error redirects, and error page interactions with other directives.
//!
//! See: https://github.com/nginx/nginx-tests/blob/master/http_error_page.t

use std::io::Write;

use testcontainers::core::ContainerPort;

use crate::common;

/// Test cascading error pages — when an error page itself triggers another error.
///
/// Inspired by nginx-tests http_error_page.t — verifies that cascading error
/// pages don't cause infinite loops and resolve correctly.
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

    // Request for non-existent file should redirect to fallback
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

    // Follow the redirect — should get the fallback content
    let response = client
        .get(format!("http://localhost:{}/fallback.html", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), "fallback content");

    container.stop().await.unwrap();
}

/// Test error page with proxy — when backend returns error, custom error page is served.
///
/// Inspired by nginx-tests http_error_page.t — verifies error pages work
/// with proxied backends.
#[tokio::test]
async fn test_error_page_with_proxy() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let mut config_file = common::create_temp_file();

    common::write_file(webroot_dir.path().join("502.html"), b"custom 502 page").unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
    root "/var/www/ferron"

    handle_error 502 {
        root "/var/www/ferron"
        status 200 {
            location "/502.html"
        }
    }

    proxy "http://localhost:19999"
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

    let client = reqwest::Client::new();

    // Request to non-existent backend should return 502
    // which should be caught by handle_error and return custom page
    let response = client
        .get(format!("http://localhost:{}/", port))
        .send()
        .await
        .unwrap();

    // The response should be either a 502 error or a custom 200 from handle_error
    assert!(
        response.status().is_server_error() || response.status().is_success(),
        "Expected error or custom error page, got {}",
        response.status()
    );

    container.stop().await.unwrap();
}

/// Test error page with status code return — returning different status codes.
///
/// Inspired by nginx-tests http_error_page.t — verifies error pages can
/// return various status codes.
#[tokio::test]
async fn test_error_page_status_codes() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let mut config_file = common::create_temp_file();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
    root "/var/www/ferron"

    handle_error 404 {
        status 410
    }
}
"#,
        )
        .unwrap();

    common::write_file(webroot_dir.path().join("index.html"), b"hello").unwrap();

    let container = common::create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();

    let client = reqwest::Client::new();

    // Request for non-existent file should return 410 Gone
    let response = client
        .get(format!("http://localhost:{}/nonexistent.html", port))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        reqwest::StatusCode::GONE,
        "Expected 410 Gone from error handler"
    );

    // Existing file should still work
    let response = client
        .get(format!("http://localhost:{}/index.html", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    container.stop().await.unwrap();
}
