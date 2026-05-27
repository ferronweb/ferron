use testcontainers::core::ContainerPort;

mod common;

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

    let container = common::create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();

    // Requests should pass through since no events have been recorded to trigger a ban
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

    // Verify the server properly handles concurrent requests with abuse protection
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

    // Configure rate_limit to allow only 1 request, and abuse_protection
    // to ban after 2 rate limit events. Sending 3 fast requests should:
    //   1st → pass (consumes the only token)
    //   2nd → 429 (rate limited, emits abuse event #1)
    //   3rd → 429 (rate limited, emits abuse event #2 → ban triggered)
    //   4th → 403 (banned by abuse protection)
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

    let container = common::create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let url = format!("http://localhost:{}/data.txt", port);

    // 1st request: passes (consumes the token)
    let resp = client.get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 200, "first request should pass");

    // 2nd request: rate limited (429), triggers abuse event #1
    let resp = client.get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 429, "second request should be rate limited");

    // 3rd request: rate limited (429), triggers abuse event #2 → ban
    let resp = client.get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 429, "third request should be rate limited");

    // 4th request: IP is now banned → 403 Forbidden with Retry-After
    let resp = client.get(&url).send().await.unwrap();
    assert_eq!(
        resp.status(),
        403,
        "fourth request should be banned by abuse protection"
    );

    // Verify the Retry-After header is present
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

    // Abuse protection without rate_limit — no events are emitted so
    // no bans should be triggered regardless of request volume.
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

    let container = common::create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();

    // Many requests with no rate limiting should all pass
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
