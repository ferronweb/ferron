//! Map directive tests for Ferron.
//!
//! These tests are inspired by the nginx-tests `map.t` test file, which
//! verifies correct variable mapping based on request properties including
//! exact match, wildcards, regex, and variable chaining.
//!
//! See: https://github.com/nginx/nginx-tests/blob/master/map.t

use std::io::Write;

use testcontainers::core::ContainerPort;

use crate::common;

/// Test basic map directive with exact match.
///
/// Inspired by nginx-tests map.t — verifies that map correctly maps
/// source values to target values.
#[tokio::test]
async fn test_map_exact_match() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let mut config_file = common::create_temp_file();

    common::write_file(webroot_dir.path().join("foo.html"), b"foo").unwrap();
    common::write_file(webroot_dir.path().join("baz.html"), b"baz").unwrap();
    common::write_file(webroot_dir.path().join("other.html"), b"other").unwrap();

    config_file
        .as_file_mut()
        .write_all(
            r#"
{
    map request.uri category {
        default "default"
        exact "/foo.html" "bar"
        exact "/baz.html" "qux"
    }
}

*:80 {
    root "/var/www/ferron"
    header +X-Map-Value "{{category}}"
}
"#
            .as_bytes(),
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

    // Test exact match for /foo.html
    let response = client
        .get(format!("http://localhost:{}/foo.html", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.headers().get("X-Map-Value").unwrap(), "bar");

    // Test exact match for /baz.html
    let response = client
        .get(format!("http://localhost:{}/baz.html", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.headers().get("X-Map-Value").unwrap(), "qux");

    // Test default
    let response = client
        .get(format!("http://localhost:{}/other.html", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.headers().get("X-Map-Value").unwrap(), "default");

    container.stop().await.unwrap();
}

/// Test map directive with regex match.
///
/// Inspired by nginx-tests map.t — verifies regex-based variable mapping.
#[tokio::test]
async fn test_map_regex_match() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let mut config_file = common::create_temp_file();

    common::write_file(webroot_dir.path().join("users.html"), b"users").unwrap();
    common::write_file(webroot_dir.path().join("other.html"), b"other").unwrap();

    config_file
        .as_file_mut()
        .write_all(
            r#"
{
    map request.uri user_id {
        default ""
        regex "^/users/([0-9]+)\\.html" "$1"
    }
}

*:80 {
    root "/var/www/ferron"
    header +X-User-ID "{{user_id}}"
}
"#
            .as_bytes(),
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

    // Test regex match with capture group — even 404 should have the header
    let response = client
        .get(format!("http://localhost:{}/users/42.html", port))
        .send()
        .await
        .unwrap();
    // File doesn't exist, but map should still be evaluated
    assert_eq!(response.headers().get("X-User-ID").unwrap(), "42");

    // Test no match
    let response = client
        .get(format!("http://localhost:{}/other.html", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response.headers().get("X-User-ID").unwrap(), "");

    container.stop().await.unwrap();
}

/// Test map directive with header-based matching.
///
/// Inspired by nginx-tests map.t — verifies header value mapping.
#[tokio::test]
async fn test_map_header_match() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let mut config_file = common::create_temp_file();

    common::write_file(webroot_dir.path().join("test.html"), b"test").unwrap();

    config_file
        .as_file_mut()
        .write_all(
            r#"
{
    map request.header.user_agent is_scanner {
        default "0"
        regex "(?i)^scanner" "1"
    }
}

*:80 {
    root "/var/www/ferron"
    header +X-Is-Scanner "{{is_scanner}}"
}
"#
            .as_bytes(),
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

    // Scanner user agent should match
    let response = client
        .get(format!("http://localhost:{}/test.html", port))
        .header("User-Agent", "scanner/1.0")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.headers().get("X-Is-Scanner").unwrap(), "1");

    // Normal user agent should not match
    let response = client
        .get(format!("http://localhost:{}/test.html", port))
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.headers().get("X-Is-Scanner").unwrap(), "0");

    container.stop().await.unwrap();
}
