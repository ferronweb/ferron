use std::{sync::Arc, time::Instant};

use ferron_observability::{
    LogEvent, LogLevel, MetricAttributeValue, MetricEvent, MetricType, MetricValue,
};
use http_body_util::{BodyExt, Empty};
use hyper_util::client::legacy::Client as HyperClient;
use hyper_util::{client::legacy::connect::HttpConnector, rt::TokioExecutor};
use parking_lot::RwLock;
use rustls::server::ResolvesServerCert;
use rustls::ClientConfig;
use rustls_pki_types::pem::PemObject;
use serde::Deserialize;
use x509_parser::certificate::X509Certificate;
use x509_parser::prelude::FromDer;

use crate::config::TlsHttpConfig;

pub type CertifiedKeyLock = Arc<RwLock<Option<Arc<rustls::sign::CertifiedKey>>>>;

/// Emit a log event through the observability sink.
pub fn emit_log(
    event_sink: &Arc<ferron_observability::CompositeEventSink>,
    level: LogLevel,
    message: &str,
    target: &'static str,
) {
    event_sink.emit(ferron_observability::Event::Log(LogEvent {
        level,
        message: message.to_string(),
        target,
        trace_context: None,
    }));
}

/// Emit a metric event through the observability sink.
#[inline]
fn emit_metric(
    event_sink: &Arc<ferron_observability::CompositeEventSink>,
    name: &'static str,
    value: MetricValue,
    ty: MetricType,
    unit: Option<&'static str>,
    description: Option<&'static str>,
    attributes: Vec<(&'static str, MetricAttributeValue)>,
) {
    event_sink.emit(ferron_observability::Event::Metric(MetricEvent {
        name,
        attributes,
        ty,
        value,
        unit,
        description,
    }));
}

#[derive(Deserialize)]
struct TlsHttpResponse {
    private_key: String,
    certificate: String,
}

pub async fn fetch_tls_cert_loop(
    config: TlsHttpConfig,
    certified_key: CertifiedKeyLock,
    event_sink: Arc<ferron_observability::CompositeEventSink>,
) {
    let url_string = config.url.to_string();
    emit_log(
        &event_sink,
        LogLevel::Info,
        &format!("TLS-HTTP certificate polling started for {url_string}"),
        "ferron_tls_http",
    );
    let Ok(tls_config) = build_rustls_client_config(config.no_verification) else {
        emit_log(
            &event_sink,
            LogLevel::Warn,
            "Can't build TLS client configuration for `tls-http`",
            "ferron_tls_http",
        );
        return;
    };
    let hyper_client: HyperClient<
        hyper_rustls::HttpsConnector<HttpConnector>,
        Empty<bytes::Bytes>,
    > = HyperClient::builder(TokioExecutor::new()).build(
        hyper_rustls::HttpsConnectorBuilder::new()
            .with_tls_config(tls_config)
            .https_or_http()
            .enable_http1()
            .enable_http2()
            .build(),
    );
    let mut is_first = true;
    loop {
        if is_first {
            is_first = false;
        } else {
            tokio::time::sleep(config.refresh_interval).await;
        }

        let start = Instant::now();
        let request = match hyper::Request::builder()
            .method("GET")
            .uri(config.url.clone())
            .body(Empty::<bytes::Bytes>::new())
        {
            Ok(req) => req,
            Err(e) => {
                emit_log(
                    &event_sink,
                    LogLevel::Warn,
                    &format!("Failed to build HTTP request for `tls-http`: {e}"),
                    "ferron_tls_http",
                );
                continue;
            }
        };

        let response = match hyper_client.request(request).await {
            Ok(res) => res,
            Err(e) => {
                let duration = start.elapsed().as_secs_f64();
                emit_metric(
                    &event_sink,
                    "ferron.tls_http.request_duration_seconds",
                    MetricValue::F64(duration),
                    MetricType::Histogram(None),
                    Some("s"),
                    Some("HTTP request duration for TLS certificate endpoint"),
                    vec![("status", MetricAttributeValue::StaticStr("error"))],
                );
                emit_log(
                    &event_sink,
                    LogLevel::Warn,
                    &format!("Failed to send HTTP request for `tls-http`: {e}"),
                    "ferron_tls_http",
                );
                continue;
            }
        };

        let duration = start.elapsed().as_secs_f64();
        emit_metric(
            &event_sink,
            "ferron.tls_http.request_duration_seconds",
            MetricValue::F64(duration),
            MetricType::Histogram(None),
            Some("s"),
            Some("HTTP request duration for TLS certificate endpoint"),
            vec![("status", MetricAttributeValue::StaticStr("success"))],
        );
        emit_metric(
            &event_sink,
            "ferron.tls_http.requests_total",
            MetricValue::U64(1),
            MetricType::Counter,
            None,
            Some("Total HTTP requests to TLS certificate endpoint"),
            vec![("status", MetricAttributeValue::StaticStr("success"))],
        );

        if !response.status().is_success() {
            emit_log(
                &event_sink,
                LogLevel::Warn,
                &format!(
                    "TLS certificate endpoint returned unsuccessful status: {}",
                    response.status()
                ),
                "ferron_tls_http",
            );
            continue;
        }

        // Get TLS certificate chain and private key from response
        let body_bytes = match response.into_body().collect().await {
            Ok(body) => body.to_bytes(),
            Err(e) => {
                emit_log(
                    &event_sink,
                    LogLevel::Warn,
                    &format!("Failed to read the HTTP response from TLS certificate endpoint: {e}"),
                    "ferron_tls_http",
                );
                continue;
            }
        };
        let body: TlsHttpResponse = match serde_json::from_slice(&body_bytes) {
            Ok(b) => b,
            Err(e) => {
                emit_log(
                    &event_sink,
                    LogLevel::Warn,
                    &format!(
                        "Failed to parse the HTTP response from TLS certificate endpoint: {e}"
                    ),
                    "ferron_tls_http",
                );
                continue;
            }
        };
        let cert_chain =
            match rustls_pki_types::CertificateDer::pem_slice_iter(body.certificate.as_bytes())
                .collect::<Result<Vec<_>, _>>()
            {
                Ok(cert) => cert,
                Err(e) => {
                    emit_log(
                        &event_sink,
                        LogLevel::Warn,
                        &format!(
                        "Failed to parse the TLS certificate chain from TLS endpoint response: {e}"
                    ),
                        "ferron_tls_http",
                    );
                    continue;
                }
            };
        let private_key =
            match rustls_pki_types::PrivateKeyDer::from_pem_slice(body.private_key.as_bytes()) {
                Ok(key) => key,
                Err(e) => {
                    emit_log(
                        &event_sink,
                        LogLevel::Warn,
                        &format!(
                            "Failed to parse the TLS private key from TLS endpoint response: {e}"
                        ),
                        "ferron_tls_http",
                    );
                    continue;
                }
            };
        let signing_key = match rustls::crypto::aws_lc_rs::default_provider()
            .key_provider
            .load_private_key(private_key)
        {
            Ok(key) => key,
            Err(e) => {
                emit_log(
                    &event_sink,
                    LogLevel::Warn,
                    &format!("Failed to load the TLS private key: {e}"),
                    "ferron_tls_http",
                );
                continue;
            }
        };
        let certified_key_to_write =
            Arc::new(rustls::sign::CertifiedKey::new(cert_chain, signing_key));

        // Check if the certified key has actually changed
        let key_changed = match certified_key.read().as_ref() {
            Some(existing) => {
                // Compare by checking if the first certificate differs
                let old_cert = existing.cert.first();
                let new_cert = certified_key_to_write.cert.first();
                match (old_cert, new_cert) {
                    (Some(a), Some(b)) => a.as_ref() != b.as_ref(),
                    _ => true,
                }
            }
            None => true,
        };

        if key_changed {
            // Emit certificate expiration metrics
            if let Some(first_cert) = certified_key_to_write.cert.first() {
                if let Ok((_, cert)) = X509Certificate::from_der(first_cert) {
                    let not_after = cert.validity().not_after.timestamp();
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);

                    emit_metric(
                        &event_sink,
                        "ferron.tls_http.cert_expires_at",
                        MetricValue::U64(not_after as u64),
                        MetricType::Gauge,
                        Some("{timestamp}"),
                        Some("Certificate expiration time (Unix timestamp)"),
                        vec![],
                    );

                    let days_remaining = (not_after - now as i64).max(0) / 86400;
                    emit_metric(
                        &event_sink,
                        "ferron.tls_http.cert_days_remaining",
                        MetricValue::I64(days_remaining),
                        MetricType::Gauge,
                        Some("{day}"),
                        Some("Days until certificate expiration"),
                        vec![],
                    );
                }
            }

            *certified_key.write() = Some(certified_key_to_write);
            emit_log(
                &event_sink,
                LogLevel::Info,
                "TLS certificate refreshed successfully from HTTP endpoint",
                "ferron_tls_http",
            );
            emit_metric(
                &event_sink,
                "ferron.tls_http.certificates_refreshed_total",
                MetricValue::U64(1),
                MetricType::Counter,
                None,
                Some("Total TLS certificate refresh outcomes from HTTP endpoint"),
                vec![("status", MetricAttributeValue::StaticStr("success"))],
            );
        }

        emit_metric(
            &event_sink,
            "ferron.tls_http.next_refresh_seconds",
            MetricValue::U64(config.refresh_interval.as_secs()),
            MetricType::Gauge,
            Some("s"),
            Some("Seconds until next certificate refresh"),
            vec![],
        );
    }
}

#[derive(Debug)]
pub struct TlsHttpResolver {
    certified_key: CertifiedKeyLock,
}

impl TlsHttpResolver {
    pub fn new(certified_key: CertifiedKeyLock) -> Self {
        Self { certified_key }
    }
}

impl ResolvesServerCert for TlsHttpResolver {
    fn resolve(
        &self,
        _client_hello: rustls::server::ClientHello<'_>,
    ) -> Option<Arc<rustls::sign::CertifiedKey>> {
        self.certified_key.read().as_ref().cloned()
    }
}

/// Builds a Rustls client configuration for ACME.
///
/// If `no_verification` is true, all certificate validation is skipped
/// (for testing or internal ACME directories).
pub fn build_rustls_client_config(
    no_verification: bool,
) -> Result<ClientConfig, Box<dyn std::error::Error + Send + Sync>> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());

    if no_verification {
        #[derive(Debug)]
        struct NoVerifier;

        impl rustls::client::danger::ServerCertVerifier for NoVerifier {
            fn verify_server_cert(
                &self,
                _end_entity: &rustls_pki_types::CertificateDer<'_>,
                _intermediates: &[rustls_pki_types::CertificateDer<'_>],
                _server_name: &rustls_pki_types::ServerName<'_>,
                _ocsp_response: &[u8],
                _now: rustls_pki_types::UnixTime,
            ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
                Ok(rustls::client::danger::ServerCertVerified::assertion())
            }

            fn verify_tls12_signature(
                &self,
                _message: &[u8],
                _cert: &rustls_pki_types::CertificateDer<'_>,
                _dss: &rustls::DigitallySignedStruct,
            ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error>
            {
                Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
            }

            fn verify_tls13_signature(
                &self,
                _message: &[u8],
                _cert: &rustls_pki_types::CertificateDer<'_>,
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

        Ok(ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoVerifier))
            .with_no_client_auth())
    } else {
        let root_store = build_root_cert_store()?;

        Ok(ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()?
            .with_root_certificates(root_store)
            .with_no_client_auth())
    }
}

/// Build a `RootCertStore` with native system certificates, falling back to
/// embedded `webpki-roots` if native certs cannot be loaded.
fn build_root_cert_store() -> Result<rustls::RootCertStore, Box<dyn std::error::Error + Send + Sync>>
{
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
