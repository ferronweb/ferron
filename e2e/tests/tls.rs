use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::{
    io::Write,
    path::Path,
    sync::{Arc, Mutex},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_rustls::TlsConnector;

use aes::cipher::{BlockModeDecrypt, KeyIvInit, block_padding::Pkcs7};
use cbc::Decryptor;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use h3::error::StreamError;

use rustls::DigitallySignedStruct;
use rustls::NamedGroup;
use rustls::SignatureScheme::{self, *};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::{Tls12ClientSessionValue, Tls13ClientSessionValue};
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

    let mut config_file = common::create_temp_file();
    let mut cert_file = common::create_temp_file();
    let mut key_file = common::create_temp_file();

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
    provider manual
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

    let mut config_file = common::create_temp_file();
    let mut cert_file = common::create_temp_file();
    let mut key_file = common::create_temp_file();

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
    provider manual
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

    let mut config_file = common::create_temp_file();
    let mut cert_file = common::create_temp_file();
    let mut key_file = common::create_temp_file();

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
    provider manual
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
        Err::<(), h3::error::ConnectionError>(
            std::future::poll_fn(|cx| driver.poll_close(cx)).await,
        )
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
    if let Err(e) = &drive_res
        && !e.is_h3_no_error()
    {
        drive_res.unwrap();
    }

    client_endpoint.wait_idle().await;

    container.stop().await.unwrap();
}

// ============================================================================
// Infrastructure helpers for TLS session ticket testing
// ============================================================================

/// Generate a ticket key file with the specified number of keys.
///
/// Returns the path to the generated key file.
fn generate_ticket_key_file(num_keys: usize) -> tempfile::NamedTempFile {
    use std::io::Write;

    let mut file = tempfile::NamedTempFile::new().expect("Failed to create temp key file");

    // Generate random key records (80 bytes each: 16-byte name + 32-byte AES + 32-byte HMAC)
    for _ in 0..num_keys.min(5) {
        let mut key = [0u8; 80];
        getrandom::fill(&mut key).expect("Failed to generate random bytes for ticket key");
        file.write_all(&key)
            .expect("Failed to write ticket key to file");
    }
    file.flush().expect("Failed to flush key file");

    file
}

/// Create an enhanced Ferron container with ticket key support.
async fn create_ferron_container_with_ticket_keys(
    config_file: &Path,
    cert_file: &Path,
    key_file: &Path,
    ticket_key_file: &Path,
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
        .with_mount(Mount::bind_mount(
            ticket_key_file.to_string_lossy(),
            "/etc/session_tickets.keys",
        ))
        .start()
        .await
}

/// Build a rustls ClientConfig that can reuse sessions from a stored session ID.
fn build_session_resumption_client() -> reqwest::Client {
    reqwest::ClientBuilder::new()
        .http1_only()
        .tls_danger_accept_invalid_certs(true)
        .tls_danger_accept_invalid_hostnames(true)
        .build()
        .unwrap()
}

// ============================================================================
// E2E Tests for TLS Session Tickets
// ============================================================================

#[tokio::test]
async fn test_tls_session_tickets_static() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Generate TLS certificate
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();

    let mut config_file = common::create_temp_file();
    let mut cert_file = common::create_temp_file();
    let mut key_file = common::create_temp_file();

    // Write certificate and key
    cert_file
        .as_file_mut()
        .write_all(cert.cert.pem().as_bytes())
        .unwrap();
    key_file
        .as_file_mut()
        .write_all(cert.signing_key.serialize_pem().as_bytes())
        .unwrap();

    // Generate ticket key file
    let ticket_key_file = generate_ticket_key_file(1);

    // Create Ferron configuration with auto-rotating ticket keys
    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
  root "/var/www/ferron"
}

*:443 {
  tls {
    provider manual
    cert "/etc/cert.pem"
    key "/etc/key.pem"
    ticket_keys {
      file "/etc/session_tickets.keys"
      auto_rotate
      rotation_interval "1h"
      max_keys 3
    }
  }
  root "/var/www/ferron"
}
"#
            .as_bytes(),
        )
        .unwrap();

    let container = create_ferron_container_with_ticket_keys(
        config_file.path(),
        cert_file.path(),
        key_file.path(),
        ticket_key_file.path(),
    )
    .await
    .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(443))
        .await
        .unwrap();

    let client = build_session_resumption_client();

    // First connection - establish session
    let response = client
        .get(format!("https://localhost:{}/", port))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);

    // Second connection - should also succeed with ticket present
    let response = client
        .get(format!("https://localhost:{}/", port))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);

    container.stop().await.unwrap();
}

#[tokio::test]
async fn test_tls_session_tickets_multiple_keys() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Generate TLS certificate
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();

    let mut config_file = common::create_temp_file();
    let mut cert_file = common::create_temp_file();
    let mut key_file = common::create_temp_file();

    // Write certificate and key
    cert_file
        .as_file_mut()
        .write_all(cert.cert.pem().as_bytes())
        .unwrap();
    key_file
        .as_file_mut()
        .write_all(cert.signing_key.serialize_pem().as_bytes())
        .unwrap();

    // Generate ticket key file with 3 keys
    let ticket_key_file = generate_ticket_key_file(3);

    // Create Ferron configuration with static ticket keys
    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
  root "/var/www/ferron"
}

*:443 {
  tls {
    provider manual
    cert "/etc/cert.pem"
    key "/etc/key.pem"
    ticket_keys {
      file "/etc/session_tickets.keys"
    }
  }
  root "/var/www/ferron"
}
"#
            .as_bytes(),
        )
        .unwrap();

    let container = create_ferron_container_with_ticket_keys(
        config_file.path(),
        cert_file.path(),
        key_file.path(),
        ticket_key_file.path(),
    )
    .await
    .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(443))
        .await
        .unwrap();

    let client = build_session_resumption_client();

    // Make multiple requests to ensure server handles multiple keys properly
    for i in 0..5 {
        let response = client
            .get(format!("https://localhost:{}/", port))
            .send()
            .await
            .unwrap_or_else(|_| panic!("Request {} failed", i));

        assert_eq!(response.status(), reqwest::StatusCode::OK);
    }

    container.stop().await.unwrap();
}

#[tokio::test]
async fn test_tls_session_tickets_auto_rotate_enabled() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Generate TLS certificate
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();

    let mut config_file = common::create_temp_file();
    let mut cert_file = common::create_temp_file();
    let mut key_file = common::create_temp_file();

    // Write certificate and key
    cert_file
        .as_file_mut()
        .write_all(cert.cert.pem().as_bytes())
        .unwrap();
    key_file
        .as_file_mut()
        .write_all(cert.signing_key.serialize_pem().as_bytes())
        .unwrap();

    // Create Ferron configuration with auto-rotation enabled (using short intervals for testing)
    config_file
        .as_file_mut()
        .write_all(
            r#"
*:80 {
  root "/var/www/ferron"
}

*:443 {
  tls {
    provider manual
    cert "/etc/cert.pem"
    key "/etc/key.pem"
    ticket_keys {
      file "/tmp/session_tickets.keys"
      auto_rotate
      rotation_interval "2s"
      max_keys 3
    }
  }
  root "/var/www/ferron"
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

    let client = build_session_resumption_client();

    // Make initial requests to ensure ticket generation works
    for i in 0..3 {
        let response = client
            .get(format!("https://localhost:{}/", port))
            .send()
            .await
            .unwrap_or_else(|_| panic!("Request {} failed", i));

        assert_eq!(response.status(), reqwest::StatusCode::OK);
    }

    // Wait for key rotation to occur (rotation_interval = 2s)
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    // Make more requests after rotation to ensure tickets still work
    for i in 0..3 {
        let response = client
            .get(format!("https://localhost:{}/", port))
            .send()
            .await
            .unwrap_or_else(|_| panic!("Post-rotation request {} failed", i));

        assert_eq!(response.status(), reqwest::StatusCode::OK);
    }

    container.stop().await.unwrap();
}

#[derive(Debug)]
struct CapturingSessionStore {
    sessions: Arc<Mutex<std::collections::HashMap<ServerName<'static>, Tls13ClientSessionValue>>>,
}

impl rustls::client::ClientSessionStore for CapturingSessionStore {
    fn set_kx_hint(&self, _: ServerName<'static>, _: NamedGroup) {}
    fn kx_hint(&self, _: &ServerName<'_>) -> Option<NamedGroup> {
        None
    }

    fn set_tls12_session(&self, _: ServerName<'static>, _: Tls12ClientSessionValue) {}
    fn tls12_session(&self, _: &ServerName<'_>) -> Option<Tls12ClientSessionValue> {
        None
    }
    fn remove_tls12_session(&self, _: &ServerName<'static>) {}

    fn insert_tls13_ticket(&self, key: ServerName<'static>, value: Tls13ClientSessionValue) {
        self.sessions.lock().unwrap().insert(key, value);
    }
    fn take_tls13_ticket(&self, key: &ServerName<'static>) -> Option<Tls13ClientSessionValue> {
        self.sessions.lock().unwrap().remove(key)
    }
}

fn decrypt_ticket(ticket: &[u8], aes_key: &[u8; 32], hmac_key: &[u8; 32]) -> Option<Vec<u8>> {
    if ticket.len() < 16 + 32 {
        return None;
    }
    let iv: &[u8; 16] = &ticket[0..16].try_into().ok()?;
    let ciphertext = &ticket[16..ticket.len() - 32];
    let hmac_val = &ticket[ticket.len() - 32..];

    // Verify HMAC
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(hmac_key).ok()?;
    mac.update(&ticket[..ticket.len() - 32]);
    mac.verify_slice(hmac_val).ok()?;

    // Decrypt AES-256-CBC
    let decryptor = Decryptor::<aes::Aes256>::new(aes_key.into(), iv.into());
    let mut buf = ciphertext.to_vec();
    let decrypted = decryptor.decrypt_padded::<Pkcs7>(&mut buf).ok()?;
    Some(decrypted.to_vec())
}

#[tokio::test]
async fn test_tls_session_ticket_decryption_and_resumption() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Generate TLS certificate
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();

    let cert_file = common::create_temp_file();
    let key_file = common::create_temp_file();
    let config_file = common::create_temp_file();
    let ticket_key_file = common::create_temp_file();

    // Generate a known ticket key
    let mut ticket_key_record = [0u8; 80];
    getrandom::fill(&mut ticket_key_record).unwrap();
    std::fs::write(ticket_key_file.path(), ticket_key_record).unwrap();

    let aes_key: [u8; 32] = ticket_key_record[16..48].try_into().unwrap();
    let hmac_key: [u8; 32] = ticket_key_record[48..80].try_into().unwrap();

    // Write cert and key
    std::fs::write(cert_file.path(), cert.cert.pem().as_bytes()).unwrap();
    std::fs::write(key_file.path(), cert.signing_key.serialize_pem().as_bytes()).unwrap();

    std::fs::write(
        config_file.path(),
        r#"
*:80 {
  root "/var/www/ferron"
}

*:443 {
  tls {
    provider manual
    cert "/etc/cert.pem"
    key "/etc/key.pem"
    ticket_keys {
      file "/etc/session_tickets.keys"
    }
  }
  root "/var/www/ferron"
}
"#,
    )
    .unwrap();

    let container = create_ferron_container_with_ticket_keys(
        config_file.path(),
        cert_file.path(),
        key_file.path(),
        ticket_key_file.path(),
    )
    .await
    .unwrap();

    let port = container
        .get_host_port_ipv4(ContainerPort::Tcp(443))
        .await
        .unwrap();

    // Create a capturing session store
    let sessions = Arc::new(Mutex::new(std::collections::HashMap::new()));
    let store = Arc::new(CapturingSessionStore {
        sessions: sessions.clone(),
    });

    let mut client_config = rustls::ClientConfig::builder()
        .with_root_certificates(rustls::RootCertStore::empty())
        .with_no_client_auth();

    client_config
        .dangerous()
        .set_certificate_verifier(Arc::new(NoServerVerifier::new()));
    client_config.resumption = rustls::client::Resumption::store(store.clone());
    let client_config = Arc::new(client_config);

    // 1. First connection to get a ticket
    let connector = TlsConnector::from(client_config.clone());
    let stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port))
        .await
        .unwrap();
    let domain = ServerName::try_from("localhost").unwrap();
    let mut tls_stream = connector.connect(domain.clone(), stream).await.unwrap();

    // Trigger NewSessionTicket by reading/writing
    tls_stream
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut buf = Vec::new();
    let _ = tls_stream.read_to_end(&mut buf).await;
    drop(tls_stream);

    // Wait for the session to be stored
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    {
        let captured_sessions = sessions.lock().unwrap();
        assert!(!captured_sessions.is_empty(), "No session was captured");

        // Hack: use Debug output to extract the ticket bytes since the field is private
        let mut decrypted_successfully = false;
        for session_value in captured_sessions.values() {
            let debug_str = format!("{:?}", session_value);
            // Look for "ticket: <hex>" in debug output
            if let Some(pos) = debug_str.find("ticket: ") {
                let ticket_hex_start = pos + 8;
                let ticket_hex_end = debug_str[ticket_hex_start..]
                    .find(|c: char| !c.is_ascii_hexdigit())
                    .map(|i| ticket_hex_start + i)
                    .unwrap_or(debug_str.len());
                let ticket_hex = &debug_str[ticket_hex_start..ticket_hex_end];

                if let Some(ticket_bytes) = (0..ticket_hex.len())
                    .step_by(2)
                    .map(|i| u8::from_str_radix(&ticket_hex[i..i + 2], 16).ok())
                    .collect::<Option<Vec<u8>>>()
                    && let Some(_decrypted) = decrypt_ticket(&ticket_bytes, &aes_key, &hmac_key)
                {
                    decrypted_successfully = true;
                    break;
                }
            }
        }
        assert!(
            decrypted_successfully,
            "Could not decrypt any captured session ticket with the provided keys"
        );
    }
    // 2. Second connection to verify resumption
    let stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port))
        .await
        .unwrap();
    let mut tls_stream = connector.connect(domain, stream).await.unwrap();

    let (_, conn) = tls_stream.get_mut();
    let is_resumed = conn.handshake_kind() == Some(rustls::HandshakeKind::Resumed);
    assert!(is_resumed, "Handshake was not resumed");

    container.stop().await.unwrap();
}
