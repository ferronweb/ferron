use std::path::Path;
#[cfg(unix)]
use std::{fs::Permissions, os::unix::fs::PermissionsExt};
use std::{io::Write, sync::Arc};

use rustls::DigitallySignedStruct;
use rustls::SignatureScheme::{self, *};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls_pki_types::{CertificateDer, ServerName, UnixTime};
use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
    runners::AsyncRunner,
};

use crate::common::build_ocsp_forged_image;

mod common;

/// A server certificate verifier that records the OCSP response passed by the server.
#[derive(Debug)]
pub struct OcspRecorder {
    pub recorded: Arc<std::sync::Mutex<Option<Vec<u8>>>>,
}

impl OcspRecorder {
    pub fn new(buf: Arc<std::sync::Mutex<Option<Vec<u8>>>>) -> Self {
        Self { recorded: buf }
    }
}

impl Default for OcspRecorder {
    fn default() -> Self {
        Self::new(Arc::new(std::sync::Mutex::new(None)))
    }
}

impl ServerCertVerifier for OcspRecorder {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if !ocsp_response.is_empty() {
            let mut lock = self.recorded.lock().unwrap();
            *lock = Some(ocsp_response.to_vec());
        }
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

async fn create_ocsp_container(
    network: &str,
    cert_dir: &Path,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let ocsp_image = build_ocsp_forged_image().await?;
    ocsp_image
        .with_exposed_port(ContainerPort::Tcp(5000))
        .with_wait_for(WaitFor::Http(Box::new(
            HttpWaitStrategy::new("/ready")
                .with_port(ContainerPort::Tcp(5000))
                .with_response_matcher(|_| true),
        )))
        .with_network(network)
        .with_hostname("ocsp")
        .with_mount(Mount::bind_mount(
            cert_dir.to_string_lossy().to_string(),
            "/certs",
        ))
        .start()
        .await
}

async fn create_ferron_container(
    network: &str,
    config_file: &Path,
    cert_file: &Path,
    key_file: &Path,
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
        .with_network(network)
        .with_mount(Mount::bind_mount(
            config_file.to_string_lossy().to_string(),
            "/etc/ferron.conf",
        ))
        .with_mount(Mount::bind_mount(
            cert_file.to_string_lossy().to_string(),
            "/etc/cert.pem",
        ))
        .with_mount(Mount::bind_mount(
            key_file.to_string_lossy().to_string(),
            "/etc/key.pem",
        ))
        .start()
        .await
}

#[tokio::test]
async fn test_ocsp_stapling_tcp() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[cfg(unix)]
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

    #[cfg(unix)]
    let cert_dir = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o777))
        .tempdir()
        .unwrap();
    #[cfg(not(unix))]
    let cert_dir = tempfile::tempdir().unwrap();

    #[cfg(unix)]
    let mut config_file = tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o666))
        .tempfile()
        .unwrap();
    #[cfg(not(unix))]
    let mut config_file = tempfile::NamedTempFile::new().unwrap();

    // Write minimal Ferron config (TLS manual, no QUIC needed)
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
  http { protocols "h1" "h2" }
  root "/var/www/ferron"
}
"#,
        )
        .unwrap();

    let network = "e2e-test-ocsp-tcp".to_string();

    // Start OCSP responder container
    let _ocsp = create_ocsp_container(&network, cert_dir.path())
        .await
        .unwrap();

    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    let server_cert = cert_dir.path().join("server.crt");
    let server_key = cert_dir.path().join("server.key");
    assert!(server_cert.exists());
    assert!(server_key.exists());

    let ferron = create_ferron_container(&network, config_file.path(), &server_cert, &server_key)
        .await
        .unwrap();

    let port = ferron
        .get_host_port_ipv4(ContainerPort::Tcp(443))
        .await
        .unwrap();

    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // CA file (generated by responder) should exist for completeness
    let ca_file = cert_dir.path().join("ca.crt");
    assert!(ca_file.exists(), "ca.crt should exist for verification");

    // Use rustls TLS client to capture stapled OCSP response via our OcspRecorder verifier.
    let recorder = Arc::new(std::sync::Mutex::new(None));
    let verifier = Arc::new(OcspRecorder::new(recorder.clone()));
    let client_config = Arc::new(
        rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(verifier)
            .with_no_client_auth(),
    );

    let mut saw_staple = false;
    for _ in 0..4 {
        let client_cfg = client_config.clone();
        let port_copy = port;
        // Perform blocking TCP+TLS handshake in a spawn_blocking to avoid blocking the async runtime
        // This could be done with `tokio_rustls` though, but it's not strictly needed,
        // since it's test code after all...
        let _res = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            use rustls::StreamOwned;
            use rustls::client::ClientConnection;
            use std::io::{Read, Write};
            use std::net::TcpStream;

            let server_name = ServerName::try_from("localhost").map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("invalid dnsname: {}", e),
                )
            })?;

            let tcp = TcpStream::connect(("127.0.0.1", port_copy))?;
            tcp.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
            tcp.set_write_timeout(Some(std::time::Duration::from_secs(5)))?;

            let conn = ClientConnection::new(client_cfg, server_name).map_err(|e| {
                std::io::Error::new(std::io::ErrorKind::Other, format!("tls conn error: {}", e))
            })?;
            let mut stream = StreamOwned::new(conn, tcp);

            // Send a simple HTTP/1.0 request to trigger handshake and receive response
            stream.write_all(b"GET / HTTP/1.0\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;

            let mut buf = Vec::new();
            let res = stream.read_to_end(&mut buf);
            if buf.is_empty() {
                res?;
            }
            Ok(())
        })
        .await
        .expect("spawn_blocking failed");

        // Check if verifier recorded OCSP bytes
        if let Some(bytes) = recorder.lock().unwrap().clone() {
            if !bytes.is_empty() {
                saw_staple = true;
                break;
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    assert!(
        !saw_staple,
        "Expected OCSP stapled response rejected due to forgery"
    );

    ferron.stop().await.unwrap();
}
