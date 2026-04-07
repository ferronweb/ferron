use std::collections::HashMap;
use std::error::Error;
use std::sync::{Arc, Once};

use async_trait::async_trait;
use bytes::Bytes;
use dashmap::DashMap;
use ferron_core::{
    config::ServerConfigurationBlock,
    loader::ModuleLoader,
    log_warn,
    providers::Provider,
    registry::{Registry, RegistryBuilder},
    Module,
};
use ferron_observability::{
    AccessEvent, Event, EventSink, LogEvent, LogLevel, MetricAttributeValue, MetricEvent,
    MetricType, MetricValue, ObservabilityContext, TraceAttributeValue, TraceEvent,
};
use hyper::header::HeaderValue;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use opentelemetry::{logs::AnyValue, trace::TracerProvider, KeyValue};
use opentelemetry_http::{HttpClient as OtelHttpClient, Response};
use opentelemetry_otlp::{WithExportConfig, WithHttpConfig, WithTonicConfig};
use opentelemetry_sdk::Resource;
use std::time::Duration;

static DROPPED_EVENT: Once = Once::new();

/// Per-host configuration for a single OTLP signal (logs, metrics, or traces)
struct SignalConfig {
    endpoint: String,
    protocol: String,
    authorization: Option<String>,
}

/// Shared configuration for an OTLP backend instance
#[allow(dead_code)]
struct OtlpBackendConfig {
    service_name: String,
    no_verify: bool,
    logs: Option<SignalConfig>,
    metrics: Option<SignalConfig>,
    traces: Option<SignalConfig>,
}

/// Correlation context: tracks active spans per host sink instance.
struct CorrelationContext {
    /// Active spans: span_name -> (trace_id_hex, span)
    active_spans: DashMap<String, (String, opentelemetry_sdk::trace::Span)>,
}

impl CorrelationContext {
    fn new() -> Self {
        Self {
            active_spans: DashMap::new(),
        }
    }

    fn insert_span(
        &self,
        name: String,
        trace_id_hex: String,
        span: opentelemetry_sdk::trace::Span,
    ) {
        self.active_spans.insert(name, (trace_id_hex, span));
    }

    fn remove_span(&self, name: &str) -> Option<(String, opentelemetry_sdk::trace::Span)> {
        self.active_spans.remove(name).map(|(_, v)| v)
    }

    /// Look up an active span's trace and span ID for use as a parent.
    fn get_parent_ids(&self, name: &str) -> Option<(String, String)> {
        use opentelemetry::trace::Span;
        self.active_spans.get(name).map(|entry| {
            let (trace_id_hex, span) = entry.value();
            (
                trace_id_hex.clone(),
                span.span_context().span_id().to_string(),
            )
        })
    }
}

/// Wrapper that carries an event with its configuration through the channel
struct ConfiguredEvent {
    event: Event,
    log_config: Arc<ServerConfigurationBlock>,
}

/// The OTLP event sink that emits events to an OTLP collector
struct OtlpEventSink {
    inner: async_channel::Sender<ConfiguredEvent>,
    log_config: Arc<ServerConfigurationBlock>,
}

impl EventSink for OtlpEventSink {
    fn emit(&self, event: Event) {
        if self
            .inner
            .try_send(ConfiguredEvent {
                event,
                log_config: self.log_config.clone(),
            })
            .is_err()
        {
            DROPPED_EVENT.call_once(|| {
                log_warn!(
                    "Observability event dropped (`otlp` observability backend). \
                    This may be caused by high server load."
                )
            });
        }
    }
}

/// Parse the OTLP backend configuration from a ServerConfigurationBlock
fn parse_otlp_config(
    config: &ServerConfigurationBlock,
) -> Result<OtlpBackendConfig, Box<dyn Error>> {
    let service_name = config
        .get_value("service_name")
        .and_then(|v| v.as_str())
        .unwrap_or("ferron")
        .to_string();

    let no_verify = config
        .get_value("no_verify")
        .and_then(|v| v.as_boolean())
        .unwrap_or(false);

    let logs = parse_signal_config(config, "logs");
    let metrics = parse_signal_config(config, "metrics");
    let traces = parse_signal_config(config, "traces");

    Ok(OtlpBackendConfig {
        service_name,
        no_verify,
        logs,
        metrics,
        traces,
    })
}

/// Parse a single signal sub-block (logs, metrics, or traces)
fn parse_signal_config(parent: &ServerConfigurationBlock, name: &str) -> Option<SignalConfig> {
    let entries = parent.directives.get(name)?;
    let entry = entries.first()?;
    let endpoint = entry.args.first().and_then(|v| v.as_str())?.to_string();

    let children = entry.children.as_ref()?;

    let protocol = children
        .get_value("protocol")
        .and_then(|v| v.as_str())
        .unwrap_or("grpc")
        .to_string();

    let authorization = children
        .get_value("authorization")
        .and_then(|v| v.as_str())
        .map(|s: &str| s.to_string());

    Some(SignalConfig {
        endpoint,
        protocol,
        authorization,
    })
}

/// Build an OTLP resource from the service name
fn build_resource(service_name: String) -> Resource {
    Resource::builder().with_service_name(service_name).build()
}

struct OtlpObservabilityModule {
    inner: async_channel::Receiver<ConfiguredEvent>,
    cancel_token: tokio_util::sync::CancellationToken,
}

impl Module for OtlpObservabilityModule {
    fn name(&self) -> &str {
        "observability-otlp"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn start(
        &self,
        runtime: &mut ferron_core::runtime::Runtime,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let cancel_token = self.cancel_token.clone();
        let rx = self.inner.clone();

        runtime.spawn_secondary_task(async move {
            // Per-config exporter cache
            let mut providers: HashMap<String, OtlpProviderCache> = HashMap::new();

            while let Some(msg) = tokio::select! {
                result = rx.recv() => result.ok(),
                _ = cancel_token.cancelled() => None,
            } {
                let config = match parse_otlp_config(&msg.log_config) {
                    Ok(c) => c,
                    Err(e) => {
                        ferron_core::log_error!("Failed to parse OTLP config: {}", e);
                        continue;
                    }
                };

                let cache_key = config_cache_key(&config);
                let entry = providers
                    .entry(cache_key)
                    .or_insert_with(|| init_providers(&config));

                match &msg.event {
                    Event::Log(log_event) => {
                        if let Some(ref provider) = entry.logs_provider {
                            emit_log(provider, log_event);
                        }
                    }
                    Event::Metric(metric_event) => {
                        if let Some(ref provider) = entry.metrics_provider {
                            emit_metric(provider, metric_event, &mut entry.metrics_instruments);
                        }
                    }
                    Event::Trace(trace_event) => {
                        if let Some(ref provider) = entry.traces_provider {
                            emit_trace(provider, trace_event, &entry.correlation);
                        }
                    }
                    Event::Access(access_event) => {
                        if let Some(ref provider) = entry.logs_provider {
                            emit_access_log(provider, access_event);
                        }
                    }
                }
            }

            // Shutdown providers
            // `tokio::task::spawn_blocking` is needed, because without it, there can be a deadlock.
            // See https://docs.rs/opentelemetry_sdk/latest/opentelemetry_sdk/trace/struct.BatchSpanProcessor.html
            tokio::task::spawn_blocking(move || {
                for (_, cache) in providers {
                    if let Some(p) = cache.logs_provider {
                        let _ = p.shutdown();
                    }
                    if let Some(p) = cache.metrics_provider {
                        let _ = p.shutdown();
                    }
                    if let Some(p) = cache.traces_provider {
                        let _ = p.shutdown();
                    }
                }
            });
        });

        Ok(())
    }
}

impl Drop for OtlpObservabilityModule {
    fn drop(&mut self) {
        self.cancel_token.cancel();
    }
}

/// Cached OTLP providers for a given config
struct OtlpProviderCache {
    logs_provider: Option<opentelemetry_sdk::logs::SdkLoggerProvider>,
    metrics_provider: Option<opentelemetry_sdk::metrics::SdkMeterProvider>,
    traces_provider: Option<opentelemetry_sdk::trace::SdkTracerProvider>,
    correlation: Arc<CorrelationContext>,
    metrics_instruments: HashMap<&'static str, CachedInstrument>,
}

enum CachedInstrument {
    F64Counter(opentelemetry::metrics::Counter<f64>),
    F64Gauge(opentelemetry::metrics::Gauge<f64>),
    F64Histogram(opentelemetry::metrics::Histogram<f64>),
    F64UpDownCounter(opentelemetry::metrics::UpDownCounter<f64>),
    I64Gauge(opentelemetry::metrics::Gauge<i64>),
    I64UpDownCounter(opentelemetry::metrics::UpDownCounter<i64>),
    U64Counter(opentelemetry::metrics::Counter<u64>),
    U64Gauge(opentelemetry::metrics::Gauge<u64>),
    U64Histogram(opentelemetry::metrics::Histogram<u64>),
}

/// Create a cache key from the signal configs
fn config_cache_key(config: &OtlpBackendConfig) -> String {
    let logs_key = config
        .logs
        .as_ref()
        .map(|s| {
            format!(
                "{}|{}|{}",
                s.endpoint,
                s.protocol,
                s.authorization.as_deref().unwrap_or("")
            )
        })
        .unwrap_or_default();
    let metrics_key = config
        .metrics
        .as_ref()
        .map(|s| {
            format!(
                "{}|{}|{}",
                s.endpoint,
                s.protocol,
                s.authorization.as_deref().unwrap_or("")
            )
        })
        .unwrap_or_default();
    let traces_key = config
        .traces
        .as_ref()
        .map(|s| {
            format!(
                "{}|{}|{}",
                s.endpoint,
                s.protocol,
                s.authorization.as_deref().unwrap_or("")
            )
        })
        .unwrap_or_default();
    format!(
        "{}|{}|{}|{}",
        config.service_name, logs_key, metrics_key, traces_key
    )
}

/// Set OTEL env vars for the current signal's headers, build the exporter, then clear them.
/// This is called during provider initialization in a single-threaded context.
fn set_otlp_headers_temporarily(signal: &str, authorization: &Option<String>) -> TempHeaderGuard {
    let var_name = format!("OTEL_EXPORTER_OTLP_{signal}_HEADERS");
    let old_val = std::env::var(&var_name).ok();

    if let Some(auth) = authorization {
        std::env::set_var(&var_name, format!("Authorization={auth}"));
    }

    TempHeaderGuard {
        var_name,
        old_val,
        had_auth: authorization.is_some(),
    }
}

struct TempHeaderGuard {
    #[allow(dead_code)]
    var_name: String,
    #[allow(dead_code)]
    old_val: Option<String>,
    #[allow(dead_code)]
    had_auth: bool,
}

impl TempHeaderGuard {
    #[allow(dead_code)]
    fn cleanup(self) {
        if self.had_auth {
            if let Some(old) = self.old_val {
                std::env::set_var(&self.var_name, old);
            } else {
                std::env::remove_var(&self.var_name);
            }
        }
    }
}

fn init_providers(config: &OtlpBackendConfig) -> OtlpProviderCache {
    let resource = build_resource(config.service_name.clone());
    let correlation = Arc::new(CorrelationContext::new());

    let logs_provider = config.logs.as_ref().and_then(|sig| {
        let _guard = set_otlp_headers_temporarily("LOGS", &sig.authorization);
        build_logs_provider(sig, &config.no_verify, &resource)
    });

    let metrics_provider = config.metrics.as_ref().and_then(|sig| {
        let _guard = set_otlp_headers_temporarily("METRICS", &sig.authorization);
        build_metrics_provider(sig, &config.no_verify, &resource)
    });

    let traces_provider = config.traces.as_ref().and_then(|sig| {
        let _guard = set_otlp_headers_temporarily("TRACES", &sig.authorization);
        build_traces_provider(sig, &config.no_verify, &resource)
    });

    OtlpProviderCache {
        logs_provider,
        metrics_provider,
        traces_provider,
        correlation,
        metrics_instruments: HashMap::new(),
    }
}

/// Build an HTTP client using hyper-util + hyper-rustls with the appropriate TLS config
/// for OTLP HTTP exporters. Uses native certificate store with webpki-roots fallback.
fn build_http_client(no_verify: bool) -> Result<HyperOtelClient, Box<dyn Error>> {
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
            ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
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

    Ok(HyperOtelClient {
        inner: client,
        timeout: Duration::from_secs(10),
    })
}

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

#[async_trait]
impl OtelHttpClient for HyperOtelClient {
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
fn build_tonic_channel(endpoint: &str, no_verify: bool) -> Option<tonic::transport::Channel> {
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

fn build_logs_provider(
    sig: &SignalConfig,
    no_verify: &bool,
    resource: &Resource,
) -> Option<opentelemetry_sdk::logs::SdkLoggerProvider> {
    use opentelemetry_otlp::LogExporter;
    use opentelemetry_sdk::logs::log_processor_with_async_runtime::BatchLogProcessor;

    let exporter: LogExporter = match sig.protocol.as_str() {
        "http/protobuf" => LogExporter::builder()
            .with_http()
            .with_http_client(build_http_client(*no_verify).ok()?)
            .with_protocol(opentelemetry_otlp::Protocol::HttpBinary)
            .with_endpoint(&sig.endpoint)
            .build()
            .ok()?,
        "http/json" => LogExporter::builder()
            .with_http()
            .with_http_client(build_http_client(*no_verify).ok()?)
            .with_protocol(opentelemetry_otlp::Protocol::HttpJson)
            .with_endpoint(&sig.endpoint)
            .build()
            .ok()?,
        _ => LogExporter::builder()
            .with_tonic()
            .with_channel(build_tonic_channel(&sig.endpoint, *no_verify)?)
            .build()
            .ok()?,
    };

    Some(
        opentelemetry_sdk::logs::SdkLoggerProvider::builder()
            .with_log_processor(
                BatchLogProcessor::builder(exporter, opentelemetry_sdk::runtime::Tokio).build(),
            )
            .with_resource(resource.clone())
            .build(),
    )
}

fn build_metrics_provider(
    sig: &SignalConfig,
    no_verify: &bool,
    resource: &Resource,
) -> Option<opentelemetry_sdk::metrics::SdkMeterProvider> {
    use opentelemetry_otlp::MetricExporter;
    use opentelemetry_sdk::metrics::periodic_reader_with_async_runtime::PeriodicReader;

    let exporter: MetricExporter = match sig.protocol.as_str() {
        "http/protobuf" => MetricExporter::builder()
            .with_http()
            .with_http_client(build_http_client(*no_verify).ok()?)
            .with_protocol(opentelemetry_otlp::Protocol::HttpBinary)
            .with_endpoint(&sig.endpoint)
            .build()
            .ok()?,
        "http/json" => MetricExporter::builder()
            .with_http()
            .with_http_client(build_http_client(*no_verify).ok()?)
            .with_protocol(opentelemetry_otlp::Protocol::HttpJson)
            .with_endpoint(&sig.endpoint)
            .build()
            .ok()?,
        _ => MetricExporter::builder()
            .with_tonic()
            .with_channel(build_tonic_channel(&sig.endpoint, *no_verify)?)
            .build()
            .ok()?,
    };

    Some(
        opentelemetry_sdk::metrics::SdkMeterProvider::builder()
            .with_reader(
                PeriodicReader::builder(exporter, opentelemetry_sdk::runtime::Tokio)
                    .with_interval(std::time::Duration::from_secs(30))
                    .build(),
            )
            .with_resource(resource.clone())
            .build(),
    )
}

fn build_traces_provider(
    sig: &SignalConfig,
    no_verify: &bool,
    resource: &Resource,
) -> Option<opentelemetry_sdk::trace::SdkTracerProvider> {
    use opentelemetry_otlp::SpanExporter;
    use opentelemetry_sdk::trace::span_processor_with_async_runtime::BatchSpanProcessor;

    let exporter: SpanExporter = match sig.protocol.as_str() {
        "http/protobuf" => SpanExporter::builder()
            .with_http()
            .with_http_client(build_http_client(*no_verify).ok()?)
            .with_protocol(opentelemetry_otlp::Protocol::HttpBinary)
            .with_endpoint(&sig.endpoint)
            .build()
            .ok()?,
        "http/json" => SpanExporter::builder()
            .with_http()
            .with_http_client(build_http_client(*no_verify).ok()?)
            .with_protocol(opentelemetry_otlp::Protocol::HttpJson)
            .with_endpoint(&sig.endpoint)
            .build()
            .ok()?,
        _ => SpanExporter::builder()
            .with_tonic()
            .with_channel(build_tonic_channel(&sig.endpoint, *no_verify)?)
            .build()
            .ok()?,
    };

    Some(
        opentelemetry_sdk::trace::SdkTracerProvider::builder()
            .with_span_processor(
                BatchSpanProcessor::builder(exporter, opentelemetry_sdk::runtime::Tokio).build(),
            )
            .with_resource(resource.clone())
            .build(),
    )
}

fn emit_log(provider: &opentelemetry_sdk::logs::SdkLoggerProvider, event: &LogEvent) {
    use opentelemetry::logs::{LogRecord, Logger, LoggerProvider, Severity};

    let logger = provider.logger("ferron");
    let mut record = logger.create_log_record();

    record.set_body(AnyValue::String(event.message.clone().into()));
    record.set_severity_number(match event.level {
        LogLevel::Error => Severity::Error,
        LogLevel::Warn => Severity::Warn,
        LogLevel::Info => Severity::Info,
        LogLevel::Debug => Severity::Debug,
    });
    record.set_severity_text(match event.level {
        LogLevel::Error => "ERROR",
        LogLevel::Warn => "WARN",
        LogLevel::Info => "INFO",
        LogLevel::Debug => "DEBUG",
    });
    record.add_attribute("log.target", event.target);

    logger.emit(record);
}

fn emit_access_log(
    provider: &opentelemetry_sdk::logs::SdkLoggerProvider,
    _event: &Arc<dyn AccessEvent>,
) {
    use opentelemetry::logs::{LogRecord, Logger, LoggerProvider};

    let logger = provider.logger("ferron.access");
    let mut record = logger.create_log_record();
    record.set_body(AnyValue::String("access_log".into()));
    logger.emit(record);
}

fn emit_metric(
    provider: &opentelemetry_sdk::metrics::SdkMeterProvider,
    event: &MetricEvent,
    instruments: &mut HashMap<&'static str, CachedInstrument>,
) {
    use opentelemetry::metrics::MeterProvider;

    let meter = provider.meter("ferron");
    let attrs: Vec<KeyValue> = event
        .attributes
        .iter()
        .map(|(k, v)| {
            KeyValue::new(
                *k,
                match v {
                    MetricAttributeValue::F64(val) => opentelemetry::Value::from(*val),
                    MetricAttributeValue::I64(val) => opentelemetry::Value::from(*val),
                    MetricAttributeValue::String(val) => opentelemetry::Value::from(val.clone()),
                    MetricAttributeValue::Bool(val) => opentelemetry::Value::from(*val),
                },
            )
        })
        .collect();

    match (&event.ty, event.value) {
        (MetricType::Counter, MetricValue::F64(val)) => {
            let instrument = instruments.entry(event.name).or_insert_with(|| {
                let mut b = meter.f64_counter(event.name);
                if let Some(u) = event.unit {
                    b = b.with_unit(u);
                }
                if let Some(d) = event.description {
                    b = b.with_description(d);
                }
                CachedInstrument::F64Counter(b.build())
            });
            if let CachedInstrument::F64Counter(i) = instrument {
                i.add(val, &attrs);
            }
        }
        (MetricType::Counter, MetricValue::U64(val)) => {
            let instrument = instruments.entry(event.name).or_insert_with(|| {
                let mut b = meter.u64_counter(event.name);
                if let Some(u) = event.unit {
                    b = b.with_unit(u);
                }
                if let Some(d) = event.description {
                    b = b.with_description(d);
                }
                CachedInstrument::U64Counter(b.build())
            });
            if let CachedInstrument::U64Counter(i) = instrument {
                i.add(val, &attrs);
            }
        }
        (MetricType::UpDownCounter, MetricValue::F64(val)) => {
            let instrument = instruments.entry(event.name).or_insert_with(|| {
                let mut b = meter.f64_up_down_counter(event.name);
                if let Some(u) = event.unit {
                    b = b.with_unit(u);
                }
                if let Some(d) = event.description {
                    b = b.with_description(d);
                }
                CachedInstrument::F64UpDownCounter(b.build())
            });
            if let CachedInstrument::F64UpDownCounter(i) = instrument {
                i.add(val, &attrs);
            }
        }
        (MetricType::UpDownCounter, MetricValue::I64(val)) => {
            let instrument = instruments.entry(event.name).or_insert_with(|| {
                let mut b = meter.i64_up_down_counter(event.name);
                if let Some(u) = event.unit {
                    b = b.with_unit(u);
                }
                if let Some(d) = event.description {
                    b = b.with_description(d);
                }
                CachedInstrument::I64UpDownCounter(b.build())
            });
            if let CachedInstrument::I64UpDownCounter(i) = instrument {
                i.add(val, &attrs);
            }
        }
        (MetricType::Gauge, MetricValue::F64(val)) => {
            let instrument = instruments.entry(event.name).or_insert_with(|| {
                let mut b = meter.f64_gauge(event.name);
                if let Some(u) = event.unit {
                    b = b.with_unit(u);
                }
                if let Some(d) = event.description {
                    b = b.with_description(d);
                }
                CachedInstrument::F64Gauge(b.build())
            });
            if let CachedInstrument::F64Gauge(i) = instrument {
                i.record(val, &attrs);
            }
        }
        (MetricType::Gauge, MetricValue::I64(val)) => {
            let instrument = instruments.entry(event.name).or_insert_with(|| {
                let mut b = meter.i64_gauge(event.name);
                if let Some(u) = event.unit {
                    b = b.with_unit(u);
                }
                if let Some(d) = event.description {
                    b = b.with_description(d);
                }
                CachedInstrument::I64Gauge(b.build())
            });
            if let CachedInstrument::I64Gauge(i) = instrument {
                i.record(val, &attrs);
            }
        }
        (MetricType::Gauge, MetricValue::U64(val)) => {
            let instrument = instruments.entry(event.name).or_insert_with(|| {
                let mut b = meter.u64_gauge(event.name);
                if let Some(u) = event.unit {
                    b = b.with_unit(u);
                }
                if let Some(d) = event.description {
                    b = b.with_description(d);
                }
                CachedInstrument::U64Gauge(b.build())
            });
            if let CachedInstrument::U64Gauge(i) = instrument {
                i.record(val, &attrs);
            }
        }
        (MetricType::Histogram(buckets), MetricValue::F64(val)) => {
            let instrument = instruments.entry(event.name).or_insert_with(|| {
                let mut b = meter.f64_histogram(event.name);
                if let Some(u) = event.unit {
                    b = b.with_unit(u);
                }
                if let Some(ref bkt) = buckets {
                    b = b.with_boundaries(bkt.clone());
                }
                if let Some(d) = event.description {
                    b = b.with_description(d);
                }
                CachedInstrument::F64Histogram(b.build())
            });
            if let CachedInstrument::F64Histogram(i) = instrument {
                i.record(val, &attrs);
            }
        }
        (MetricType::Histogram(buckets), MetricValue::U64(val)) => {
            let instrument = instruments.entry(event.name).or_insert_with(|| {
                let mut b = meter.u64_histogram(event.name);
                if let Some(u) = event.unit {
                    b = b.with_unit(u);
                }
                if let Some(ref bkt) = buckets {
                    b = b.with_boundaries(bkt.clone());
                }
                if let Some(d) = event.description {
                    b = b.with_description(d);
                }
                CachedInstrument::U64Histogram(b.build())
            });
            if let CachedInstrument::U64Histogram(i) = instrument {
                i.record(val, &attrs);
            }
        }
        _ => {}
    }
}

fn emit_trace(
    provider: &opentelemetry_sdk::trace::SdkTracerProvider,
    event: &TraceEvent,
    correlation: &CorrelationContext,
) {
    use opentelemetry::trace::{
        Span, SpanContext, SpanId, TraceContextExt, TraceFlags, TraceId, TraceState, Tracer,
    };
    use opentelemetry::Context;

    let tracer = provider.tracer("ferron");

    match event {
        TraceEvent::StartSpan {
            name,
            parent_span_id,
            attributes,
        } => {
            let mut span = if let Some(parent_name) = parent_span_id {
                // Look up the parent span's trace_id and span_id by name
                if let Some((trace_id_hex, parent_span_id_hex)) =
                    correlation.get_parent_ids(parent_name)
                {
                    if let (Ok(trace_id), Ok(span_id)) = (
                        TraceId::from_hex(&trace_id_hex),
                        SpanId::from_hex(&parent_span_id_hex),
                    ) {
                        let parent_ctx = SpanContext::new(
                            trace_id,
                            span_id,
                            TraceFlags::SAMPLED,
                            true,
                            TraceState::default(),
                        );
                        let parent_cx = Context::new().with_remote_span_context(parent_ctx);
                        tracer.start_with_context(name.clone(), &parent_cx)
                    } else {
                        tracer.start(name.clone())
                    }
                } else {
                    tracer.start(name.clone())
                }
            } else {
                tracer.start(name.clone())
            };

            // Set semantic convention attributes
            for (key, value) in attributes {
                span.set_attribute(trace_kv(key, value));
            }

            let trace_id_hex = span.span_context().trace_id().to_string();
            correlation.insert_span(name.clone(), trace_id_hex, span);
        }
        TraceEvent::EndSpan {
            name,
            error,
            attributes,
        } => {
            if let Some((_, mut span)) = correlation.remove_span(name) {
                // Apply any final attributes (e.g. http.response.status_code)
                for (key, value) in attributes {
                    span.set_attribute(trace_kv(key, value));
                }
                if let Some(error_desc) = error {
                    span.set_status(opentelemetry::trace::Status::error(error_desc.clone()));
                }
                span.end();
            }
        }
    }
}

/// Convert a TraceAttributeValue into an OTEL KeyValue.
fn trace_kv(key: &'static str, value: &TraceAttributeValue) -> KeyValue {
    match value {
        TraceAttributeValue::String(s) => KeyValue::new(key, s.clone()),
        TraceAttributeValue::Bool(b) => KeyValue::new(key, *b),
        TraceAttributeValue::I64(i) => KeyValue::new(key, *i),
        TraceAttributeValue::F64(f) => KeyValue::new(key, *f),
    }
}

struct OtlpObservabilityProvider {
    inner: async_channel::Sender<ConfiguredEvent>,
}

impl Provider<ObservabilityContext> for OtlpObservabilityProvider {
    fn name(&self) -> &str {
        "otlp"
    }

    fn execute(&self, ctx: &mut ObservabilityContext) -> Result<(), Box<dyn Error>> {
        ctx.sink = Some(Arc::new(OtlpEventSink {
            inner: self.inner.clone(),
            log_config: ctx.log_config.clone(),
        }));
        Ok(())
    }
}

pub struct OtlpObservabilityModuleLoader {
    cache: Option<Arc<OtlpObservabilityModule>>,
    channel: (
        async_channel::Sender<ConfiguredEvent>,
        async_channel::Receiver<ConfiguredEvent>,
    ),
}

impl Default for OtlpObservabilityModuleLoader {
    fn default() -> Self {
        Self {
            cache: None,
            channel: async_channel::bounded(131072),
        }
    }
}

impl ModuleLoader for OtlpObservabilityModuleLoader {
    fn register_providers(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        let channel = self.channel.0.clone();

        registry.with_provider::<ObservabilityContext, _>(move || {
            Arc::new(OtlpObservabilityProvider {
                inner: channel.clone(),
            })
        })
    }

    fn register_modules(
        &mut self,
        _registry: Arc<Registry>,
        modules: &mut Vec<Arc<dyn Module>>,
        _config: Arc<ferron_core::config::ServerConfiguration>,
    ) -> Result<(), Box<dyn Error>> {
        if self.cache.is_none() {
            let module = Arc::new(OtlpObservabilityModule {
                inner: self.channel.1.clone(),
                cancel_token: tokio_util::sync::CancellationToken::new(),
            });

            self.cache = Some(module.clone());
            modules.push(module);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferron_core::config::ServerConfigurationBlock;
    use ferron_core::config::ServerConfigurationDirectiveEntry;
    use ferron_core::config::ServerConfigurationValue;

    #[allow(dead_code)]
    fn make_block(
        directives: HashMap<String, Vec<ServerConfigurationDirectiveEntry>>,
    ) -> ServerConfigurationBlock {
        ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: HashMap::new(),
            span: None,
        }
    }

    #[allow(dead_code)]
    fn directive_string(
        name: &str,
        value: &str,
    ) -> (String, Vec<ServerConfigurationDirectiveEntry>) {
        (
            name.to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::String(value.to_string(), None)],
                children: None,
                span: None,
            }],
        )
    }

    #[allow(dead_code)]
    fn directive_bool(name: &str, value: bool) -> (String, Vec<ServerConfigurationDirectiveEntry>) {
        (
            name.to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::Boolean(value, None)],
                children: None,
                span: None,
            }],
        )
    }

    #[allow(dead_code)]
    fn directive_with_children(
        name: &str,
        value: &str,
        children: HashMap<String, Vec<ServerConfigurationDirectiveEntry>>,
    ) -> (String, Vec<ServerConfigurationDirectiveEntry>) {
        (
            name.to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::String(value.to_string(), None)],
                children: Some(ServerConfigurationBlock {
                    directives: Arc::new(children),
                    matchers: HashMap::new(),
                    span: None,
                }),
                span: None,
            }],
        )
    }

    #[test]
    fn parses_otlp_config_with_all_signals() {
        let mut directives = HashMap::new();
        directives.insert(
            "service_name".to_string(),
            vec![ferron_core::config::ServerConfigurationDirectiveEntry {
                args: vec![ferron_core::config::ServerConfigurationValue::String(
                    "test-service".to_string(),
                    None,
                )],
                children: None,
                span: None,
            }],
        );
        directives.insert(
            "no_verify".to_string(),
            vec![ferron_core::config::ServerConfigurationDirectiveEntry {
                args: vec![ferron_core::config::ServerConfigurationValue::Boolean(
                    true, None,
                )],
                children: None,
                span: None,
            }],
        );

        let mut logs_children = HashMap::new();
        logs_children.insert(
            "protocol".to_string(),
            vec![ferron_core::config::ServerConfigurationDirectiveEntry {
                args: vec![ferron_core::config::ServerConfigurationValue::String(
                    "http/protobuf".to_string(),
                    None,
                )],
                children: None,
                span: None,
            }],
        );
        directives.insert(
            "logs".to_string(),
            vec![ferron_core::config::ServerConfigurationDirectiveEntry {
                args: vec![ferron_core::config::ServerConfigurationValue::String(
                    "https://collector:4318/v1/logs".to_string(),
                    None,
                )],
                children: Some(ServerConfigurationBlock {
                    directives: Arc::new(logs_children),
                    matchers: HashMap::new(),
                    span: None,
                }),
                span: None,
            }],
        );

        let block = ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: HashMap::new(),
            span: None,
        };

        let config = parse_otlp_config(&block).expect("should parse");
        assert_eq!(config.service_name, "test-service");
        assert!(config.no_verify);
        assert!(config.logs.is_some());
        assert!(config.metrics.is_none());
        assert!(config.traces.is_none());

        let logs = config.logs.unwrap();
        assert_eq!(logs.endpoint, "https://collector:4318/v1/logs");
        assert_eq!(logs.protocol, "http/protobuf");
    }

    #[test]
    fn parses_otlp_config_minimal() {
        let directives = HashMap::new();
        let block = ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: HashMap::new(),
            span: None,
        };

        let config = parse_otlp_config(&block).expect("should parse");
        assert_eq!(config.service_name, "ferron");
        assert!(!config.no_verify);
        assert!(config.logs.is_none());
        assert!(config.metrics.is_none());
        assert!(config.traces.is_none());
    }

    #[test]
    fn correlation_context_tracks_active_spans() {
        use opentelemetry::trace::{Span, Tracer, TracerProvider};

        let ctx = CorrelationContext::new();
        let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder().build();
        let tracer = provider.tracer("test");

        let span = tracer.start("ferron.request_handler");
        let trace_id_hex = span.span_context().trace_id().to_string();
        let span_id_hex = span.span_context().span_id().to_string();

        ctx.insert_span(
            "ferron.request_handler".to_string(),
            trace_id_hex.clone(),
            span,
        );

        let (t_id, s_id) = ctx
            .get_parent_ids("ferron.request_handler")
            .expect("should have active span");
        assert_eq!(t_id, trace_id_hex);
        assert_eq!(s_id, span_id_hex);

        let result = ctx.remove_span("ferron.request_handler");
        assert!(result.is_some());

        assert!(ctx.get_parent_ids("ferron.request_handler").is_none());
    }

    #[test]
    fn emit_trace_start_span_stores_span_object() {
        use ferron_observability::TraceAttributeValue;

        let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder().build();
        let correlation = CorrelationContext::new();

        let event = TraceEvent::StartSpan {
            name: "test.span".to_string(),
            parent_span_id: None,
            attributes: vec![
                (
                    "http.request.method",
                    TraceAttributeValue::String("GET".to_string()),
                ),
                (
                    "url.path",
                    TraceAttributeValue::String("/api/test".to_string()),
                ),
            ],
        };

        emit_trace(&provider, &event, &correlation);

        // The span should be stored (not dropped)
        assert!(correlation.get_parent_ids("test.span").is_some());
    }

    #[test]
    fn emit_trace_end_span_ends_properly() {
        use ferron_observability::TraceAttributeValue;

        let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder().build();
        let correlation = CorrelationContext::new();

        // Start a span
        let start_event = TraceEvent::StartSpan {
            name: "test.span".to_string(),
            parent_span_id: None,
            attributes: vec![(
                "http.request.method",
                TraceAttributeValue::String("POST".to_string()),
            )],
        };
        emit_trace(&provider, &start_event, &correlation);

        // End the span with error
        let end_event = TraceEvent::EndSpan {
            name: "test.span".to_string(),
            error: Some("test error".to_string()),
            attributes: vec![("http.response.status_code", TraceAttributeValue::I64(500))],
        };
        emit_trace(&provider, &end_event, &correlation);

        // The span should be removed from the correlation context
        assert!(correlation.get_parent_ids("test.span").is_none());
    }

    #[test]
    fn emit_trace_end_span_without_error() {
        let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder().build();
        let correlation = CorrelationContext::new();

        let start_event = TraceEvent::StartSpan {
            name: "test.span".to_string(),
            parent_span_id: None,
            attributes: vec![],
        };
        emit_trace(&provider, &start_event, &correlation);

        let end_event = TraceEvent::EndSpan {
            name: "test.span".to_string(),
            error: None,
            attributes: vec![("http.response.status_code", TraceAttributeValue::I64(200))],
        };
        emit_trace(&provider, &end_event, &correlation);

        assert!(correlation.get_parent_ids("test.span").is_none());
    }

    #[test]
    fn emit_trace_end_span_on_unknown_name_does_nothing() {
        let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder().build();
        let correlation = CorrelationContext::new();

        // End a span that was never started — should not panic
        let end_event = TraceEvent::EndSpan {
            name: "unknown.span".to_string(),
            error: Some("should be ignored".to_string()),
            attributes: vec![],
        };
        emit_trace(&provider, &end_event, &correlation);
        assert!(correlation.get_parent_ids("unknown.span").is_none());
    }
}
