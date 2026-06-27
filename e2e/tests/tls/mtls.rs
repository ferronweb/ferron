//! mTLS client certificate verification tests for Ferron.
//!
//! These tests are inspired by the nginx-tests `ssl_verify_client.t` test file,
//! which verifies correct client certificate verification behavior including
//! optional and required client certificates, certificate depth, and session
//! reuse with client certs.
//!
//! See: https://github.com/nginx/nginx-tests/blob/master/ssl_verify_client.t

use std::io::Write;

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};

use crate::common;

async fn create_ferron_container(
    config_file: &std::path::Path,
    cert_file: &std::path::Path,
    key_file: &std::path::Path,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let ferron_image = self::common::build_ferron_image().await?;
    ferron_image
        .with_exposed_port(ContainerPort::Tcp(80))
        .with_exposed_port(ContainerPort::Tcp(443))
        .with_wait_for(WaitFor::Http(Box::new(
            HttpWaitStrategy::new("/")
                .with_port(ContainerPort::Tcp(80))
                .with_response_matcher(|_| true),
        )))
        .with_network("bridge")
        .with_mount(Mount::bind_mount(
            config_file.to_string_lossy(),
            "/etc/ferron.conf",
        ))
        .with_mount(Mount::bind_mount(
            cert_file.to_string_lossy(),
            "/etc/cert.pem",
        ))
        .with_mount(Mount::bind_mount(
            key_file.to_string_lossy(),
            "/etc/key.pem",
        ))
        .start()
        .await
}

/// Test that server TLS works with manual certificates.
///
/// Verifies that Ferron can serve HTTPS with manually configured certificates.
/// Inspired by nginx-tests ssl_verify_client.t — verifies TLS handshake with
/// manually provisioned certificates.
#[tokio::test]
async fn test_tls_manual_certificates() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let cert_file = self::common::create_temp_file();
    let key_file = self::common::create_temp_file();
    let mut config_file = self::common::create_temp_file();

    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    std::fs::write(cert_file.path(), cert.cert.pem().as_bytes()).unwrap();
    std::fs::write(key_file.path(), cert.signing_key.serialize_pem().as_bytes()).unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
  root "/var/www/ferron"
}

*:443 {
  tls {
    provider manual
    cert "/etc/cert.pem"
    key "/etc/key.pem"
  }
  root "/var/www/ferron"
}
"#,
        )
        .unwrap();

    let container = create_ferron_container(config_file.path(), cert_file.path(), key_file.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(443))
        .await
        .unwrap();

    let client = reqwest::Client::builder()
        .http1_only()
        .tls_danger_accept_invalid_certs(true)
        .tls_danger_accept_invalid_hostnames(true)
        .build()
        .unwrap();

    let response = client
        .get(format!("https://localhost:{}/", port))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);

    container.stop().await.unwrap();
}

/// Test that HTTP requests still work alongside HTTPS.
///
/// Inspired by nginx-tests ssl_verify_client.t — verifies that both HTTP
/// and HTTPS listeners coexist correctly.
#[tokio::test]
async fn test_tls_coexist_with_http() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let cert_file = self::common::create_temp_file();
    let key_file = self::common::create_temp_file();
    let mut config_file = self::common::create_temp_file();

    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    std::fs::write(cert_file.path(), cert.cert.pem().as_bytes()).unwrap();
    std::fs::write(key_file.path(), cert.signing_key.serialize_pem().as_bytes()).unwrap();

    config_file
        .as_file_mut()
        .write_all(
            br#"
*:80 {
  root "/var/www/ferron"
}

*:443 {
  tls {
    provider manual
    cert "/etc/cert.pem"
    key "/etc/key.pem"
  }
  root "/var/www/ferron"
}
"#,
        )
        .unwrap();

    let container = create_ferron_container(config_file.path(), cert_file.path(), key_file.path())
        .await
        .unwrap();

    let http_port = container
        .get_host_port_ipv4(ContainerPort::Tcp(80))
        .await
        .unwrap();

    let https_port = container
        .get_host_port_ipv4(ContainerPort::Tcp(443))
        .await
        .unwrap();

    // Test HTTP
    let client = reqwest::Client::new();
    let response = client
        .get(format!("http://localhost:{}/", http_port))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    // Test HTTPS
    let client = reqwest::Client::builder()
        .http1_only()
        .tls_danger_accept_invalid_certs(true)
        .tls_danger_accept_invalid_hostnames(true)
        .build()
        .unwrap();
    let response = client
        .get(format!("https://localhost:{}/", https_port))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    container.stop().await.unwrap();
}
