//! Trace ID injection tests for Ferron.
//!
//! These tests verify that the `trace_id_header` directive correctly injects
//! the request's trace ID into response headers, with support for custom
//! header names and on-demand reflection.
//!
//! See: modules/http-traceid/

use std::io::Write;

use testcontainers::core::ContainerPort;

use crate::common;

/// Test that trace_id_header injects the default header.
///
/// Verifies that when `trace_id_header true` is configured, the response
/// includes an `x-ferron-trace-id` header with a non-empty trace ID value.
#[tokio::test]
async fn test_trace_id_header_default() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let mut config_file = common::create_temp_file();

    common::write_file(webroot_dir.path().join("test.txt"), b"hello").unwrap();

    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
    root "/var/www/ferron"
    trace_id_header true
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

    let response = client
        .get(format!("http://localhost:{}/test.txt", port))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);

    let trace_id = response
        .headers()
        .get("x-ferron-trace-id")
        .and_then(|v| v.to_str().ok());

    assert!(
        trace_id.is_some(),
        "Expected x-ferron-trace-id header to be present"
    );

    let trace_id = trace_id.unwrap();
    assert!(
        !trace_id.is_empty(),
        "Expected non-empty trace ID, got: {:?}",
        trace_id
    );

    container.stop().await.unwrap();
}

/// Test that trace_id_header works with a custom header name.
///
/// Verifies that `header_name` subdirective changes the injected header name.
#[tokio::test]
async fn test_trace_id_custom_header_name() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let mut config_file = common::create_temp_file();

    common::write_file(webroot_dir.path().join("test.txt"), b"hello").unwrap();

    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
    root "/var/www/ferron"
    trace_id_header true {
        header_name "X-Request-ID"
    }
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

    let response = client
        .get(format!("http://localhost:{}/test.txt", port))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);

    // Default header should NOT be present
    assert!(
        response.headers().get("x-ferron-trace-id").is_none(),
        "Default trace ID header should not be present"
    );

    // Custom header should be present
    let trace_id = response
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok());

    assert!(
        trace_id.is_some(),
        "Expected X-Request-ID header to be present"
    );

    let trace_id = trace_id.unwrap();
    assert!(
        !trace_id.is_empty(),
        "Expected non-empty trace ID in X-Request-ID, got: {:?}",
        trace_id
    );

    container.stop().await.unwrap();
}

/// Test that trace_id_header with reflect_request only injects when
/// the request includes the reflection trigger header.
///
/// Verifies that when `reflect_request true` is set, the trace ID header
/// is only injected when the client sends `x-ferron-trace-reflect: 1`.
#[tokio::test]
async fn test_trace_id_reflect_request() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let mut config_file = common::create_temp_file();

    common::write_file(webroot_dir.path().join("test.txt"), b"hello").unwrap();

    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
    root "/var/www/ferron"
    trace_id_header true {
        reflect_request true
    }
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

    // Request WITHOUT the reflect header — trace ID should NOT be injected
    let response = client
        .get(format!("http://localhost:{}/test.txt", port))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert!(
        response.headers().get("x-ferron-trace-id").is_none(),
        "Trace ID should not be present without reflect header"
    );

    // Request WITH the reflect header — trace ID SHOULD be injected
    let response = client
        .get(format!("http://localhost:{}/test.txt", port))
        .header("x-ferron-trace-reflect", "1")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);

    let trace_id = response
        .headers()
        .get("x-ferron-trace-id")
        .and_then(|v| v.to_str().ok());

    assert!(
        trace_id.is_some(),
        "Expected x-ferron-trace-id header when reflect header is present"
    );

    let trace_id = trace_id.unwrap();
    assert!(
        !trace_id.is_empty(),
        "Expected non-empty trace ID, got: {:?}",
        trace_id
    );

    container.stop().await.unwrap();
}

/// Test that trace_id_header disabled does not inject the header.
///
/// Verifies that `trace_id_header false` prevents trace ID injection.
#[tokio::test]
async fn test_trace_id_disabled() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let mut config_file = common::create_temp_file();

    common::write_file(webroot_dir.path().join("test.txt"), b"hello").unwrap();

    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
    root "/var/www/ferron"
    trace_id_header false
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

    let response = client
        .get(format!("http://localhost:{}/test.txt", port))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert!(
        response.headers().get("x-ferron-trace-id").is_none(),
        "Trace ID should not be present when disabled"
    );

    container.stop().await.unwrap();
}

/// Test that trace ID is unique per request.
///
/// Verifies that each request gets a different trace ID value.
#[tokio::test]
async fn test_trace_id_unique_per_request() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let mut config_file = common::create_temp_file();

    common::write_file(webroot_dir.path().join("test.txt"), b"hello").unwrap();

    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
    root "/var/www/ferron"
    trace_id_header true
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

    let mut trace_ids = Vec::new();

    for _ in 0..5 {
        let response = client
            .get(format!("http://localhost:{}/test.txt", port))
            .send()
            .await
            .unwrap();

        let trace_id = response
            .headers()
            .get("x-ferron-trace-id")
            .and_then(|v| v.to_str().ok())
            .unwrap()
            .to_string();

        trace_ids.push(trace_id);
    }

    // All trace IDs should be unique
    let unique_count = trace_ids
        .iter()
        .collect::<std::collections::HashSet<_>>()
        .len();
    assert_eq!(
        unique_count,
        trace_ids.len(),
        "Expected all trace IDs to be unique, got: {:?}",
        trace_ids
    );

    container.stop().await.unwrap();
}
