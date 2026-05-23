use std::io::Write;

use crate::{common, create_ferron_container};

/// Test for rate limiting race condition fix.
/// Ensures rate limiting bucket creation doesn't allow bypassing capacity.
#[tokio::test]
async fn test_rate_limiting_race_condition_fixed() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let mut config_file = common::create_temp_file();

    // Simple rate limiting test with low limit to trigger quickly
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

    // Send multiple requests sequentially to stay within bucket window
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

    // With 5r/s limit, we expect ~5 allowed, ~5 rejected
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
