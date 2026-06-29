use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use ferron_observability::{
    LogAttributeValue, LogEvent, LogLevel, MetricAttributeValue, MetricEvent, MetricType,
    MetricValue,
};
use ferron_tls::observability;
use http_body_util::{BodyExt, Empty};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client as HyperClient;
use hyper_util::rt::TokioExecutor;
use parking_lot::RwLock;
use rustls::server::ResolvesServerCert;
use rustls::ClientConfig;
use rustls_pki_types::pem::PemObject;
use serde::Deserialize;
use tokio::sync::RwLock as TokioRwLock;

use crate::config::{TlsHttpConfig, TlsHttpOnDemandConfigData};

pub type CertifiedKeyLock = Arc<RwLock<Option<Arc<rustls::sign::CertifiedKey>>>>;
pub type ErrorMessageLock = Arc<RwLock<Option<String>>>;
pub type SniCertLock = Arc<TokioRwLock<HashMap<String, Arc<rustls::sign::CertifiedKey>>>>;

/// Emit a log event through the observability sink.
pub fn emit_log(
    event_sink: &Arc<ferron_observability::CompositeEventSink>,
    level: LogLevel,
    summary: &'static str,
    message: &str,
    target: &'static str,
    attributes: Vec<(&'static str, LogAttributeValue)>,
    error_message: ErrorMessageLock,
) {
    event_sink.emit(ferron_observability::Event::Log(LogEvent {
        level,
        message: message.to_string(),
        summary: summary.into(),
        target,
        attributes,
        trace_context: None,
    }));
    *error_message.write() = Some(message.to_string());
}

/// Emit a log event without setting the error_message lock.
fn emit_log_simple(
    event_sink: &Arc<ferron_observability::CompositeEventSink>,
    level: LogLevel,
    summary: &'static str,
    message: &str,
    target: &'static str,
    attributes: Vec<(&'static str, LogAttributeValue)>,
) {
    event_sink.emit(ferron_observability::Event::Log(LogEvent {
        level,
        message: message.to_string(),
        summary: summary.into(),
        target,
        attributes,
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
        trace_context: None,
    }));
}

#[derive(Deserialize)]
struct TlsHttpResponse {
    private_key: String,
    certificate: String,
}

/// Fetch a certificate for a specific domain from the given endpoint URL.
///
/// Appends `?domain=<encoded>` to the URL for domain-specific certificate
/// retrieval.
pub async fn fetch_cert_for_domain(
    url: &hyper::Uri,
    domain: &str,
    no_verification: bool,
    event_sink: &Arc<ferron_observability::CompositeEventSink>,
) -> Result<Arc<rustls::sign::CertifiedKey>, ()> {
    let hyper_client = build_hyper_client(no_verification)?;

    let endpoint_url = append_domain_to_url(url, domain).map_err(|_| ())?;
    let start = Instant::now();

    let request = hyper::Request::builder()
        .method("GET")
        .uri(endpoint_url)
        .body(Empty::<bytes::Bytes>::new())
        .map_err(|e| {
            emit_log_simple(
                event_sink,
                LogLevel::Warn,
                "TLS-HTTP request build failed",
                &format!("Failed to build HTTP request for `tls-http`: {e}"),
                "ferron-tls-http",
                vec![("error.message", LogAttributeValue::String(e.to_string()))],
            );
        })?;

    let response = hyper_client.request(request).await.map_err(|e| {
        let duration = start.elapsed().as_secs_f64();
        emit_metric(
            event_sink,
            "ferron.tls_http.request_duration_seconds",
            MetricValue::F64(duration),
            MetricType::Histogram(None),
            Some("s"),
            Some("HTTP request duration for TLS certificate endpoint"),
            vec![("status", MetricAttributeValue::StaticStr("error"))],
        );
        emit_log_simple(
            event_sink,
            LogLevel::Warn,
            "TLS-HTTP request failed",
            &format!("Failed to send HTTP request for `tls-http`: {e}"),
            "ferron-tls-http",
            vec![("error.message", LogAttributeValue::String(e.to_string()))],
        );
    })?;

    let duration = start.elapsed().as_secs_f64();
    emit_metric(
        event_sink,
        "ferron.tls_http.request_duration_seconds",
        MetricValue::F64(duration),
        MetricType::Histogram(None),
        Some("s"),
        Some("HTTP request duration for TLS certificate endpoint"),
        vec![("status", MetricAttributeValue::StaticStr("success"))],
    );
    emit_metric(
        event_sink,
        "ferron.tls_http.requests_total",
        MetricValue::U64(1),
        MetricType::Counter,
        None,
        Some("Total HTTP requests to TLS certificate endpoint"),
        vec![("status", MetricAttributeValue::StaticStr("success"))],
    );

    if !response.status().is_success() {
        emit_log_simple(
            event_sink,
            LogLevel::Warn,
            "TLS-HTTP endpoint error",
            &format!(
                "TLS certificate endpoint returned unsuccessful status: {}",
                response.status()
            ),
            "ferron-tls-http",
            vec![(
                "http.status_code",
                LogAttributeValue::I64(response.status().as_u16() as i64),
            )],
        );
        return Err(());
    }

    let body_bytes = response
        .into_body()
        .collect()
        .await
        .map_err(|e| {
            emit_log_simple(
                event_sink,
                LogLevel::Warn,
                "TLS-HTTP response read failed",
                &format!("Failed to read the HTTP response from TLS certificate endpoint: {e}"),
                "ferron-tls-http",
                vec![("error.message", LogAttributeValue::String(e.to_string()))],
            );
        })?
        .to_bytes();

    let body: TlsHttpResponse = serde_json::from_slice(&body_bytes).map_err(|e| {
        emit_log_simple(
            event_sink,
            LogLevel::Warn,
            "TLS-HTTP response parse failed",
            &format!("Failed to parse the HTTP response from TLS certificate endpoint: {e}"),
            "ferron-tls-http",
            vec![("error.message", LogAttributeValue::String(e.to_string()))],
        );
    })?;

    let cert_chain = rustls_pki_types::CertificateDer::pem_slice_iter(body.certificate.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| {
            emit_log_simple(
                event_sink,
                LogLevel::Warn,
                "TLS-HTTP certificate chain parse failed",
                &format!(
                    "Failed to parse the TLS certificate chain from TLS endpoint response: {e}"
                ),
                "ferron-tls-http",
                vec![("error.message", LogAttributeValue::String(e.to_string()))],
            );
        })?;

    let private_key = rustls_pki_types::PrivateKeyDer::from_pem_slice(body.private_key.as_bytes())
        .map_err(|e| {
            emit_log_simple(
                event_sink,
                LogLevel::Warn,
                "TLS-HTTP private key parse failed",
                &format!("Failed to parse the TLS private key from TLS endpoint response: {e}"),
                "ferron-tls-http",
                vec![("error.message", LogAttributeValue::String(e.to_string()))],
            );
        })?;

    let signing_key = rustls::crypto::aws_lc_rs::default_provider()
        .key_provider
        .load_private_key(private_key)
        .map_err(|e| {
            emit_log_simple(
                event_sink,
                LogLevel::Warn,
                "TLS-HTTP private key load failed",
                &format!("Failed to load the TLS private key: {e}"),
                "ferron-tls-http",
                vec![("error.message", LogAttributeValue::String(e.to_string()))],
            );
        })?;

    Ok(Arc::new(rustls::sign::CertifiedKey::new(
        cert_chain,
        signing_key,
    )))
}

/// Build a hyper HTTPS client.
fn build_hyper_client(
    no_verification: bool,
) -> Result<HyperClient<hyper_rustls::HttpsConnector<HttpConnector>, Empty<bytes::Bytes>>, ()> {
    let tls_config = build_rustls_client_config(no_verification).map_err(|_| {
        // Error already logged at call sites
    })?;
    Ok(HyperClient::builder(TokioExecutor::new()).build(
        hyper_rustls::HttpsConnectorBuilder::new()
            .with_tls_config(tls_config)
            .https_or_http()
            .enable_http1()
            .enable_http2()
            .build(),
    ))
}

/// Append `?domain=<encoded>` to a URI.
fn append_domain_to_url(
    url: &hyper::Uri,
    domain: &str,
) -> Result<hyper::Uri, Box<dyn std::error::Error + Send + Sync>> {
    let mut url_parts = url.clone().into_parts();
    let domain_encoded = urlencoding::encode(domain);
    let path_and_query_str = if let Some(path_and_query) = url_parts.path_and_query {
        let query = path_and_query.query();
        let query = if let Some(query) = query {
            format!("{}&domain={}", query, domain_encoded)
        } else {
            format!("domain={}", domain_encoded)
        };
        format!("{}?{}", path_and_query.path(), query)
    } else {
        format!("/?domain={}", domain_encoded)
    };
    url_parts.path_and_query = Some(path_and_query_str.parse()?);
    Ok(hyper::Uri::from_parts(url_parts)?)
}

/// SNI-aware resolver for on-demand mode.
///
/// Resolves certificates from an SNI-indexed map. If no certificate is found
/// for the requested hostname, sends an on-demand request through the channel
/// to trigger a background fetch.
#[derive(Debug)]
pub struct TlsHttpOnDemandResolver {
    sni_cert_lock: SniCertLock,
    on_demand_tx: async_channel::Sender<(String, u16)>,
    port: u16,
}

impl TlsHttpOnDemandResolver {
    pub fn new(
        sni_cert_lock: SniCertLock,
        on_demand_tx: async_channel::Sender<(String, u16)>,
        port: u16,
    ) -> Self {
        Self {
            sni_cert_lock,
            on_demand_tx,
            port,
        }
    }
}

impl ResolvesServerCert for TlsHttpOnDemandResolver {
    fn resolve(
        &self,
        client_hello: rustls::server::ClientHello<'_>,
    ) -> Option<Arc<rustls::sign::CertifiedKey>> {
        let sni = client_hello.server_name()?;

        if let Ok(map) = self.sni_cert_lock.try_read() {
            if let Some(cert) = map.get(sni) {
                return Some(cert.clone());
            }
        }

        let _ = self.on_demand_tx.try_send((sni.to_string(), self.port));

        None
    }
}

/// Background listener for on-demand certificate requests.
///
/// Receives `(sni_hostname, port)` requests from the resolver, checks the
/// approval endpoint (if configured), fetches the certificate, stores it in
/// the SNI map, and spawns a per-SNI refresh task.
pub async fn run_tls_http_background_task(
    on_demand_rx: async_channel::Receiver<(String, u16)>,
    on_demand_configs: Arc<TokioRwLock<Vec<TlsHttpOnDemandConfigData>>>,
    sni_cert_lock: SniCertLock,
    event_sink: Arc<ferron_observability::CompositeEventSink>,
) {
    let mut existing_combinations: std::collections::HashSet<(String, u16)> =
        std::collections::HashSet::new();

    while let Ok((sni_hostname, port)) = on_demand_rx.recv().await {
        if !existing_combinations.insert((sni_hostname.clone(), port)) {
            continue;
        }

        emit_log_simple(
            &event_sink,
            LogLevel::Info,
            "On-demand certificate requested",
            &format!("On-demand certificate requested for SNI {sni_hostname}:{port}"),
            "ferron-tls-http",
            vec![
                ("tls.sni", LogAttributeValue::String(sni_hostname.clone())),
                ("tls.port", LogAttributeValue::I64(port as i64)),
            ],
        );
        emit_metric(
            &event_sink,
            "ferron.tls_http.on_demand_requests_total",
            MetricValue::U64(1),
            MetricType::Counter,
            Some("{request}"),
            Some("Total on-demand certificate requests"),
            vec![],
        );

        let configs_guard = on_demand_configs.read().await;
        let config = configs_guard.iter().find(|c| c.port == port).cloned();
        drop(configs_guard);

        let Some(config) = config else {
            emit_log_simple(
                &event_sink,
                LogLevel::Error,
                "On-demand config not found",
                &format!(
                    "No on-demand configuration found for port {port}, request for {sni_hostname} ignored"
                ),
                "ferron-tls-http",
                vec![
                    ("tls.sni", LogAttributeValue::String(sni_hostname)),
                    ("tls.port", LogAttributeValue::I64(port as i64)),
                ],
            );
            continue;
        };

        match check_ask_endpoint(
            &sni_hostname,
            config.on_demand_ask.as_deref(),
            config.on_demand_ask_auth.as_deref(),
            config.on_demand_ask_no_verification,
        )
        .await
        {
            Ok(true) => {}
            Ok(false) => {
                emit_log_simple(
                    &event_sink,
                    LogLevel::Error,
                    "Certificate issuance denied",
                    &format!(
                        "The TLS certificate cannot be issued for \"{}\" hostname",
                        &sni_hostname
                    ),
                    "ferron-tls-http",
                    vec![("tls.sni", LogAttributeValue::String(sni_hostname))],
                );
                continue;
            }
            Err(err) => {
                emit_log_simple(
                    &event_sink,
                    LogLevel::Error,
                    "Ask endpoint error",
                    &format!(
                        "Error while determining if the TLS certificate can be issued for \
                         \"{sni_hostname}\" hostname: {err}"
                    ),
                    "ferron-tls-http",
                    vec![
                        ("tls.sni", LogAttributeValue::String(sni_hostname)),
                        ("error.message", LogAttributeValue::String(err.to_string())),
                    ],
                );
                continue;
            }
        }

        let certified_key = match fetch_cert_for_domain(
            &config.url,
            &sni_hostname,
            config.no_verification,
            &event_sink,
        )
        .await
        {
            Ok(key) => key,
            Err(()) => continue,
        };

        if let Some(first_cert) = certified_key.cert.first() {
            observability::emit_certificate_not_after(
                &event_sink,
                "http",
                &sni_hostname,
                first_cert,
            );
        }

        sni_cert_lock
            .write()
            .await
            .insert(sni_hostname.clone(), certified_key);

        emit_log_simple(
            &event_sink,
            LogLevel::Info,
            "On-demand certificate fetched",
            &format!("On-demand TLS certificate fetched for {sni_hostname}:{port}"),
            "ferron-tls-http",
            vec![
                ("tls.sni", LogAttributeValue::String(sni_hostname.clone())),
                ("tls.port", LogAttributeValue::I64(port as i64)),
            ],
        );

        let sni = sni_hostname;
        let sni_cert_lock2 = sni_cert_lock.clone();
        let refresh_config = config;
        let event_sink2 = event_sink.clone();

        tokio::spawn(async move {
            loop {
                tokio::time::sleep(refresh_config.refresh_interval).await;

                match fetch_cert_for_domain(
                    &refresh_config.url,
                    &sni,
                    refresh_config.no_verification,
                    &event_sink2,
                )
                .await
                {
                    Ok(new_cert) => {
                        let key_changed = {
                            let map = sni_cert_lock2.read().await;
                            match map.get(&sni) {
                                Some(existing) => {
                                    let old_leaf = existing.cert.first();
                                    let new_leaf = new_cert.cert.first();
                                    match (old_leaf, new_leaf) {
                                        (Some(a), Some(b)) => a.as_ref() != b.as_ref(),
                                        _ => true,
                                    }
                                }
                                None => true,
                            }
                        };

                        if key_changed {
                            if let Some(first_cert) = new_cert.cert.first() {
                                observability::emit_certificate_not_after(
                                    &event_sink2,
                                    "http",
                                    &sni,
                                    first_cert,
                                );
                            }
                            sni_cert_lock2.write().await.insert(sni.clone(), new_cert);
                            emit_log_simple(
                                &event_sink2,
                                LogLevel::Info,
                                "TLS-HTTP certificate refreshed",
                                &format!(
                                    "TLS certificate refreshed successfully from HTTP endpoint for {sni}"
                                ),
                                "ferron-tls-http",
                                vec![(
                                    "ferron.tls_http.host",
                                    LogAttributeValue::String(sni.clone()),
                                )],
                            );
                            emit_metric(
                                &event_sink2,
                                "ferron.tls_http.certificates_refreshed_total",
                                MetricValue::U64(1),
                                MetricType::Counter,
                                None,
                                Some("Total TLS certificate refresh outcomes from HTTP endpoint"),
                                vec![("status", MetricAttributeValue::StaticStr("success"))],
                            );
                        }
                    }
                    Err(()) => {
                        // fetch_cert_for_domain already logs errors
                    }
                }

                emit_metric(
                    &event_sink2,
                    "ferron.tls_http.next_refresh_seconds",
                    MetricValue::U64(refresh_config.refresh_interval.as_secs()),
                    MetricType::Gauge,
                    Some("s"),
                    Some("Seconds until next certificate refresh"),
                    vec![],
                );
            }
        });
    }
}

/// Check the approval endpoint to see if a certificate can be issued for the
/// given domain.
pub async fn check_ask_endpoint(
    domain: &str,
    on_demand_ask: Option<&str>,
    auth: Option<&str>,
    no_verification: bool,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let Some(on_demand_ask) = on_demand_ask else {
        return Ok(true);
    };

    let base_url = on_demand_ask.parse::<hyper::Uri>()?;
    let endpoint_url = append_domain_to_url(&base_url, domain)?;

    let client = HyperClient::builder(TokioExecutor::new()).build::<_, Empty<bytes::Bytes>>(
        hyper_rustls::HttpsConnectorBuilder::new()
            .with_tls_config(build_rustls_client_config(no_verification)?)
            .https_or_http()
            .enable_http1()
            .enable_http2()
            .build(),
    );

    let mut request_builder = hyper::Request::builder()
        .method(hyper::Method::GET)
        .uri(endpoint_url);

    if let Some(auth) = auth {
        request_builder = request_builder.header("Authorization", auth);
    }

    let request = request_builder.body(Empty::<hyper::body::Bytes>::new())?;
    let response = client.request(request).await?;

    Ok(response.status().is_success())
}

pub async fn fetch_tls_cert_loop(
    config: TlsHttpConfig,
    certified_key: CertifiedKeyLock,
    error_message: ErrorMessageLock,
    host: String,
    event_sink: Arc<ferron_observability::CompositeEventSink>,
) {
    let url_string = config.url.to_string();
    emit_log(
        &event_sink,
        LogLevel::Info,
        "TLS-HTTP polling started",
        &format!("TLS-HTTP certificate polling started for {url_string}"),
        "ferron-tls-http",
        vec![("ferron.tls_http.url", LogAttributeValue::String(url_string))],
        error_message.clone(),
    );

    let Ok(tls_config) = build_rustls_client_config(config.no_verification) else {
        emit_log(
            &event_sink,
            LogLevel::Warn,
            "TLS-HTTP client config build failed",
            "Can't build TLS client configuration for `tls-http`",
            "ferron-tls-http",
            Vec::new(),
            error_message.clone(),
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
                    "TLS-HTTP request build failed",
                    &format!("Failed to build HTTP request for `tls-http`: {e}"),
                    "ferron-tls-http",
                    vec![("error.message", LogAttributeValue::String(e.to_string()))],
                    error_message.clone(),
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
                    "TLS-HTTP request failed",
                    &format!("Failed to send HTTP request for `tls-http`: {e}"),
                    "ferron-tls-http",
                    vec![("error.message", LogAttributeValue::String(e.to_string()))],
                    error_message.clone(),
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
                "TLS-HTTP endpoint error",
                &format!(
                    "TLS certificate endpoint returned unsuccessful status: {}",
                    response.status()
                ),
                "ferron-tls-http",
                vec![(
                    "http.status_code",
                    LogAttributeValue::I64(response.status().as_u16() as i64),
                )],
                error_message.clone(),
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
                    "TLS-HTTP response read failed",
                    &format!("Failed to read the HTTP response from TLS certificate endpoint: {e}"),
                    "ferron-tls-http",
                    vec![("error.message", LogAttributeValue::String(e.to_string()))],
                    error_message.clone(),
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
                    "TLS-HTTP response parse failed",
                    &format!(
                        "Failed to parse the HTTP response from TLS certificate endpoint: {e}"
                    ),
                    "ferron-tls-http",
                    vec![("error.message", LogAttributeValue::String(e.to_string()))],
                    error_message.clone(),
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
                        "TLS-HTTP certificate chain parse failed",
                        &format!(
                        "Failed to parse the TLS certificate chain from TLS endpoint response: {e}"
                    ),
                        "ferron-tls-http",
                        vec![("error.message", LogAttributeValue::String(e.to_string()))],
                        error_message.clone(),
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
                        "TLS-HTTP private key parse failed",
                        &format!(
                            "Failed to parse the TLS private key from TLS endpoint response: {e}"
                        ),
                        "ferron-tls-http",
                        vec![("error.message", LogAttributeValue::String(e.to_string()))],
                        error_message.clone(),
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
                    "TLS-HTTP private key load failed",
                    &format!("Failed to load the TLS private key: {e}"),
                    "ferron-tls-http",
                    vec![("error.message", LogAttributeValue::String(e.to_string()))],
                    error_message.clone(),
                );
                continue;
            }
        };
        let certified_key_to_write =
            Arc::new(rustls::sign::CertifiedKey::new(cert_chain, signing_key));

        // Check if the certified key has actually changed
        let key_changed = match certified_key.read().as_ref() {
            Some(existing) => {
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
            if let Some(first_cert) = certified_key_to_write.cert.first() {
                observability::emit_certificate_not_after(&event_sink, "http", &host, first_cert);
            }

            *certified_key.write() = Some(certified_key_to_write);
            emit_log(
                &event_sink,
                LogLevel::Info,
                "TLS-HTTP certificate refreshed",
                "TLS certificate refreshed successfully from HTTP endpoint",
                "ferron-tls-http",
                vec![(
                    "ferron.tls_http.host",
                    LogAttributeValue::String(host.clone()),
                )],
                error_message.clone(),
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

/// Builds a Rustls client configuration.
///
/// If `no_verification` is true, all certificate validation is skipped
/// (for testing or internal endpoints).
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
