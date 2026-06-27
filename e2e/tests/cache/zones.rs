#[cfg(unix)]
use std::{io::Write, path::Path};

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};

use crate::common;

async fn create_ferron_container(
    config_file: &Path,
    mounts: &[(&Path, &str)],
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let ferron_image = self::common::build_ferron_image().await?;
    let mut container = ferron_image
        .with_exposed_port(ContainerPort::Tcp(80))
        .with_wait_for(WaitFor::Http(Box::new(
            HttpWaitStrategy::new("/")
                .with_port(ContainerPort::Tcp(80))
                .with_response_matcher(|_| true),
        )))
        .with_network("bridge")
        .with_mount(Mount::bind_mount(
            config_file.to_string_lossy(),
            "/etc/ferron.conf",
        ));
    for (host_path, container_path) in mounts {
        container = container.with_mount(Mount::bind_mount(
            host_path.to_string_lossy(),
            *container_path,
        ));
    }
    container.start().await
}

/// Test that two hostnames sharing a global cache zone share the same cache store.
///
/// Config:
///   cache { max_entries = 1024 }       -- global zone
///   host-a: root "/var/www/host-a", cache true
///   host-b: root "/var/www/host-b", cache true
///
/// Both host-a and host-b should independently cache their own entries in the
/// shared global zone (cache keys include hostname, so entries are distinct).
/// This test verifies that caching works correctly under a shared zone.
#[tokio::test]
async fn test_cache_zone_global_shared() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let webroot_a = common::create_temp_dir();
    #[cfg(unix)]
    let webroot_b = common::create_temp_dir();
    #[cfg(unix)]
    let mut config_file = common::create_temp_file();
    #[cfg(not(unix))]
    let webroot_a = tempfile::tempdir().unwrap();
    #[cfg(not(unix))]
    let webroot_b = tempfile::tempdir().unwrap();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    config_file
        .as_file_mut()
        .write_all(
            r#"
      {
        cache {
          max_entries 1024
        }
      }

      host-a.example.com:80 {
        root "/var/www/host-a"
        file_cache_control "public, max-age=60"
        cache true
      }

      host-b.example.com:80 {
        root "/var/www/host-b"
        file_cache_control "public, max-age=60"
        cache true
      }
    "#
            .as_bytes(),
        )
        .unwrap();

    let container = create_ferron_container(
        config_file.path(),
        &[
            (webroot_a.path(), "/var/www/host-a"),
            (webroot_b.path(), "/var/www/host-b"),
        ],
    )
    .await
    .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();

    // Write distinct files to each webroot
    common::write_file(webroot_a.path().join("test.txt"), b"host-a-content").unwrap();
    common::write_file(webroot_b.path().join("test.txt"), b"host-b-content").unwrap();

    let client = reqwest::Client::new();

    // Request from host-a: should miss, then hit
    let resp = client
        .get(format!("http://localhost:{}/test.txt", port))
        .header("Host", "host-a.example.com")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let cache_status = resp
        .headers()
        .get("Cache-Status")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        cache_status.contains("miss"),
        "expected miss, got: {cache_status}"
    );
    assert_eq!(resp.bytes().await.unwrap().as_ref(), b"host-a-content");

    let resp = client
        .get(format!("http://localhost:{}/test.txt", port))
        .header("Host", "host-a.example.com")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let cache_status = resp
        .headers()
        .get("Cache-Status")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        cache_status.contains("hit"),
        "expected hit, got: {cache_status}"
    );

    // Request from host-b: should miss (different hostname = different cache key)
    let resp = client
        .get(format!("http://localhost:{}/test.txt", port))
        .header("Host", "host-b.example.com")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let cache_status = resp
        .headers()
        .get("Cache-Status")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        cache_status.contains("miss"),
        "expected miss for host-b, got: {cache_status}"
    );
    assert_eq!(resp.bytes().await.unwrap().as_ref(), b"host-b-content");

    // Second request from host-b: should hit
    let resp = client
        .get(format!("http://localhost:{}/test.txt", port))
        .header("Host", "host-b.example.com")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let cache_status = resp
        .headers()
        .get("Cache-Status")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        cache_status.contains("hit"),
        "expected hit for host-b, got: {cache_status}"
    );

    container.stop().await.unwrap();
}

/// Test that a host opting out of the global zone with `max_entries` gets its
/// own isolated per-host cache store.
///
/// Config:
///   cache { max_entries = 1024 }       -- global zone
///   host-a: root, cache true           -- joins global zone
///   host-b: root, cache { max_entries = 512 }  -- per-host zone
///
/// Both hosts should cache independently. The per-host zone for host-b is
/// separate from the global zone used by host-a.
#[tokio::test]
async fn test_cache_zone_per_host_opt_out() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let webroot_a = common::create_temp_dir();
    #[cfg(unix)]
    let webroot_b = common::create_temp_dir();
    #[cfg(unix)]
    let mut config_file = common::create_temp_file();
    #[cfg(not(unix))]
    let webroot_a = tempfile::tempdir().unwrap();
    #[cfg(not(unix))]
    let webroot_b = tempfile::tempdir().unwrap();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    config_file
        .as_file_mut()
        .write_all(
            r#"
      {
        cache {
          max_entries 1024
        }
      }

      host-a.example.com:80 {
        root "/var/www/host-a"
        file_cache_control "public, max-age=60"
        cache true
      }

      host-b.example.com:80 {
        root "/var/www/host-b"
        file_cache_control "public, max-age=60"
        cache true
        cache {
          max_entries 512
        }
      }
    "#
            .as_bytes(),
        )
        .unwrap();

    let container = create_ferron_container(
        config_file.path(),
        &[
            (webroot_a.path(), "/var/www/host-a"),
            (webroot_b.path(), "/var/www/host-b"),
        ],
    )
    .await
    .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();

    common::write_file(webroot_a.path().join("test.txt"), b"global-zone").unwrap();
    common::write_file(webroot_b.path().join("test.txt"), b"per-host-zone").unwrap();

    let client = reqwest::Client::new();

    // host-a caches in the global zone
    let resp = client
        .get(format!("http://localhost:{}/test.txt", port))
        .header("Host", "host-a.example.com")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.bytes().await.unwrap().as_ref(), b"global-zone");

    let resp = client
        .get(format!("http://localhost:{}/test.txt", port))
        .header("Host", "host-a.example.com")
        .send()
        .await
        .unwrap();
    let cache_status = resp
        .headers()
        .get("Cache-Status")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        cache_status.contains("hit"),
        "host-a expected hit, got: {cache_status}"
    );

    // host-b caches in its own per-host zone (opted out via max_entries)
    let resp = client
        .get(format!("http://localhost:{}/test.txt", port))
        .header("Host", "host-b.example.com")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.bytes().await.unwrap().as_ref(), b"per-host-zone");

    let resp = client
        .get(format!("http://localhost:{}/test.txt", port))
        .header("Host", "host-b.example.com")
        .send()
        .await
        .unwrap();
    let cache_status = resp
        .headers()
        .get("Cache-Status")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        cache_status.contains("hit"),
        "host-b expected hit, got: {cache_status}"
    );

    container.stop().await.unwrap();
}

/// Test that a named zone allows multiple hostnames to share a cache store.
///
/// Config:
///   cache { zone "shared" { max_entries = 1024 } }
///   host-a: root, cache { zone "shared" }
///   host-b: root, cache { zone "shared" }
///
/// Both hosts use the same named zone. Caching works independently per hostname
/// (different cache keys), but both share the same physical store.
#[tokio::test]
async fn test_cache_zone_named_shared() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let webroot_a = common::create_temp_dir();
    #[cfg(unix)]
    let webroot_b = common::create_temp_dir();
    #[cfg(unix)]
    let mut config_file = common::create_temp_file();
    #[cfg(not(unix))]
    let webroot_a = tempfile::tempdir().unwrap();
    #[cfg(not(unix))]
    let webroot_b = tempfile::tempdir().unwrap();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    config_file
        .as_file_mut()
        .write_all(
            r#"
      {
        cache {
          max_entries 1024
          zone "shared" {
            max_entries 1024
          }
        }
      }

      host-a.example.com:80 {
        root "/var/www/host-a"
        file_cache_control "public, max-age=60"
        cache {
          zone "shared"
        }
      }

      host-b.example.com:80 {
        root "/var/www/host-b"
        file_cache_control "public, max-age=60"
        cache {
          zone "shared"
        }
      }
    "#
            .as_bytes(),
        )
        .unwrap();

    let container = create_ferron_container(
        config_file.path(),
        &[
            (webroot_a.path(), "/var/www/host-a"),
            (webroot_b.path(), "/var/www/host-b"),
        ],
    )
    .await
    .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();

    common::write_file(webroot_a.path().join("page.txt"), b"named-zone-a").unwrap();
    common::write_file(webroot_b.path().join("page.txt"), b"named-zone-b").unwrap();

    let client = reqwest::Client::new();

    // host-a: miss then hit
    let resp = client
        .get(format!("http://localhost:{}/page.txt", port))
        .header("Host", "host-a.example.com")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let cache_status = resp
        .headers()
        .get("Cache-Status")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        cache_status.contains("miss"),
        "host-a first request should miss, got: {cache_status}"
    );
    assert_eq!(resp.bytes().await.unwrap().as_ref(), b"named-zone-a");

    let resp = client
        .get(format!("http://localhost:{}/page.txt", port))
        .header("Host", "host-a.example.com")
        .send()
        .await
        .unwrap();
    let cache_status = resp
        .headers()
        .get("Cache-Status")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        cache_status.contains("hit"),
        "host-a second request should hit, got: {cache_status}"
    );

    // host-b: miss then hit (same named zone, different cache key)
    let resp = client
        .get(format!("http://localhost:{}/page.txt", port))
        .header("Host", "host-b.example.com")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let cache_status = resp
        .headers()
        .get("Cache-Status")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        cache_status.contains("miss"),
        "host-b first request should miss, got: {cache_status}"
    );
    assert_eq!(resp.bytes().await.unwrap().as_ref(), b"named-zone-b");

    let resp = client
        .get(format!("http://localhost:{}/page.txt", port))
        .header("Host", "host-b.example.com")
        .send()
        .await
        .unwrap();
    let cache_status = resp
        .headers()
        .get("Cache-Status")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        cache_status.contains("hit"),
        "host-b second request should hit, got: {cache_status}"
    );

    container.stop().await.unwrap();
}

/// Test that per-host caching works correctly when no global zone exists.
///
/// Config:
///   (no global cache block)
///   host-a: root, cache true   -- implicit per-host zone
///   host-b: root, cache true   -- implicit per-host zone
///
/// Each host gets its own isolated cache store.
#[tokio::test]
async fn test_cache_zone_per_host_default() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let webroot_a = common::create_temp_dir();
    #[cfg(unix)]
    let webroot_b = common::create_temp_dir();
    #[cfg(unix)]
    let mut config_file = common::create_temp_file();
    #[cfg(not(unix))]
    let webroot_a = tempfile::tempdir().unwrap();
    #[cfg(not(unix))]
    let webroot_b = tempfile::tempdir().unwrap();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    // No global cache block — each host should get its own per-host zone
    config_file
        .as_file_mut()
        .write_all(
            r#"
      host-a.example.com:80 {
        root "/var/www/host-a"
        file_cache_control "public, max-age=60"
        cache true
      }

      host-b.example.com:80 {
        root "/var/www/host-b"
        file_cache_control "public, max-age=60"
        cache true
      }
    "#
            .as_bytes(),
        )
        .unwrap();

    let container = create_ferron_container(
        config_file.path(),
        &[
            (webroot_a.path(), "/var/www/host-a"),
            (webroot_b.path(), "/var/www/host-b"),
        ],
    )
    .await
    .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();

    common::write_file(webroot_a.path().join("item.txt"), b"isolated-a").unwrap();
    common::write_file(webroot_b.path().join("item.txt"), b"isolated-b").unwrap();

    let client = reqwest::Client::new();

    // host-a: miss then hit
    let resp = client
        .get(format!("http://localhost:{}/item.txt", port))
        .header("Host", "host-a.example.com")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.bytes().await.unwrap().as_ref(), b"isolated-a");

    let resp = client
        .get(format!("http://localhost:{}/item.txt", port))
        .header("Host", "host-a.example.com")
        .send()
        .await
        .unwrap();
    let cache_status = resp
        .headers()
        .get("Cache-Status")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        cache_status.contains("hit"),
        "host-a expected hit, got: {cache_status}"
    );

    // host-b: miss then hit (separate per-host zone)
    let resp = client
        .get(format!("http://localhost:{}/item.txt", port))
        .header("Host", "host-b.example.com")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.bytes().await.unwrap().as_ref(), b"isolated-b");

    let resp = client
        .get(format!("http://localhost:{}/item.txt", port))
        .header("Host", "host-b.example.com")
        .send()
        .await
        .unwrap();
    let cache_status = resp
        .headers()
        .get("Cache-Status")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        cache_status.contains("hit"),
        "host-b expected hit, got: {cache_status}"
    );

    container.stop().await.unwrap();
}
