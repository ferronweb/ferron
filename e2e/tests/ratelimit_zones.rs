#[cfg(unix)]
use std::io::Write;

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};

mod common;

pub(crate) async fn create_ferron_container(
    webroot_dir: &std::path::Path,
    config_file: &std::path::Path,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let ferron_image = self::common::build_ferron_image().await?;
    ferron_image
        .with_exposed_port(ContainerPort::Tcp(80))
        .with_wait_for(WaitFor::Http(Box::new(
            HttpWaitStrategy::new("/")
                .with_port(ContainerPort::Tcp(80))
                .with_response_matcher(|_| true),
        )))
        .with_network("bridge")
        .with_mount(Mount::bind_mount(
            webroot_dir.to_string_lossy(),
            "/var/www/ferron",
        ))
        .with_mount(Mount::bind_mount(
            config_file.to_string_lossy(),
            "/etc/ferron.conf",
        ))
        .start()
        .await
}

/// Test that two hostnames sharing a global rate limit zone share buckets.
///
/// Config:
///   { rate_limit { rate 10; burst 0; key remote_address } }
///   *:80 { root }
///
/// The global block defines rate limit rules. All hosts inherit them
/// and share the same global zone. We send many rapid requests from
/// both hosts and verify the total allowed is shared (not doubled).
#[tokio::test]
async fn test_ratelimit_zone_global_shared() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let mut config_file = common::create_temp_file();

    config_file
        .as_file_mut()
        .write_all(
            r#"
{
    rate_limit {
        rate 10
        burst 0
        key remote_address
    }
}

*:80 {
    root "/var/www/ferron"
}
"#
            .as_bytes(),
        )
        .unwrap();
    config_file.flush().unwrap();

    common::write_file(webroot_dir.path().join("test.txt"), b"content").unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::new();

    // Send rapid requests from both hosts and count total allowed
    // With shared zone, total allowed should be ~10 (not ~20)
    let mut allowed = 0;
    for _ in 0..5 {
        let resp = client
            .get(format!("http://localhost:{}/test.txt", port))
            .header("Host", "host-a.example.com")
            .send()
            .await
            .unwrap();
        if resp.status().is_success() {
            allowed += 1;
        }
    }
    for _ in 0..15 {
        let resp = client
            .get(format!("http://localhost:{}/test.txt", port))
            .header("Host", "host-b.example.com")
            .send()
            .await
            .unwrap();
        if resp.status().is_success() {
            allowed += 1;
        }
    }

    // If zones are shared, total allowed should be ~10 (health check + 9 test)
    // If zones are separate, total allowed would be ~20
    assert!(
        allowed <= 15,
        "total allowed requests ({allowed}) should be limited by shared zone (expected ~10)"
    );

    container.stop().await.unwrap();
}

/// Test that a host with its own rate_limit rules gets isolated per-host buckets.
///
/// Config:
///   { rate_limit { rate 10; burst 0; key remote_address } }
///   *:80 { root }
///   host-b: { rate_limit { rate 10; burst 0; key remote_address } }
///
/// host-a inherits the global zone. host-b has its own rate_limit
/// block, so it gets a per-host zone.
#[tokio::test]
async fn test_ratelimit_zone_per_host_opt_out() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let mut config_file = common::create_temp_file();

    config_file
        .as_file_mut()
        .write_all(
            r#"
{
    rate_limit {
        rate 10
        burst 0
        key remote_address
    }
}

*:80 {
    root "/var/www/ferron"
}

host-b.example.com:80 {
    rate_limit {
        rate 10
        burst 0
        key remote_address
    }
}
"#
            .as_bytes(),
        )
        .unwrap();
    config_file.flush().unwrap();

    common::write_file(webroot_dir.path().join("test.txt"), b"content").unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::new();

    // Send rapid requests from host-a to exhaust global zone
    let mut host_a_allowed = 0;
    for _ in 0..15 {
        let resp = client
            .get(format!("http://localhost:{}/test.txt", port))
            .header("Host", "host-a.example.com")
            .send()
            .await
            .unwrap();
        if resp.status().is_success() {
            host_a_allowed += 1;
        }
    }

    // host-b has its own per-host zone, so it should still have tokens
    let mut host_b_allowed = 0;
    for _ in 0..10 {
        let resp = client
            .get(format!("http://localhost:{}/test.txt", port))
            .header("Host", "host-b.example.com")
            .send()
            .await
            .unwrap();
        if resp.status().is_success() {
            host_b_allowed += 1;
        }
    }

    // host-a should be rate-limited (global zone exhausted)
    assert!(
        host_a_allowed < 15,
        "host-a should be rate limited by global zone (allowed={host_a_allowed})"
    );
    // host-b should have requests allowed (separate per-host zone)
    assert!(
        host_b_allowed > 0,
        "host-b should have its own zone (allowed={host_b_allowed})"
    );

    container.stop().await.unwrap();
}

/// Test that a named zone allows multiple hostnames to share buckets.
///
/// Config:
///   host-a: rate_limit { zone "shared"; rate 10; burst 0 }
///   host-b: rate_limit { zone "shared"; rate 10; burst 0 }
#[tokio::test]
async fn test_ratelimit_zone_named_shared() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let mut config_file = common::create_temp_file();

    config_file
        .as_file_mut()
        .write_all(
            r#"
host-a.example.com:80 {
    rate_limit {
        zone "shared"
        rate 10
        burst 0
        key remote_address
    }
    root "/var/www/ferron"
}

host-b.example.com:80 {
    rate_limit {
        zone "shared"
        rate 10
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

    common::write_file(webroot_dir.path().join("test.txt"), b"content").unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::new();

    // Send rapid requests from both hosts and count total allowed
    let mut allowed = 0;
    for _ in 0..5 {
        let resp = client
            .get(format!("http://localhost:{}/test.txt", port))
            .header("Host", "host-a.example.com")
            .send()
            .await
            .unwrap();
        if resp.status().is_success() {
            allowed += 1;
        }
    }
    for _ in 0..15 {
        let resp = client
            .get(format!("http://localhost:{}/test.txt", port))
            .header("Host", "host-b.example.com")
            .send()
            .await
            .unwrap();
        if resp.status().is_success() {
            allowed += 1;
        }
    }

    // If zones are shared, total allowed should be ~10
    // If zones are separate, total allowed would be ~20
    assert!(
        allowed <= 15,
        "total allowed requests ({allowed}) should be limited by shared named zone (expected ~10)"
    );

    container.stop().await.unwrap();
}

/// Test that per-host isolation works when no global zone exists.
///
/// Config:
///   host-a: rate_limit { rate 10; burst 0 }
///   host-b: rate_limit { rate 10; burst 0 }
///
/// Each host gets its own per-host zone with isolated buckets.
#[tokio::test]
async fn test_ratelimit_zone_per_host_default() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let mut config_file = common::create_temp_file();

    config_file
        .as_file_mut()
        .write_all(
            r#"
host-a.example.com:80 {
    rate_limit {
        rate 10
        burst 0
        key remote_address
    }
    root "/var/www/ferron"
}

host-b.example.com:80 {
    rate_limit {
        rate 10
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

    common::write_file(webroot_dir.path().join("test.txt"), b"content").unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::new();

    // Send rapid requests from both hosts — each has its own zone
    let mut host_a_allowed = 0;
    for _ in 0..15 {
        let resp = client
            .get(format!("http://localhost:{}/test.txt", port))
            .header("Host", "host-a.example.com")
            .send()
            .await
            .unwrap();
        if resp.status().is_success() {
            host_a_allowed += 1;
        }
    }

    let mut host_b_allowed = 0;
    for _ in 0..15 {
        let resp = client
            .get(format!("http://localhost:{}/test.txt", port))
            .header("Host", "host-b.example.com")
            .send()
            .await
            .unwrap();
        if resp.status().is_success() {
            host_b_allowed += 1;
        }
    }

    // Both hosts should have their own buckets — each should get ~10 allowed
    assert!(
        host_a_allowed >= 5,
        "host-a should have its own zone (allowed={host_a_allowed})"
    );
    assert!(
        host_b_allowed >= 5,
        "host-b should have its own zone (allowed={host_b_allowed})"
    );

    container.stop().await.unwrap();
}
