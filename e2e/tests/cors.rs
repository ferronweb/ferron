use std::io::Write;

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};

mod common;

struct CorsTestContext {
    _container: ContainerAsync<GenericImage>,
    _webroot_dir: tempfile::TempDir,
    _config_file: tempfile::NamedTempFile,
    base_url: String,
    client: reqwest::Client,
}

impl CorsTestContext {
    async fn new(config_body: &[u8]) -> Self {
        let _ = rustls::crypto::ring::default_provider().install_default();

        #[cfg(unix)]
        nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

        #[cfg(unix)]
        let webroot_dir = common::create_temp_dir();
        #[cfg(not(unix))]
        let webroot_dir = tempfile::tempdir().unwrap();

        std::fs::write(webroot_dir.path().join("index.html"), b"cors test").unwrap();

        #[cfg(unix)]
        let mut config_file = common::create_temp_file();
        #[cfg(not(unix))]
        let mut config_file = tempfile::NamedTempFile::new().unwrap();

        config_file.as_file_mut().write_all(config_body).unwrap();
        config_file.flush().unwrap();

        let ferron_image = self::common::build_ferron_image().await.unwrap();
        let container = ferron_image
            .with_exposed_port(ContainerPort::Tcp(80))
            .with_wait_for(WaitFor::Http(Box::new(
                HttpWaitStrategy::new("/")
                    .with_port(ContainerPort::Tcp(80))
                    .with_response_matcher(|_| true),
            )))
            .with_mount(Mount::bind_mount(
                webroot_dir.path().to_string_lossy(),
                "/var/www/ferron",
            ))
            .with_mount(Mount::bind_mount(
                config_file.path().to_string_lossy(),
                "/etc/ferron.conf",
            ))
            .start()
            .await
            .unwrap();

        let port = container
            .get_host_port_ipv4(ContainerPort::Tcp(80))
            .await
            .unwrap();

        let client = reqwest::Client::builder().build().unwrap();
        let base_url = format!("http://localhost:{port}");

        Self {
            _container: container,
            _webroot_dir: webroot_dir,
            _config_file: config_file,
            base_url,
            client,
        }
    }
}

/// CORS preflight with wildcard origin should return 204 with CORS headers.
#[tokio::test]
async fn test_cors_preflight_wildcard_origin() {
    let config = br#"
*:80 {
    cors {
        origins "*"
        methods GET POST
        headers "Content-Type"
        max_age 3600
    }
    root "/var/www/ferron"
}
"#;

    let ctx = CorsTestContext::new(config).await;

    // Preflight request
    let resp = ctx
        .client
        .request(
            reqwest::Method::OPTIONS,
            format!("{}/index.html", ctx.base_url),
        )
        .header("Origin", "https://example.com")
        .header("Access-Control-Request-Method", "GET")
        .send()
        .await
        .expect("preflight failed");

    assert_eq!(
        resp.status(),
        204,
        "CORS preflight should return 204 No Content"
    );

    let headers = resp.headers();
    assert_eq!(
        headers
            .get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok()),
        Some("*"),
        "Access-Control-Allow-Origin should be *"
    );
    assert!(
        headers.contains_key("access-control-allow-methods"),
        "Should have Access-Control-Allow-Methods header"
    );
    assert!(
        headers.contains_key("access-control-allow-headers"),
        "Should have Access-Control-Allow-Headers header"
    );
    assert_eq!(
        headers
            .get("access-control-max-age")
            .and_then(|v| v.to_str().ok()),
        Some("3600"),
        "Access-Control-Max-Age should be 3600"
    );
}

/// Simple CORS request: GET with Origin should get Access-Control-Allow-Origin back.
#[tokio::test]
async fn test_cors_simple_request() {
    let config = br#"
*:80 {
    cors {
        origins "https://example.com" "https://app.example.com"
        methods GET POST
    }
    root "/var/www/ferron"
}
"#;

    let ctx = CorsTestContext::new(config).await;

    let resp = ctx
        .client
        .get(format!("{}/index.html", ctx.base_url))
        .header("Origin", "https://example.com")
        .send()
        .await
        .expect("GET failed");

    assert_eq!(resp.status(), 200, "Expected 200 OK");
    assert_eq!(
        resp.headers()
            .get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok()),
        Some("https://example.com"),
        "Access-Control-Allow-Origin should echo the matching origin"
    );
}

/// Request from a non-allowed origin should NOT get CORS headers.
#[tokio::test]
async fn test_cors_disallowed_origin() {
    let config = br#"
*:80 {
    cors {
        origins "https://allowed.example.com"
        methods GET
    }
    root "/var/www/ferron"
}
"#;

    let ctx = CorsTestContext::new(config).await;

    let resp = ctx
        .client
        .get(format!("{}/index.html", ctx.base_url))
        .header("Origin", "https://evil.example.com")
        .send()
        .await
        .expect("GET failed");

    assert_eq!(resp.status(), 200, "Expected 200 OK");
    assert!(
        !resp.headers().contains_key("access-control-allow-origin"),
        "Non-allowed origin should NOT get CORS headers"
    );
}

/// CORS with credentials should set Access-Control-Allow-Credentials: true.
#[tokio::test]
async fn test_cors_with_credentials() {
    let config = br#"
*:80 {
    cors {
        origins "https://example.com"
        methods GET
        credentials true
    }
    root "/var/www/ferron"
}
"#;

    let ctx = CorsTestContext::new(config).await;

    let resp = ctx
        .client
        .get(format!("{}/index.html", ctx.base_url))
        .header("Origin", "https://example.com")
        .send()
        .await
        .expect("GET failed");

    assert_eq!(resp.status(), 200, "Expected 200 OK");
    assert_eq!(
        resp.headers()
            .get("access-control-allow-credentials")
            .and_then(|v| v.to_str().ok()),
        Some("true"),
        "Access-Control-Allow-Credentials should be true"
    );
    assert_eq!(
        resp.headers()
            .get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok()),
        Some("https://example.com"),
        "Access-Control-Allow-Origin should be the specific origin (not *) when credentials are enabled"
    );
}

/// CORS with expose_headers should include Access-Control-Expose-Headers.
#[tokio::test]
async fn test_cors_expose_headers() {
    let config = br#"
*:80 {
    cors {
        origins "*"
        methods GET
        expose_headers "X-Total-Count" "X-Page"
    }
    root "/var/www/ferron"
}
"#;

    let ctx = CorsTestContext::new(config).await;

    let resp = ctx
        .client
        .get(format!("{}/index.html", ctx.base_url))
        .header("Origin", "https://example.com")
        .send()
        .await
        .expect("GET failed");

    assert_eq!(resp.status(), 200, "Expected 200 OK");
    let expose = resp
        .headers()
        .get("access-control-expose-headers")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        expose.contains("X-Total-Count"),
        "Access-Control-Expose-Headers should contain X-Total-Count, got: {expose}"
    );
    assert!(
        expose.contains("X-Page"),
        "Access-Control-Expose-Headers should contain X-Page, got: {expose}"
    );
}

/// Vary: Origin header should be present on CORS responses with non-wildcard origins.
#[tokio::test]
async fn test_cors_vary_origin_header() {
    let config = br#"
*:80 {
    cors {
        origins "https://example.com"
        methods GET
    }
    root "/var/www/ferron"
}
"#;

    let ctx = CorsTestContext::new(config).await;

    let resp = ctx
        .client
        .get(format!("{}/index.html", ctx.base_url))
        .header("Origin", "https://example.com")
        .send()
        .await
        .expect("GET failed");

    assert_eq!(resp.status(), 200, "Expected 200 OK");
    let vary = resp
        .headers()
        .get("vary")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        vary.to_lowercase().contains("origin"),
        "Vary header should contain Origin, got: {vary}"
    );
}
