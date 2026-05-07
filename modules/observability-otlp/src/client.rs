use std::{error::Error, sync::Arc, time::Duration};

use bytes::Bytes;
use http::{HeaderValue, Response};
use hyper_util::client::legacy::{connect::HttpConnector, Client};

/// Build a `RootCertStore` with native system certificates, falling back to
/// embedded `webpki-roots` if native certs cannot be loaded.
fn build_root_cert_store() -> Result<rustls::RootCertStore, Box<dyn Error>> {
    let mut root_store = rustls::RootCertStore::empty();
    let mut found_any = false;

    // Try native certs first
    match rustls_native_certs::load_native_certs() {
        cert_result if !cert_result.errors.is_empty() => {
            ferron_core::log_warn!(
                "native root CA certificate loading errors: {:?}",
                cert_result.errors
            );
        }
        cert_result if cert_result.certs.is_empty() => {
            ferron_core::log_warn!("no native root CA certificates found");
        }
        cert_result => {
            for cert in cert_result.certs {
                if let Err(err) = root_store.add(cert) {
                    ferron_core::log_warn!("native certificate parsing failed: {:?}", err);
                } else {
                    found_any = true;
                }
            }
        }
    }

    // Always add webpki-roots as fallback
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    if !found_any {
        ferron_core::log_warn!("using webpki-roots as fallback (no native root CAs available)");
    }

    if root_store.is_empty() {
        return Err("No root certificates available".into());
    }

    Ok(root_store)
}

/// Wrapper adapting hyper-util + hyper-rustls to opentelemetry-http's HttpClient trait.
#[derive(Clone, Debug)]
pub struct HyperOtelClient {
    inner: Client<
        hyper_rustls::HttpsConnector<HttpConnector>,
        http_body_util::Full<hyper::body::Bytes>,
    >,
    timeout: Duration,
}

impl HyperOtelClient {
    /// Build an HTTP client using hyper-util + hyper-rustls with the appropriate TLS config
    /// for OTLP HTTP exporters. Uses native certificate store with webpki-roots fallback.
    pub fn new(no_verify: bool) -> Result<Self, Box<dyn Error>> {
        use hyper_rustls::HttpsConnectorBuilder;
        use rustls::client::danger::ServerCertVerifier;
        use rustls::crypto::CryptoProvider;

        let crypto = CryptoProvider::get_default()
            .cloned()
            .unwrap_or_else(|| Arc::new(rustls::crypto::aws_lc_rs::default_provider()));

        let tls_config = if no_verify {
            #[derive(Debug)]
            struct NoServerVerifier;
            impl ServerCertVerifier for NoServerVerifier {
                fn verify_server_cert(
                    &self,
                    _end_entity: &rustls::pki_types::CertificateDer<'_>,
                    _intermediates: &[rustls::pki_types::CertificateDer<'_>],
                    _server_name: &rustls::pki_types::ServerName<'_>,
                    _ocsp_response: &[u8],
                    _now: rustls::pki_types::UnixTime,
                ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error>
                {
                    Ok(rustls::client::danger::ServerCertVerified::assertion())
                }

                fn verify_tls12_signature(
                    &self,
                    _message: &[u8],
                    _cert: &rustls::pki_types::CertificateDer<'_>,
                    _dss: &rustls::DigitallySignedStruct,
                ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error>
                {
                    Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
                }

                fn verify_tls13_signature(
                    &self,
                    _message: &[u8],
                    _cert: &rustls::pki_types::CertificateDer<'_>,
                    _dss: &rustls::DigitallySignedStruct,
                ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error>
                {
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
            rustls::ClientConfig::builder_with_provider(crypto)
                .with_safe_default_protocol_versions()
                .map_err(|e| format!("Failed to build TLS config: {e}"))?
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(NoServerVerifier))
                .with_no_client_auth()
        } else {
            let root_store = build_root_cert_store()?;
            rustls::ClientConfig::builder_with_provider(crypto)
                .with_safe_default_protocol_versions()
                .map_err(|e| format!("Failed to build TLS config: {e}"))?
                .with_root_certificates(root_store)
                .with_no_client_auth()
        };

        let https = HttpsConnectorBuilder::new()
            .with_tls_config(tls_config)
            .https_or_http()
            .enable_http1()
            .enable_http2()
            .build();

        let client = Client::builder(hyper_util::rt::TokioExecutor::new()).build(https);

        Ok(Self {
            inner: client,
            timeout: Duration::from_secs(10),
        })
    }
}

#[async_trait::async_trait]
impl opentelemetry_http::HttpClient for HyperOtelClient {
    async fn send_bytes(
        &self,
        request: opentelemetry_http::Request<Bytes>,
    ) -> Result<Response<Bytes>, opentelemetry_http::HttpError> {
        use tokio::time::timeout;

        let (parts, body) = request.into_parts();

        let mut req = hyper::Request::builder()
            .method(parts.method)
            .uri(parts.uri);

        for (key, value) in &parts.headers {
            req = req.header(key.as_str(), HeaderValue::from_bytes(value.as_ref())?);
        }

        let full_body = http_body_util::Full::new(body);
        let req = req.body(full_body)?;

        let fut = self.inner.request(req);
        let resp = timeout(self.timeout, fut).await??;

        let status = resp.status();
        let headers = resp.headers().clone();
        let body_bytes: Bytes = http_body_util::BodyExt::collect(resp.into_body())
            .await?
            .to_bytes();

        let mut response = http::Response::builder().status(status);

        for (key, value) in headers.iter() {
            response = response.header(key.as_str(), value.clone());
        }

        Ok(response.body(body_bytes)?)
    }
}

/// Build a tonic Channel with matching TLS config for use with OTLP gRPC exporters.
/// Uses native certificate store with webpki-roots fallback.
pub fn build_tonic_channel(endpoint: &str, no_verify: bool) -> Option<tonic::transport::Channel> {
    use hyper::Uri;
    use hyper_rustls::HttpsConnectorBuilder;
    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::crypto::CryptoProvider;
    use rustls::pki_types::ServerName;
    use tonic::transport::Endpoint;

    let crypto = CryptoProvider::get_default()
        .cloned()
        .unwrap_or_else(|| Arc::new(rustls::crypto::aws_lc_rs::default_provider()));

    let tls_config = if no_verify {
        #[derive(Debug)]
        struct NoServerVerifier;
        impl ServerCertVerifier for NoServerVerifier {
            fn verify_server_cert(
                &self,
                _end_entity: &rustls::pki_types::CertificateDer<'_>,
                _intermediates: &[rustls::pki_types::CertificateDer<'_>],
                _server_name: &ServerName<'_>,
                _ocsp: &[u8],
                _now: rustls::pki_types::UnixTime,
            ) -> Result<ServerCertVerified, rustls::Error> {
                Ok(ServerCertVerified::assertion())
            }
            fn verify_tls12_signature(
                &self,
                _message: &[u8],
                _cert: &rustls::pki_types::CertificateDer<'_>,
                _dss: &rustls::DigitallySignedStruct,
            ) -> Result<HandshakeSignatureValid, rustls::Error> {
                Err(rustls::Error::General("not supported".into()))
            }
            fn verify_tls13_signature(
                &self,
                _message: &[u8],
                _cert: &rustls::pki_types::CertificateDer<'_>,
                _dss: &rustls::DigitallySignedStruct,
            ) -> Result<HandshakeSignatureValid, rustls::Error> {
                Err(rustls::Error::General("not supported".into()))
            }
            fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
                vec![]
            }
        }
        rustls::ClientConfig::builder_with_provider(crypto)
            .with_safe_default_protocol_versions()
            .ok()?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoServerVerifier))
            .with_no_client_auth()
    } else {
        let root_store = build_root_cert_store().ok()?;
        rustls::ClientConfig::builder_with_provider(crypto)
            .with_safe_default_protocol_versions()
            .ok()?
            .with_root_certificates(root_store)
            .with_no_client_auth()
    };

    let https = HttpsConnectorBuilder::new()
        .with_tls_config(tls_config)
        .https_or_http()
        .enable_http1()
        .enable_http2()
        .build();

    let uri: Uri = endpoint.parse().ok()?;
    Some(Endpoint::from(uri).connect_with_connector_lazy(https))
}
