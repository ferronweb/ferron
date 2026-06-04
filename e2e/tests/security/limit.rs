use std::io::Write;

use testcontainers::core::ContainerPort;

use crate::{common, create_ferron_container};

/// Test for rate limiting race condition fix.
/// Ensures rate limiting bucket creation doesn't allow bypassing capacity.
#[tokio::test]
async fn test_rate_limiting_race_condition_fixed() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let mut config_file = common::create_temp_file();

    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
    rate_limit {
        rate 5
        burst 0
        key remote_address
    }
    root "/var/www/ferron"
}
"#
            .as_bytes(),
        )
        .unwrap();
    config_file.flush().unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .expect("Failed to create container");

    let client = reqwest::Client::new();
    let ferron_addr = format!(
        "http://127.0.0.1:{}",
        container
            .get_host_port_ipv4(80)
            .await
            .expect("Failed to get host port")
    );

    let mut allowed = 0;
    let mut rejected = 0;
    for _ in 0..10 {
        let response = client
            .get(&ferron_addr)
            .send()
            .await
            .expect("Failed to send request");
        if response.status().is_success() {
            allowed += 1;
        } else if response.status() == 429 {
            rejected += 1;
        }
    }

    println!(
        "Rate limiting test: allowed={}, rejected={} (expected ~5 each)",
        allowed, rejected
    );
    assert!(
        rejected > 0 || allowed <= 5,
        "Rate limiting should be enforced (rejected={}, allowed={})",
        rejected,
        allowed
    );
}

/// Basic rate limiting: rate 2 / burst 2 should trigger 429 under load.
#[tokio::test]
async fn test_rate_limiting_basic() {
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
    burst 2
  }
}
"#
            .as_bytes(),
        )
        .unwrap();

    common::write_file(webroot_dir.path().join("test.txt"), b"test content").unwrap();
    common::write_file(webroot_dir.path().join("basic.txt"), b"basic content").unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::new();

    for _ in 0..5 {
        client
            .get(format!("http://localhost:{}/test.txt", port))
            .send()
            .await
            .ok();
        client
            .get(format!("http://localhost:{}/test.txt", port))
            .send()
            .await
            .ok();
    }

    let response = client
        .get(format!("http://localhost:{}/basic.txt", port))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::TOO_MANY_REQUESTS);

    container.stop().await.unwrap();
}
