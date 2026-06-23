#[cfg(unix)]
use std::{io::Write, path::Path};

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};

mod common;

async fn create_ferron_container(
    webroot_dir: &Path,
    config_file: &Path,
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

#[tokio::test]
async fn test_cache_hit() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Set umask to 000 to ensure that the webroot directory is accessible to the container.
    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let webroot_dir = common::create_temp_dir();
    #[cfg(unix)]
    let mut config_file = common::create_temp_file();
    #[cfg(not(unix))]
    let webroot_dir = tempfile::tempdir().unwrap();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    config_file
        .as_file_mut()
        .write_all(
            r#"
      *:80 {
        root "/var/www/ferron"
        file_cache_control "public, max-age=60"
        cache true
      }
  "#
            .as_bytes(),
        )
        .unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    self::common::write_file(webroot_dir.path().join("test.txt"), "v1".as_bytes()).unwrap();
    let response = reqwest::get(format!(
        "http://localhost:{}/test.txt",
        container
            .get_host_port_ipv4(ContainerPort::Tcp(80))
            .await
            .unwrap()
    ))
    .await
    .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert!(
        String::from_utf8_lossy(response.headers().get("Cache-Status").unwrap().as_bytes())
            .contains("miss")
    );
    assert_eq!(&*response.bytes().await.unwrap(), b"v1");

    self::common::write_file(webroot_dir.path().join("test.txt"), "v2".as_bytes()).unwrap();
    let response = reqwest::get(format!(
        "http://localhost:{}/test.txt",
        container
            .get_host_port_ipv4(ContainerPort::Tcp(80))
            .await
            .unwrap()
    ))
    .await
    .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert!(
        String::from_utf8_lossy(response.headers().get("Cache-Status").unwrap().as_bytes())
            .contains("hit")
    );
    assert_eq!(&*response.bytes().await.unwrap(), b"v1");

    container.stop().await.unwrap();
}

#[tokio::test]
async fn test_cache_expiry() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Set umask to 000 to ensure that the webroot directory is accessible to the container.
    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let webroot_dir = common::create_temp_dir();
    #[cfg(unix)]
    let mut config_file = common::create_temp_file();
    #[cfg(not(unix))]
    let webroot_dir = tempfile::tempdir().unwrap();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    config_file
        .as_file_mut()
        .write_all(
            r#"
      *:80 {
        root "/var/www/ferron"
        file_cache_control "public, max-age=2"
        cache true
      }
  "#
            .as_bytes(),
        )
        .unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    self::common::write_file(webroot_dir.path().join("test.txt"), "v1".as_bytes()).unwrap();
    let response = reqwest::get(format!(
        "http://localhost:{}/test.txt",
        container
            .get_host_port_ipv4(ContainerPort::Tcp(80))
            .await
            .unwrap()
    ))
    .await
    .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert!(
        String::from_utf8_lossy(response.headers().get("Cache-Status").unwrap().as_bytes())
            .contains("miss")
    );
    assert_eq!(&*response.bytes().await.unwrap(), b"v1");

    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    self::common::write_file(webroot_dir.path().join("test.txt"), "v2".as_bytes()).unwrap();
    let response = reqwest::get(format!(
        "http://localhost:{}/test.txt",
        container
            .get_host_port_ipv4(ContainerPort::Tcp(80))
            .await
            .unwrap()
    ))
    .await
    .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert!(
        String::from_utf8_lossy(response.headers().get("Cache-Status").unwrap().as_bytes())
            .contains("miss")
    );
    assert_eq!(&*response.bytes().await.unwrap(), b"v2");

    container.stop().await.unwrap();
}

#[tokio::test]
async fn test_cache_vary() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Set umask to 000 to ensure that the webroot directory is accessible to the container.
    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let webroot_dir = common::create_temp_dir();
    #[cfg(unix)]
    let mut config_file = common::create_temp_file();
    #[cfg(not(unix))]
    let webroot_dir = tempfile::tempdir().unwrap();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    config_file
        .as_file_mut()
        .write_all(
            r#"
      *:80 {
        root "/var/www/ferron"
        file_cache_control "public, max-age=2"
        cache {
          vary "X-Test-Header"
        }
      }
  "#
            .as_bytes(),
        )
        .unwrap();

    let container = create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    self::common::write_file(webroot_dir.path().join("test.txt"), "v1".as_bytes()).unwrap();
    let response = reqwest::Client::new()
        .get(format!(
            "http://localhost:{}/test.txt",
            container
                .get_host_port_ipv4(ContainerPort::Tcp(80))
                .await
                .unwrap()
        ))
        .header("X-Test-Header", "A")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert!(
        String::from_utf8_lossy(response.headers().get("Cache-Status").unwrap().as_bytes())
            .contains("miss")
    );
    assert_eq!(&*response.bytes().await.unwrap(), b"v1");

    self::common::write_file(webroot_dir.path().join("test.txt"), "v2".as_bytes()).unwrap();
    let response = reqwest::Client::new()
        .get(format!(
            "http://localhost:{}/test.txt",
            container
                .get_host_port_ipv4(ContainerPort::Tcp(80))
                .await
                .unwrap()
        ))
        .header("X-Test-Header", "B")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert!(
        String::from_utf8_lossy(response.headers().get("Cache-Status").unwrap().as_bytes())
            .contains("miss")
    );
    assert_eq!(&*response.bytes().await.unwrap(), b"v2");

    let response = reqwest::Client::new()
        .get(format!(
            "http://localhost:{}/test.txt",
            container
                .get_host_port_ipv4(ContainerPort::Tcp(80))
                .await
                .unwrap()
        ))
        .header("X-Test-Header", "A")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert!(
        String::from_utf8_lossy(response.headers().get("Cache-Status").unwrap().as_bytes())
            .contains("hit")
    );
    assert_eq!(&*response.bytes().await.unwrap(), b"v1");

    container.stop().await.unwrap();
}

// Helper to create a backend container for cache revalidation tests.
async fn create_backend_container(
    network: &str,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let backend_image = self::common::build_backend_image().await?;
    backend_image
        .with_exposed_port(ContainerPort::Tcp(3000))
        .with_wait_for(WaitFor::Http(Box::new(
            HttpWaitStrategy::new("/")
                .with_port(ContainerPort::Tcp(3000))
                .with_response_matcher(|_| true),
        )))
        .with_network(network)
        .with_hostname("backend")
        .start()
        .await
}

/// Context for cache revalidation E2E tests using a reverse proxy + cache.
struct CacheRevalidationTestContext {
    _backend: ContainerAsync<GenericImage>,
    _ferron: ContainerAsync<GenericImage>,
    base_url: String,
    client: reqwest::Client,
}

impl CacheRevalidationTestContext {
    async fn new(test_name: &str) -> Self {
        let _ = rustls::crypto::ring::default_provider().install_default();

        #[cfg(unix)]
        nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

        #[cfg(unix)]
        let mut config_file = common::create_temp_file();
        #[cfg(not(unix))]
        let mut config_file = tempfile::NamedTempFile::new().unwrap();

        let network = format!("e2e-test-cache-etag-{}", test_name);
        let backend = create_backend_container(&network).await.unwrap();

        config_file
            .as_file_mut()
            .write_all(
                br#"
      *:80 {
        proxy "http://backend:3000"
        cache {
          emit_litespeed_headers true
        }
      }
      "#,
            )
            .unwrap();

        let ferron_image = self::common::build_ferron_image().await.unwrap();
        let ferron = ferron_image
            .with_exposed_port(ContainerPort::Tcp(80))
            .with_wait_for(WaitFor::Http(Box::new(
                HttpWaitStrategy::new("/")
                    .with_port(ContainerPort::Tcp(80))
                    .with_response_matcher(|_| true),
            )))
            .with_network(&network)
            .with_hostname("ferron")
            .with_mount(Mount::bind_mount(
                config_file.path().to_string_lossy().to_string(),
                "/etc/ferron.conf",
            ))
            .start()
            .await
            .unwrap();

        let port = ferron
            .get_host_port_ipv4(ContainerPort::Tcp(80))
            .await
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        Self {
            _backend: backend,
            _ferron: ferron,
            base_url: format!("http://localhost:{}", port),
            client: reqwest::Client::new(),
        }
    }

    async fn get(&self, path: &str) -> reqwest::Response {
        self.client
            .get(format!(
                "{}/{}",
                self.base_url,
                path.trim_start_matches('/')
            ))
            .send()
            .await
            .unwrap()
    }

    async fn get_with_headers(&self, path: &str, headers: &[(&str, &str)]) -> reqwest::Response {
        let mut req = self.client.get(format!(
            "{}/{}",
            self.base_url,
            path.trim_start_matches('/')
        ));
        for (name, value) in headers {
            req = req.header(*name, *value);
        }
        req.send().await.unwrap()
    }

    async fn update_backend_version(&self, id: &str) {
        // Access the backend container directly (not through Ferron) to update
        // the stored version counter. Both containers are on the same network.
        let backend_port = self
            ._backend
            .get_host_port_ipv4(ContainerPort::Tcp(3000))
            .await
            .unwrap();
        let resp = reqwest::Client::new()
            .post(format!(
                "http://localhost:{}/cache-etag/update?id={}",
                backend_port, id
            ))
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success());
    }
}

#[tokio::test]
async fn test_cache_etag_revalidation_304() {
    let ctx = CacheRevalidationTestContext::new("revalidate-304").await;

    // First request: cache miss, stores response with ETag W/"v1"
    let resp = ctx.get("/cache-etag?id=reval-304").await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let cache_status = resp
        .headers()
        .get("X-LiteSpeed-Cache")
        .expect("X-LiteSpeed-Cache header missing")
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(cache_status, "miss");
    assert_eq!(resp.text().await.unwrap(), "v1");

    // Second request with max-age=0: triggers conditional revalidation.
    // Backend receives If-None-Match: W/"v1" and returns 304.
    // Ferron should reconstruct a 200 response with the cached body.
    let resp = ctx
        .get_with_headers(
            "/cache-etag?id=reval-304",
            &[("Cache-Control", "max-age=0")],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let cache_status = resp
        .headers()
        .get("X-LiteSpeed-Cache")
        .expect("X-LiteSpeed-Cache header missing")
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(cache_status, "hit");
    // The Cache-Status header should indicate revalidation
    let cache_status_header = resp
        .headers()
        .get("Cache-Status")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        cache_status_header.contains("revalidated"),
        "Expected revalidated Cache-Status, got: {}",
        cache_status_header
    );
    assert_eq!(resp.text().await.unwrap(), "v1");
}

#[tokio::test]
async fn test_cache_etag_content_change() {
    let ctx = CacheRevalidationTestContext::new("content-change").await;

    // First request: cache miss
    let resp = ctx.get("/cache-etag?id=content-change").await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "v1");

    // Trigger a content update on the backend
    ctx.update_backend_version("content-change").await;

    // Request with max-age=0: backend now returns v2 with new ETag
    let resp = ctx
        .get_with_headers(
            "/cache-etag?id=content-change",
            &[("Cache-Control", "max-age=0")],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let cache_status = resp
        .headers()
        .get("X-LiteSpeed-Cache")
        .expect("X-LiteSpeed-Cache header missing")
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(cache_status, "miss");
    assert_eq!(
        resp.text().await.unwrap(),
        "v2",
        "Should return updated content"
    );

    // Next unconditional request should hit the cache with the new content
    let resp = ctx.get("/cache-etag?id=content-change").await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let cache_status = resp
        .headers()
        .get("X-LiteSpeed-Cache")
        .expect("X-LiteSpeed-Cache header missing")
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(cache_status, "hit");
    assert_eq!(resp.text().await.unwrap(), "v2");
}

#[tokio::test]
async fn test_cache_no_cache_triggers_revalidation() {
    let ctx = CacheRevalidationTestContext::new("no-cache-reval").await;

    // First request: cache miss
    let resp = ctx.get("/cache-etag?id=no-cache-reval").await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "v1");

    // Request with Cache-Control: no-cache triggers revalidation
    let resp = ctx
        .get_with_headers(
            "/cache-etag?id=no-cache-reval",
            &[("Cache-Control", "no-cache")],
        )
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let cache_status_old = resp
        .headers()
        .get("Cache-Status")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        cache_status_old.contains("revalidated"),
        "Expected revalidated Cache-Status for no-cache, got: {}",
        cache_status_old
    );
    assert_eq!(
        resp.text().await.unwrap(),
        "v1",
        "should return cached content after 304"
    );
}

/// Test stale-while-revalidate behavior
#[tokio::test]
async fn test_cache_stale_while_revalidate() {
    let ctx = CacheRevalidationTestContext::new("swr").await;

    // First request: cache miss, stores response with max-age=1, stale-while-revalidate=60
    let resp = ctx.get("/cache-swr?id=swr-test").await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let cache_status = resp
        .headers()
        .get("X-LiteSpeed-Cache")
        .expect("X-LiteSpeed-Cache header missing")
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(cache_status, "miss");
    assert_eq!(resp.text().await.unwrap(), "swr-v1");

    // Wait for the entry to expire (max-age=1s)
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Second request: entry is stale but within SWR window
    // Should serve stale content immediately
    let resp = ctx.get("/cache-swr?id=swr-test").await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let cache_status = resp
        .headers()
        .get("X-LiteSpeed-Cache")
        .expect("X-LiteSpeed-Cache header missing")
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(cache_status, "hit", "SWR should serve stale as hit");

    // Verify Cache-Status header contains stale-while-revalidate detail
    let cache_status_header = resp
        .headers()
        .get("Cache-Status")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        cache_status_header.contains("stale-while-revalidate"),
        "Cache-Status should indicate stale-while-revalidate, got: {}",
        cache_status_header
    );

    assert_eq!(
        resp.text().await.unwrap(),
        "swr-v1",
        "SWR should return stale content"
    );
}

/// Test stale-if-error behavior
#[tokio::test]
async fn test_cache_stale_if_error() {
    let ctx = CacheRevalidationTestContext::new("sie").await;

    // First request: cache miss, stores response with max-age=300, stale-if-error=60
    let resp = ctx.get("/cache-sie?id=sie-test").await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let cache_status = resp
        .headers()
        .get("X-LiteSpeed-Cache")
        .expect("X-LiteSpeed-Cache header missing")
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(cache_status, "miss");
    assert_eq!(resp.text().await.unwrap(), "sie-v1");

    // Enable error mode on the backend
    let backend_port = ctx
        ._backend
        .get_host_port_ipv4(ContainerPort::Tcp(3000))
        .await
        .unwrap();
    let resp = reqwest::Client::new()
        .post(format!(
            "http://localhost:{}/cache-sie/error?id=sie-test",
            backend_port
        ))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());

    // Request with max-age=0 to trigger revalidation with the backend
    // Backend returns 503, Ferron should serve stale content (SIE)
    let resp = ctx
        .get_with_headers("/cache-sie?id=sie-test", &[("Cache-Control", "max-age=0")])
        .await;
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let cache_status = resp
        .headers()
        .get("X-LiteSpeed-Cache")
        .expect("X-LiteSpeed-Cache header missing")
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(cache_status, "hit", "SIE should serve stale as hit");

    // Verify Cache-Status header contains stale-while-revalidate detail (SIE reuses this variant)
    let cache_status_header = resp
        .headers()
        .get("Cache-Status")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        cache_status_header.contains("stale-while-revalidate"),
        "Cache-Status should indicate stale-while-revalidate for SIE, got: {}",
        cache_status_header
    );

    assert_eq!(
        resp.text().await.unwrap(),
        "sie-v1",
        "SIE should return stale content"
    );
}
