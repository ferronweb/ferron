#[cfg(unix)]
use std::{io::Write, path::Path, time::Duration};

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};

async fn create_auth_backend_container(
    network: &str,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let auth_backend_image = crate::common::build_auth_backend_image().await?;
    auth_backend_image
        .with_exposed_port(ContainerPort::Tcp(9090))
        .with_wait_for(WaitFor::Http(Box::new(
            HttpWaitStrategy::new("/")
                .with_port(ContainerPort::Tcp(9090))
                .with_response_matcher(|_| true),
        )))
        .with_network(network)
        .with_hostname("auth-backend")
        .start()
        .await
}

async fn create_ferron_container(
    network: &str,
    webroot_dir: &Path,
    config_file: &Path,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let ferron_image = crate::common::build_ferron_image().await?;
    ferron_image
        .with_exposed_port(ContainerPort::Tcp(80))
        .with_wait_for(WaitFor::Http(Box::new(
            HttpWaitStrategy::new("/")
                .with_port(ContainerPort::Tcp(80))
                .with_response_matcher(|_| true),
        )))
        .with_network(network)
        .with_hostname("ferron")
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

struct FAuthTestContext {
    _auth_backend: ContainerAsync<GenericImage>,
    _ferron: ContainerAsync<GenericImage>,
    base_url: String,
    client: reqwest::Client,
    _webroot_dir: tempfile::TempDir,
    _config_file: tempfile::NamedTempFile,
}

impl FAuthTestContext {
    async fn new(test_name: &str, config: &str) -> Self {
        let _ = rustls::crypto::ring::default_provider().install_default();

        #[cfg(unix)]
        nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

        #[cfg(unix)]
        let webroot_dir = crate::common::create_temp_dir();
        #[cfg(unix)]
        let mut config_file = crate::common::create_temp_file();
        #[cfg(not(unix))]
        let webroot_dir = tempfile::tempdir().unwrap();
        #[cfg(not(unix))]
        let mut config_file = tempfile::NamedTempFile::new().unwrap();

        let network = format!("e2e-test-fauth-{}", test_name);

        let auth_backend = create_auth_backend_container(&network).await.unwrap();

        crate::common::write_file(webroot_dir.path().join("index.html"), b"Authenticated!")
            .unwrap();

        config_file
            .as_file_mut()
            .write_all(config.as_bytes())
            .unwrap();

        let ferron = create_ferron_container(&network, webroot_dir.path(), config_file.path())
            .await
            .unwrap();

        let port = ferron
            .get_host_port_ipv4(ContainerPort::Tcp(80))
            .await
            .unwrap();
        let client = reqwest::Client::new();
        let base_url = format!("http://localhost:{}", port);

        tokio::time::sleep(Duration::from_millis(150)).await;

        Self {
            _auth_backend: auth_backend,
            _ferron: ferron,
            base_url,
            client,
            _webroot_dir: webroot_dir,
            _config_file: config_file,
        }
    }
}

#[tokio::test]
async fn test_fauth_success() {
    let ctx = FAuthTestContext::new(
        "success",
        r#"
*:80 {
    auth_to http://auth-backend:9090/auth/ok

    root "/var/www/ferron"
}
"#,
    )
    .await;

    let response = ctx.client.get(&ctx.base_url).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), "Authenticated!");
}

#[tokio::test]
async fn test_fauth_failure_401() {
    let ctx = FAuthTestContext::new(
        "failure-401",
        r#"
*:80 {
    auth_to http://auth-backend:9090/auth/fail

    root "/var/www/ferron"
}
"#,
    )
    .await;

    let response = ctx.client.get(&ctx.base_url).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    assert_eq!(response.text().await.unwrap(), "Unauthorized");
}

#[tokio::test]
async fn test_fauth_failure_403() {
    let ctx = FAuthTestContext::new(
        "failure-403",
        r#"
*:80 {
    auth_to http://auth-backend:9090/auth/forbidden

    root "/var/www/ferron"
}
"#,
    )
    .await;

    let response = ctx.client.get(&ctx.base_url).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
    assert_eq!(response.text().await.unwrap(), "Forbidden");
}

#[tokio::test]
async fn test_fauth_failure_403_with_proxy() {
    let ctx = FAuthTestContext::new(
        "failure-403-proxy",
        r#"
*:80 {
    auth_to http://auth-backend:9090/auth/forbidden

    proxy http://auth-backend:9090
}
"#,
    )
    .await;

    let response = ctx.client.get(&ctx.base_url).send().await.unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
    let body = response.text().await.unwrap();
    assert!(!body.contains("Hello from auth backend!"));
}

#[tokio::test]
async fn test_fauth_failure_500() {
    let ctx = FAuthTestContext::new(
        "failure-500",
        r#"
*:80 {
    auth_to http://auth-backend:9090/auth/500

    root "/var/www/ferron"
}
"#,
    )
    .await;

    let response = ctx.client.get(&ctx.base_url).send().await.unwrap();
    assert_eq!(
        response.status(),
        reqwest::StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(response.text().await.unwrap(), "Internal Error");
}

#[tokio::test]
async fn test_fauth_header_copy() {
    let ctx = FAuthTestContext::new(
        "header-copy",
        r#"
*:80 {
    auth_to http://auth-backend:9090/auth/check-header-added {
        copy X-Custom-Header
    }

    proxy http://auth-backend:9090
}
"#,
    )
    .await;

    let response = ctx
        .client
        .get(&ctx.base_url)
        .header("X-Custom-Header", "added-value")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body = response.text().await.unwrap();
    assert!(body.contains("Hello from auth backend!"));
}

#[tokio::test]
async fn test_fauth_request_header_add() {
    let ctx = FAuthTestContext::new(
        "request-header-add",
        r#"
*:80 {
    auth_to http://auth-backend:9090/auth/check-header-added {
        request_header +X-Custom-Header "added-value"
    }

    root "/var/www/ferron"
}
"#,
    )
    .await;

    let response = ctx.client.get(&ctx.base_url).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), "Authenticated!");
}

#[tokio::test]
async fn test_fauth_request_header_remove() {
    let ctx = FAuthTestContext::new(
        "request-header-remove",
        r#"
*:80 {
    auth_to http://auth-backend:9090/auth/check-header-removed {
        request_header -X-Sensitive-Header
    }

    root "/var/www/ferron"
}
"#,
    )
    .await;

    let response = ctx
        .client
        .get(&ctx.base_url)
        .header("X-Sensitive-Header", "should-be-removed")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), "Authenticated!");
}

#[tokio::test]
async fn test_fauth_request_header_replace() {
    let ctx = FAuthTestContext::new(
        "request-header-replace",
        r#"
*:80 {
    auth_to http://auth-backend:9090/auth/check-header-replaced {
        request_header X-Replace-Me "replaced-value"
    }

    root "/var/www/ferron"
}
"#,
    )
    .await;

    let response = ctx
        .client
        .get(&ctx.base_url)
        .header("X-Replace-Me", "original-value")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), "Authenticated!");
}

#[tokio::test]
async fn test_fauth_connection_refused() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let webroot_dir = crate::common::create_temp_dir();
    #[cfg(unix)]
    let mut config_file = crate::common::create_temp_file();
    #[cfg(not(unix))]
    let webroot_dir = tempfile::tempdir().unwrap();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    let network = "e2e-test-fauth-conn-refused";

    crate::common::write_file(webroot_dir.path().join("index.html"), b"test").unwrap();

    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
    auth_to http://localhost:19999/auth/ok

    root "/var/www/ferron"
}
"#
            .as_bytes(),
        )
        .unwrap();

    let ferron = create_ferron_container(network, webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let port = ferron
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();
    let client = reqwest::Client::new();

    let response = client
        .get(format!("http://localhost:{}", port))
        .send()
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        reqwest::StatusCode::INTERNAL_SERVER_ERROR
    );

    ferron.stop().await.unwrap();
}

#[tokio::test]
async fn test_fauth_nested_url_directive() {
    let ctx = FAuthTestContext::new(
        "nested-url",
        r#"
*:80 {
    auth_to {
        url http://auth-backend:9090/auth/ok
    }

    root "/var/www/ferron"
}
"#,
    )
    .await;

    let response = ctx.client.get(&ctx.base_url).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), "Authenticated!");
}

#[tokio::test]
async fn test_fauth_malformed_response() {
    let ctx = FAuthTestContext::new(
        "malformed-response",
        r#"
*:80 {
    auth_to http://auth-backend:9090/auth/malformed

    root "/var/www/ferron"
}
"#,
    )
    .await;

    let response = ctx.client.get(&ctx.base_url).send().await.unwrap();
    assert_eq!(
        response.status(),
        reqwest::StatusCode::INTERNAL_SERVER_ERROR
    );
}
