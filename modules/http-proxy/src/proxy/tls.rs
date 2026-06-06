use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use http::StatusCode;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};

use crate::types::upstream::MtlsCredentials;

#[allow(clippy::type_complexity)]
static TLS_CLIENT_CONFIG_CACHE: LazyLock<
    parking_lot::RwLock<HashMap<(bool, bool, bool, Option<usize>), Arc<ClientConfig>>>,
> = LazyLock::new(|| parking_lot::RwLock::new(HashMap::new()));

fn build_tls_config(
    http2: bool,
    http2_only: bool,
    no_verification: bool,
    mtls_credentials: Option<Arc<MtlsCredentials>>,
) -> ClientConfig {
    let builder = rustls::ClientConfig::builder();
    let builder = if no_verification {
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoServerVerifier))
    } else {
        let mut root_store = rustls::RootCertStore::empty();

        match rustls_native_certs::load_native_certs() {
            cert_result if !cert_result.errors.is_empty() => (),
            cert_result if cert_result.certs.is_empty() => (),
            cert_result => {
                for cert in cert_result.certs {
                    let _ = root_store.add(cert);
                }
            }
        }

        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

        builder.with_root_certificates(root_store)
    };
    let mut tls_client_config = if let Some(client_auth) = mtls_credentials {
        builder
            .clone()
            .with_client_auth_cert(client_auth.certs.clone(), client_auth.key.clone_key())
            .unwrap_or_else(|_| builder.with_no_client_auth())
    } else {
        builder.with_no_client_auth()
    };

    if http2_only {
        tls_client_config.alpn_protocols = vec![b"h2".to_vec()];
    } else if http2 {
        tls_client_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    } else {
        tls_client_config.alpn_protocols = vec![b"http/1.1".to_vec()];
    }

    tls_client_config
}

pub(super) fn cached_tls_config(
    http2: bool,
    http2_only: bool,
    no_verification: bool,
    mtls_credentials: Option<Arc<MtlsCredentials>>,
) -> Arc<ClientConfig> {
    let cache_key = (
        http2,
        http2_only,
        no_verification,
        mtls_credentials.as_ref().map(|c| Arc::as_ptr(c) as usize),
    );
    {
        let cache_read = TLS_CLIENT_CONFIG_CACHE.read();
        if let Some(config) = cache_read.get(&cache_key).cloned() {
            return config;
        }
    }

    let config = Arc::new(build_tls_config(
        http2,
        http2_only,
        no_verification,
        mtls_credentials,
    ));
    let mut cache_write = TLS_CLIENT_CONFIG_CACHE.write();
    Arc::clone(cache_write.entry(cache_key).or_insert(config))
}

#[derive(Debug)]
struct NoServerVerifier;

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
        rustls::crypto::aws_lc_rs::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

pub(super) fn io_error_status(err: &std::io::Error) -> (StatusCode, &'static str) {
    match err.kind() {
        std::io::ErrorKind::ConnectionRefused
        | std::io::ErrorKind::NotFound
        | std::io::ErrorKind::HostUnreachable => {
            (StatusCode::SERVICE_UNAVAILABLE, "Service unavailable")
        }
        std::io::ErrorKind::TimedOut => (StatusCode::GATEWAY_TIMEOUT, "Gateway timeout"),
        _ => (StatusCode::BAD_GATEWAY, "Bad gateway"),
    }
}
