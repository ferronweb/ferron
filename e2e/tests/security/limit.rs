use std::io::Write;

use testcontainers::core::ContainerPort;

use crate::{common, create_ferron_container};

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

#[tokio::test]
async fn test_rate_limiting_basic_throttle() {
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
    throttle
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

    // 2 requests/iteration * 2 iterations = 4 requests, just enough to trigger the rate limit
    for i in 0..2 {
        if i > 0 {
            // Spawn as background task to avoid stalling the test
            let client = client.clone();
            tokio::spawn(async move {
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
            });
        } else {
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
    }

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // rate_limit {
    //  rate 2
    //  burst 2
    //  throttle
    // }
    //
    // Here, the rate limit is set to 2 requests per second with a burst of 2.
    //
    // The throttle option means that when the limit is exceeded, instead of immediately
    // returning a 429 Too Many Requests response, the server will delay the response until
    // the rate limit allows it.
    //
    // In Ferron 3, refill rate per second is allowed rate per second + burst.
    // Therefore, the delay would be around 1 / 4 = 0.25 seconds per request when the limit
    // is exceeded.

    let start = std::time::Instant::now();
    let response = client
        .get(format!("http://localhost:{}/basic.txt", port))
        .send()
        .await
        .unwrap();

    assert_ne!(response.status(), reqwest::StatusCode::TOO_MANY_REQUESTS);
    assert!(start.elapsed().as_millis() > 150); // ± 100ms variance allowed

    container.stop().await.unwrap();
}
