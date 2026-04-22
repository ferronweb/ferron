use std::io::Write;
use std::path::Path;
use std::time::Duration;
#[cfg(unix)]
use std::{fs::Permissions, os::unix::fs::PermissionsExt};

use rustls::client::danger::{ServerCertVerified, ServerCertVerifier};
use rustls_pki_types::{CertificateDer, ServerName, UnixTime};
use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor},
    runners::AsyncRunner,
};

mod common;

/// A server certificate verifier that always returns a successful verification.
#[derive(Debug)]
pub struct NoServerVerifier;

impl Default for NoServerVerifier {
    fn default() -> Self {
        Self::new()
    }
}

impl NoServerVerifier {
    pub fn new() -> Self {
        Self
    }
}

impl ServerCertVerifier for NoServerVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        use rustls::SignatureScheme::*;
        vec![
            ECDSA_NISTP384_SHA384,
            ECDSA_NISTP256_SHA256,
            ED25519,
            RSA_PSS_SHA512,
            RSA_PSS_SHA384,
            RSA_PSS_SHA256,
            RSA_PKCS1_SHA512,
            RSA_PKCS1_SHA384,
            RSA_PKCS1_SHA256,
        ]
    }
}

async fn create_pebble_container(
    network: &str,
    config_file: &Path,
    cert_dir: &Path,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    GenericImage::new("ghcr.io/letsencrypt/pebble", "latest")
        .with_exposed_port(ContainerPort::Tcp(14000))
        // Wait for Pebble to be ready.
        // Since we can't easily do a secure HTTP check against self-signed cert in wait strategy,
        // we'll wait some time.
        .with_wait_for(WaitFor::seconds(5))
        .with_network(network)
        .with_hostname("pebble")
        .with_mount(Mount::bind_mount(
            config_file.to_string_lossy().to_string(),
            "/etc/pebble-config.json",
        ))
        .with_mount(Mount::bind_mount(
            cert_dir.to_string_lossy().to_string(),
            "/etc/certs",
        ))
        .with_cmd(vec!["-config", "/etc/pebble-config.json"])
        .start()
        .await
}

async fn create_ferron_container(
    network: &str,
    webroot_dir: &Path,
    config_file: &Path,
    cache_dir: &Path,
    alias: &str,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let ferron_image = self::common::build_ferron_image().await?;
    ferron_image
        .with_exposed_port(ContainerPort::Tcp(80))
        .with_exposed_port(ContainerPort::Tcp(443))
        // No wait strategy here because we want to test availability which might take time due to ACME
        .with_network(network)
        .with_hostname(alias)
        .with_mount(Mount::bind_mount(
            webroot_dir.to_string_lossy().to_string(),
            "/var/www/ferron",
        ))
        .with_mount(Mount::bind_mount(
            config_file.to_string_lossy().to_string(),
            "/etc/ferron.conf",
        ))
        .with_mount(Mount::bind_mount(
            cache_dir.to_string_lossy().to_string(),
            "/var/cache/ferron-acme",
        ))
        .start()
        .await
}

async fn test_acme_common(challenge_type: &str, hostname: &str, extra_host_config: &str) {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    // Prepare directories
    #[cfg(unix)]
    let webroot_dir = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o777))
        .tempdir()
        .unwrap();
    #[cfg(unix)]
    let cert_dir = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o777))
        .tempdir()
        .unwrap();
    #[cfg(unix)]
    let cache_dir = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o777))
        .tempdir()
        .unwrap();

    #[cfg(not(unix))]
    let webroot_dir = tempfile::tempdir().unwrap();
    #[cfg(not(unix))]
    let cert_dir = tempfile::tempdir().unwrap();
    #[cfg(not(unix))]
    let cache_dir = tempfile::tempdir().unwrap();

    // Prepare config files
    #[cfg(unix)]
    let mut ferron_config = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();
    #[cfg(unix)]
    let mut pebble_config = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();

    #[cfg(not(unix))]
    let mut ferron_config = tempfile::NamedTempFile::new().unwrap();
    #[cfg(not(unix))]
    let mut pebble_config = tempfile::NamedTempFile::new().unwrap();

    // 1. Generate CA for Pebble
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    std::fs::write(cert_dir.path().join("ca.crt"), cert.cert.pem()).unwrap();
    std::fs::write(
        cert_dir.path().join("ca.key"),
        cert.signing_key.serialize_pem(),
    )
    .unwrap();

    // 2. Write Pebble config
    pebble_config
        .as_file_mut()
        .write_all(
            br#"{
  "pebble": {
    "listenAddress": "0.0.0.0:14000",
    "managementListenAddress": "0.0.0.0:15000",
    "certificate": "/etc/certs/ca.crt",
    "privateKey": "/etc/certs/ca.key",
    "httpPort": 80,
    "tlsPort": 443,
    "externalAccountBindingRequired": false,
    "domainBlocklist": [],
    "retryAfter": {
      "authz": 3,
      "order": 5
    }
  }
}"#,
        )
        .unwrap();

    // 3. Write Ferron config
    ferron_config
        .as_file_mut()
        .write_all(
            format!(
                r#"
{} {{
  tls {{
    provider "acme"
    cache "/var/cache/ferron-acme"
    directory "https://pebble:14000/dir"
    no_verification true
    challenge "{}"
    {}
  }}
  root "/var/www/ferron"
}}
"#,
                hostname, challenge_type, extra_host_config
            )
            .as_bytes(),
        )
        .unwrap();

    self::common::write_file(
        webroot_dir.path().join("index.html"),
        b"Ferron is installed successfully!",
    )
    .unwrap();

    let network = format!("e2e-test-ferronacme-{}", hostname);

    // 4. Start Pebble
    let _pebble = create_pebble_container(&network, pebble_config.path(), cert_dir.path())
        .await
        .unwrap();

    // 5. Start Ferron
    let ferron = create_ferron_container(
        &network,
        webroot_dir.path(),
        ferron_config.path(),
        cache_dir.path(),
        hostname,
    )
    .await
    .unwrap();

    // 6. Wait for certificate issuance and verify
    let port = ferron
        .get_host_port_ipv4(ContainerPort::Tcp(443))
        .await
        .unwrap();

    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .resolve(
            hostname,
            std::net::SocketAddr::new(
                std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1)),
                port,
            ),
        )
        .build()
        .unwrap();

    // Poll until success
    let mut success = false;
    for _ in 0..60 {
        // 60 seconds should be enough
        if let Ok(response) = client
            .get(format!("https://{}:{}/", hostname, port))
            .send()
            .await
        {
            if response.status().is_success() {
                success = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    assert!(
        success,
        "Failed to connect to Ferron via HTTPS with auto-obtained certificate"
    );
}

#[tokio::test]
async fn test_acme_http01() {
    test_acme_common("http-01", "ferron-http01", "").await;
}

#[tokio::test]
async fn test_acme_tls_alpn_01() {
    test_acme_common("tls-alpn-01", "ferron-tlsalpn01", "").await;
}

#[tokio::test]
async fn test_acme_broken_cache() {
    // We attempt to use a directory that is likely not writable by the user running Ferron in the container.
    // Since Ferron typically runs as a non-root user (e.g. nobody or ferron), /root/cache should be inaccessible.
    test_acme_common("http-01", "ferron-brokencache", "cache \"/root/cache\"").await;
}

#[tokio::test]
async fn test_acme_ondemand() {
    test_acme_common("tls-alpn-01", "ferron-ondemand", "on_demand").await;
}

#[tokio::test]
async fn test_acme_http01_ondemand() {
    test_acme_common("http-01", "ferron-http01-ondemand", "on_demand").await;
}
