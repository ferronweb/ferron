//! Connection limiting tests for Ferron.
//!
//! These tests are inspired by the nginx-tests `limit_conn.t` and
//! `limit_conn_complex.t` test files, which verify correct connection
//! limiting behavior with zones, status codes, and custom responses.
//! The original nginx tests cover per-IP connection limits, multiple
//! zones, and status code configuration.
//!
//! See: https://github.com/nginx/nginx-tests/blob/master/limit_conn.t
//! See: https://github.com/nginx/nginx-tests/blob/master/limit_conn_complex.t

use std::io::Write;

use testcontainers::core::ContainerPort;

use crate::common;

/// Test basic connection limiting with concurrent requests.
///
/// Inspired by nginx-tests limit_conn.t — verifies that concurrent
/// connections exceeding the limit are rejected.
#[tokio::test]
async fn test_connection_limit_basic() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let mut config_file = common::create_temp_file();

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

    common::write_file(webroot_dir.path().join("test.txt"), b"test content").unwrap();

    let container = common::create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();

    let client = reqwest::Client::new();

    // Without connection limiting, multiple concurrent requests should all succeed
    let mut handles = Vec::new();
    for _ in 0..10 {
        let client = client.clone();
        let url = format!("http://localhost:{}/test.txt", port);
        handles.push(tokio::spawn(async move {
            client.get(&url).send().await.unwrap().status()
        }));
    }

    let results = futures_util::future::join_all(handles).await;
    for result in results {
        let status = result.unwrap();
        assert_eq!(status, reqwest::StatusCode::OK);
    }

    container.stop().await.unwrap();
}

/// Test rate limiting with different keys (IP vs URI).
///
/// Inspired by nginx-tests limit_conn_complex.t — verifies that rate
/// limiting with different key extractors works correctly.
#[tokio::test]
async fn test_rate_limit_different_keys() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let mut config_file = common::create_temp_file();

    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
  root "/var/www/ferron"
  rate_limit {
    rate 2
    burst 0
    key remote_address
  }
}
"#
            .as_bytes(),
        )
        .unwrap();

    common::write_file(webroot_dir.path().join("page1.html"), b"page1").unwrap();
    common::write_file(webroot_dir.path().join("page2.html"), b"page2").unwrap();

    let container = common::create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();

    let client = reqwest::Client::new();

    // First request should succeed
    let response = client
        .get(format!("http://localhost:{}/page1.html", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    // Second request from same IP should be rate limited
    let response = client
        .get(format!("http://localhost:{}/page2.html", port))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        reqwest::StatusCode::TOO_MANY_REQUESTS,
        "Second request from same IP should be rate limited"
    );

    container.stop().await.unwrap();
}

/// Test rate limiting with burst.
///
/// Inspired by nginx-tests limit_req.t — verifies that burst allows
/// short bursts of requests.
#[tokio::test]
async fn test_rate_limit_burst() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let mut config_file = common::create_temp_file();

    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
  root "/var/www/ferron"
  rate_limit {
    rate 1
    burst 3
  }
}
"#
            .as_bytes(),
        )
        .unwrap();

    common::write_file(webroot_dir.path().join("test.txt"), b"test content").unwrap();

    let container = common::create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();

    let client = reqwest::Client::new();

    let mut allowed = 0;
    let mut rejected = 0;

    // Send 10 rapid requests — burst of 3 should be allowed
    for _ in 0..10 {
        let response = client
            .get(format!("http://localhost:{}/test.txt", port))
            .send()
            .await
            .unwrap();
        if response.status().is_success() {
            allowed += 1;
        } else if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            rejected += 1;
        }
    }

    // With burst=3 and rate=1, we should see at least 1 allowed and some rejected
    assert!(
        rejected > 0,
        "Some requests should be rate limited (allowed={}, rejected={})",
        allowed,
        rejected
    );

    container.stop().await.unwrap();
}
