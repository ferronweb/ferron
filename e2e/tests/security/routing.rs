use std::io::Write;

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};

use crate::common;

#[tokio::test]
async fn test_routing_bypass() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let webroot_dir = common::create_temp_dir();
    let mut config_file = common::create_temp_file();

    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
    root "/var/www/ferron"
    location /private {
        status 403
    }
}
"#
            .as_bytes(),
        )
        .unwrap();

    common::write_file(webroot_dir.path().join("test.txt"), b"test content").unwrap();
    common::create_dir(webroot_dir.path().join("private")).unwrap();
    common::write_file(
        webroot_dir.path().join("private").join("test.txt"),
        b"private content",
    )
    .unwrap();

    let container = common::create_ferron_container(webroot_dir.path(), config_file.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();

    let client = reqwest::Client::new();

    // Smoke test
    let response = client
        .get(format!("http://localhost:{}/test.txt", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    // Normal case
    let response = client
        .get(format!("http://localhost:{}/private/test.txt", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);

    // Potential bypass edge cases
    let response = client
        .get(format!("http://localhost:{}/private%2Ftest.txt", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
    let response = client
        .get(format!("http://localhost:{}/private%2F..%2Ftest.txt", port))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);

    container.stop().await.unwrap();
}

/// A `proxy` directive on the wildcard host must not leak into a named host.
///
/// Config:
///   *:80 { proxy "http://backend:3000" }
///   isolated.example.com:80 { root "/var/www/ferron" }
///
/// The named host serves static files while other hosts are proxied.
#[tokio::test]
async fn test_wildcard_proxy_does_not_leak_into_named_host() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    let network = "e2e-test-routing-host-isolation";

    let backend_image = common::build_backend_image().await.unwrap();
    let backend = backend_image
        .with_exposed_port(ContainerPort::Tcp(3000))
        .with_wait_for(WaitFor::Http(Box::new(
            HttpWaitStrategy::new("/")
                .with_port(ContainerPort::Tcp(3000))
                .with_response_matcher(|_| true),
        )))
        .with_network(network)
        .with_hostname("backend")
        .with_env_var("BACKEND_NAME", "proxied-backend")
        .start()
        .await
        .unwrap();

    let webroot_dir = common::create_temp_dir();
    let mut config_file = common::create_temp_file();

    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
    proxy "http://backend:3000"
}

isolated.example.com:80 {
    root "/var/www/ferron"
}
"#
            .as_bytes(),
        )
        .unwrap();
    config_file.flush().unwrap();

    common::write_file(webroot_dir.path().join("index.html"), b"static content").unwrap();

    let ferron_image = common::build_ferron_image().await.unwrap();
    let container: ContainerAsync<GenericImage> = ferron_image
        .with_exposed_port(ContainerPort::Tcp(80))
        .with_wait_for(WaitFor::Http(Box::new(
            HttpWaitStrategy::new("/")
                .with_port(ContainerPort::Tcp(80))
                .with_response_matcher(|_| true),
        )))
        .with_network(network)
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
    let client = reqwest::Client::new();

    // The named host serves static files instead of proxying.
    let response = client
        .get(format!("http://localhost:{}/index.html", port))
        .header("Host", "isolated.example.com")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), "static content");

    // Other hosts are still proxied through the wildcard host.
    let response = client
        .get(format!("http://localhost:{}/whoami", port))
        .header("Host", "other.example.com")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.text().await.unwrap(), "proxied-backend");

    backend.stop().await.unwrap();
    container.stop().await.unwrap();
}
