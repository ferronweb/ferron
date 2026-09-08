use std::io::Write;
use std::path::Path;

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};

use crate::common;

async fn create_redis_container(
    network: &str,
) -> Result<ContainerAsync<GenericImage>, testcontainers::TestcontainersError> {
    GenericImage::new("redis", "8-alpine")
        .with_exposed_port(ContainerPort::Tcp(6379))
        .with_wait_for(WaitFor::message_on_stdout("Ready to accept connections"))
        .with_network(network)
        .with_hostname("redis")
        .start()
        .await
}

async fn create_ferron_on_network(
    network: &str,
    webroot_dir: &Path,
    config_file: &Path,
) -> Result<ContainerAsync<GenericImage>, testcontainers::TestcontainersError> {
    let ferron_image = common::build_ferron_image().await?;
    ferron_image
        .with_exposed_port(ContainerPort::Tcp(80))
        .with_wait_for(WaitFor::Http(Box::new(
            HttpWaitStrategy::new("/")
                .with_port(ContainerPort::Tcp(80))
                .with_response_matcher(|_| true),
        )))
        .with_network(network)
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

/// Distributed rate limiting via Redis: burst is enforced through Redis.
#[tokio::test]
async fn test_rate_limiting_redis_basic() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let network = format!("e2e-ratelimit-redis-{}", std::process::id());
    let _redis = create_redis_container(&network)
        .await
        .expect("Failed to start redis");

    let webroot_dir = common::create_temp_dir();
    let mut config_file = common::create_temp_file();
    common::write_file(webroot_dir.path().join("test.txt"), b"test content").unwrap();

    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
    rate_limit_backend {
        type redis
        url "redis://redis:6379/0"
        key_prefix "e2e:rl:basic:"
        timeout "2s"
    }
    rate_limit {
        rate 2
        burst 0
        key uri
    }
    root "/var/www/ferron"
}
"#
            .as_bytes(),
        )
        .unwrap();
    config_file.flush().unwrap();

    let container = create_ferron_on_network(&network, webroot_dir.path(), config_file.path())
        .await
        .expect("Failed to create ferron");

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::new();

    let mut allowed = 0;
    let mut rejected = 0;
    for _ in 0..4 {
        let response = client
            .get(format!("http://localhost:{}/test.txt", port))
            .send()
            .await
            .expect("request failed");
        if response.status().is_success() {
            allowed += 1;
        } else if response.status() == 429 {
            rejected += 1;
        }
    }

    assert_eq!(allowed, 2, "expected burst of 2 through Redis");
    assert_eq!(rejected, 2, "expected 2 rejections through Redis");

    container.stop().await.unwrap();
}

/// Fail-open: unreachable Redis with default `fail_open true` allows traffic.
#[tokio::test]
async fn test_rate_limiting_redis_fail_open() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let mut config_file = common::create_temp_file();
    common::write_file(webroot_dir.path().join("test.txt"), b"test content").unwrap();

    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
    rate_limit_backend {
        type redis
        url "redis://127.0.0.1:6399/0"
        timeout "1s"
    }
    rate_limit {
        rate 1
        burst 0
    }
    root "/var/www/ferron"
}
"#
            .as_bytes(),
        )
        .unwrap();
    config_file.flush().unwrap();

    let container = crate::create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .expect("Failed to create ferron");

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::new();

    for _ in 0..3 {
        let response = client
            .get(format!("http://localhost:{}/test.txt", port))
            .send()
            .await
            .expect("request failed");
        assert!(
            response.status().is_success(),
            "fail-open should allow on Redis error, got {}",
            response.status()
        );
    }

    container.stop().await.unwrap();
}
