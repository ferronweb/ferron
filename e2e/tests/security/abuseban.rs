use testcontainers::core::ContainerPort;

use crate::common;

#[tokio::test]
async fn test_abuse_protection_does_not_block_normal_traffic() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let config_file = common::create_temp_file();

    common::write_file(
        config_file.path().to_path_buf(),
        r#"
*:80 {
  root "/var/www/ferron"
  abuse_protection {
    ban_duration "1m"
    rate_limit_threshold {
      events 1
      window "60s"
    }
  }
}
"#
        .as_bytes(),
    )
    .unwrap();

    common::write_file(
        webroot_dir.path().join("test.txt").to_path_buf(),
        b"test content",
    )
    .unwrap();

    let container = crate::create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();

    let response = client
        .get(format!("http://localhost:{}/test.txt", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    let response = client
        .get(format!("http://localhost:{}/test.txt", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    let mut handles = Vec::new();
    for _ in 0..10 {
        let url = format!("http://localhost:{}/test.txt", port);
        let cl = client.clone();
        handles.push(tokio::spawn(async move { cl.get(&url).send().await }));
    }
    for handle in handles {
        let response = handle.await.unwrap().unwrap();
        assert_eq!(response.status(), 200);
    }

    container.stop().await.unwrap();
}

#[tokio::test]
async fn test_abuse_protection_blocks_rate_limit_abusers() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let config_file = common::create_temp_file();

    common::write_file(
        config_file.path().to_path_buf(),
        r#"
*:80 {
  root "/var/www/ferron"

  rate_limit {
    rate 2
    burst 0
    key remote_address
  }

  abuse_protection {
    ban_duration "1m"
    rate_limit_threshold {
      events 2
      window "60s"
    }
  }
}
"#
        .as_bytes(),
    )
    .unwrap();

    common::write_file(webroot_dir.path().join("data.txt").to_path_buf(), b"data").unwrap();

    let container = crate::create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let url = format!("http://localhost:{}/data.txt", port);

    let resp = client.get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 200, "first request should pass");

    let resp = client.get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 429, "second request should be rate limited");

    let resp = client.get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 429, "third request should be rate limited");

    let resp = client.get(&url).send().await.unwrap();
    assert_eq!(
        resp.status(),
        403,
        "fourth request should be banned by abuse protection"
    );

    let retry_after = resp.headers().get("retry-after");
    assert!(
        retry_after.is_some(),
        "banned response should have Retry-After header"
    );

    container.stop().await.unwrap();
}

#[tokio::test]
async fn test_abuse_protection_without_rate_limit_does_not_block() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let config_file = common::create_temp_file();

    common::write_file(
        config_file.path().to_path_buf(),
        r#"
*:80 {
  root "/var/www/ferron"
  abuse_protection {
    enabled true
    ban_duration "1m"
    rate_limit_threshold {
      events 1
      window "60s"
    }
  }
}
"#
        .as_bytes(),
    )
    .unwrap();

    common::write_file(webroot_dir.path().join("page.html").to_path_buf(), b"page").unwrap();

    let container = crate::create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();

    for _ in 0..20 {
        let resp = client
            .get(format!("http://localhost:{}/page.html", port))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
    }

    container.stop().await.unwrap();
}

#[tokio::test]
async fn test_abuse_protection_blocks_error_rate_abusers() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let config_file = common::create_temp_file();

    common::write_file(
        config_file.path().to_path_buf(),
        r#"
*:80 {
  root "/var/www/ferron"
  abuse_protection {
    ban_duration "1m"
    error_rate_threshold {
      events 2
      window "60s"
      status_codes "404"
    }
  }
}
"#
        .as_bytes(),
    )
    .unwrap();

    common::write_file(webroot_dir.path().join("exists.txt").to_path_buf(), b"content").unwrap();

    let container = crate::create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();

    // First request to non-existent file: 404
    let resp = client
        .get(format!("http://localhost:{}/nonexistent.txt", port))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404, "first 404 request should pass");

    // Second request to non-existent file: 404 (still below threshold)
    let resp = client
        .get(format!("http://localhost:{}/another-nonexistent.txt", port))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404, "second 404 request should pass");

    // Third request: should be banned (threshold reached)
    let resp = client
        .get(format!("http://localhost:{}/yet-another.txt", port))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        403,
        "third request should be banned by error rate threshold"
    );

    let retry_after = resp.headers().get("retry-after");
    assert!(
        retry_after.is_some(),
        "banned response should have Retry-After header"
    );

    // Existing file should also be blocked while banned
    let resp = client
        .get(format!("http://localhost:{}/exists.txt", port))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        403,
        "existing file should also be blocked while banned"
    );

    container.stop().await.unwrap();
}
