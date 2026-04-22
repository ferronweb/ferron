use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
#[cfg(unix)]
use std::{fs::Permissions, os::unix::fs::PermissionsExt};
use std::{io::Write, path::Path, sync::Arc};

use h3::error::StreamError;
use rustls::DigitallySignedStruct;
use rustls::SignatureScheme::{self, *};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls_pki_types::{CertificateDer, ServerName, UnixTime};
use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
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
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        // Extend the list when necessary
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

async fn create_ferron_container(
    config_file: &Path,
    cert_file: &Path,
    key_file: &Path,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let ferron_image = self::common::build_ferron_image().await?;
    ferron_image
        .with_exposed_port(ContainerPort::Tcp(80))
        .with_exposed_port(ContainerPort::Tcp(443))
        .with_exposed_port(ContainerPort::Udp(443)) // QUIC
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

#[tokio::test]
async fn test_tls_http_1() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();
    #[cfg(unix)]
    let mut cert_file = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();
    #[cfg(unix)]
    let mut key_file = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();
    #[cfg(not(unix))]
    let mut cert_file = tempfile::NamedTempFile::new().unwrap();
    #[cfg(not(unix))]
    let mut key_file = tempfile::NamedTempFile::new().unwrap();

    // Generate self-signed TLS certificate using rcgen
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    cert_file
        .as_file_mut()
        .write_all(cert.cert.pem().as_bytes())
        .unwrap();
    key_file
        .as_file_mut()
        .write_all(cert.signing_key.serialize_pem().as_bytes())
        .unwrap();

    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
  root "/var/www/ferron" # Serve "Ferron is installed successfully" page
}

*:443 {
  tls {
    provider "manual"
    cert "/etc/cert.pem"
    key "/etc/key.pem"
  }
  root "/var/www/ferron" # Serve "Ferron is installed successfully" page
}
"#
            .as_bytes(),
        )
        .unwrap();

    let container = create_ferron_container(config_file.path(), cert_file.path(), key_file.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(443))
        .await
        .unwrap();
    let client = reqwest::ClientBuilder::new()
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

#[tokio::test]
async fn test_tls_http_2() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();
    #[cfg(unix)]
    let mut cert_file = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();
    #[cfg(unix)]
    let mut key_file = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();
    #[cfg(not(unix))]
    let mut cert_file = tempfile::NamedTempFile::new().unwrap();
    #[cfg(not(unix))]
    let mut key_file = tempfile::NamedTempFile::new().unwrap();

    // Generate self-signed TLS certificate using rcgen
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    cert_file
        .as_file_mut()
        .write_all(cert.cert.pem().as_bytes())
        .unwrap();
    key_file
        .as_file_mut()
        .write_all(cert.signing_key.serialize_pem().as_bytes())
        .unwrap();

    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
  root "/var/www/ferron" # Serve "Ferron is installed successfully" page
}

*:443 {
  tls {
    provider "manual"
    cert "/etc/cert.pem"
    key "/etc/key.pem"
  }
  root "/var/www/ferron" # Serve "Ferron is installed successfully" page
}
"#
            .as_bytes(),
        )
        .unwrap();

    let container = create_ferron_container(config_file.path(), cert_file.path(), key_file.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(443))
        .await
        .unwrap();
    let client = reqwest::ClientBuilder::new()
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
    assert_eq!(response.version(), reqwest::Version::HTTP_2);

    container.stop().await.unwrap();
}

#[tokio::test]
async fn test_tls_http_3() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let mut config_file = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();
    #[cfg(unix)]
    let mut cert_file = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();
    #[cfg(unix)]
    let mut key_file = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();
    #[cfg(not(unix))]
    let mut cert_file = tempfile::NamedTempFile::new().unwrap();
    #[cfg(not(unix))]
    let mut key_file = tempfile::NamedTempFile::new().unwrap();

    // Generate self-signed TLS certificate using rcgen
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    cert_file
        .as_file_mut()
        .write_all(cert.cert.pem().as_bytes())
        .unwrap();
    key_file
        .as_file_mut()
        .write_all(cert.signing_key.serialize_pem().as_bytes())
        .unwrap();

    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
  root "/var/www/ferron" # Serve "Ferron is installed successfully" page
}

*:443 {
  tls {
    provider "manual"
    cert "/etc/cert.pem"
    key "/etc/key.pem"
  }
  http {
    protocols "h1" "h2" "h3"
  }
  root "/var/www/ferron" # Serve "Ferron is installed successfully" page
}
"#
            .as_bytes(),
        )
        .unwrap();

    let container = create_ferron_container(config_file.path(), cert_file.path(), key_file.path())
        .await
        .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Udp(443))
        .await
        .unwrap(); // QUIC uses UDP

    let mut tls_config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoServerVerifier::new()))
        .with_no_client_auth();

    tls_config.enable_early_data = true;
    tls_config.alpn_protocols = vec![b"h3".into()];

    let mut client_endpoint =
        h3_quinn::quinn::Endpoint::client("0.0.0.0:0".parse().unwrap()).unwrap();

    let client_config = quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(tls_config).unwrap(),
    ));
    client_endpoint.set_default_client_config(client_config);

    let conn = client_endpoint
        .connect(
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), port)),
            "localhost",
        )
        .unwrap()
        .await
        .unwrap();

    let quinn_conn = h3_quinn::Connection::new(conn);

    let (mut driver, mut send_request) = h3::client::new(quinn_conn).await.unwrap();

    let drive = async move {
        return Err::<(), h3::error::ConnectionError>(
            std::future::poll_fn(|cx| driver.poll_close(cx)).await,
        );
    };

    let request = async move {
        let req = http::Request::builder()
            .uri(format!("https://localhost:{}/", port))
            .body(())
            .unwrap();
        let mut stream = send_request.send_request(req).await?;
        stream.finish().await?;

        let resp = stream.recv_response().await?;

        assert_eq!(resp.status(), http::StatusCode::OK);

        Ok::<_, StreamError>(())
    };

    let (req_res, drive_res) = tokio::join!(request, drive);
    req_res.unwrap();
    if let Err(e) = &drive_res {
        if !e.is_h3_no_error() {
            drive_res.unwrap();
        }
    }

    client_endpoint.wait_idle().await;

    container.stop().await.unwrap();
}
