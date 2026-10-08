//! Active health check task for probing upstream backends.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use ferron_observability::LogAttributeValue;
use tokio::time::sleep;

use crate::types::health::{
    ExpectedStatusCodes, HealthCheckMethod, HealthCheckStateMap, UpstreamHealthCheckConfig,
};
use crate::types::upstream::{MtlsCredentials, ResolvedUpstream, SrvUpstream, Upstream};

use hyper_rustls::HttpsConnectorBuilder;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};

/// Resolver that sends every connection to one address when it is pinned.
///
/// An upstream with a hostname in its URL resolves to one backend per address,
/// and each backend carries its own health state. A probe has to reach the
/// address it reports on, so this resolver pins the TCP connection while the
/// request keeps the hostname as its authority. That keeps the `Host` header
/// and the TLS server name pointing at the hostname, which an upstream needs
/// for virtual hosting and for certificate verification. Without a pinned
/// address the resolver defers to the system resolver.
#[derive(Clone, Debug, Default)]
struct PinnedResolver {
    pinned: Option<std::net::SocketAddr>,
}

impl PinnedResolver {
    #[inline]
    fn pinned(addr: Option<std::net::SocketAddr>) -> Self {
        Self { pinned: addr }
    }
}

/// Address iterator produced by [`PinnedResolver`].
#[derive(Debug)]
struct PinnedAddrs(Vec<std::net::SocketAddr>);

impl Iterator for PinnedAddrs {
    type Item = std::net::SocketAddr;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.0.is_empty() {
            None
        } else {
            Some(self.0.remove(0))
        }
    }
}

impl tower_service::Service<hyper_util::client::legacy::connect::dns::Name> for PinnedResolver {
    type Response = PinnedAddrs;
    type Error = std::io::Error;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    #[inline]
    fn poll_ready(
        &mut self,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    #[inline]
    fn call(&mut self, name: hyper_util::client::legacy::connect::dns::Name) -> Self::Future {
        match self.pinned {
            Some(addr) => Box::pin(std::future::ready(Ok(PinnedAddrs(vec![addr])))),
            None => {
                let mut gai = hyper_util::client::legacy::connect::dns::GaiResolver::new();
                let resolved = tower_service::Service::call(&mut gai, name);
                Box::pin(async move { Ok(PinnedAddrs(resolved.await?.collect())) })
            }
        }
    }
}

/// Concrete HTTPS connector type used for health check probes.
type HttpsConnector = hyper_rustls::HttpsConnector<
    hyper_util::client::legacy::connect::HttpConnector<PinnedResolver>,
>;

#[inline]
fn build_default_https_connector(
    mtls: Option<Arc<MtlsCredentials>>,
    pinned: Option<std::net::SocketAddr>,
) -> HttpsConnector {
    let mut root_store = rustls::RootCertStore::empty();
    let mut found_any = false;

    match rustls_native_certs::load_native_certs() {
        cert_result if !cert_result.errors.is_empty() => {
            ferron_core::log_debug!(
                "Health check: native root CA loading errors: {:?}",
                cert_result.errors
            );
        }
        cert_result if cert_result.certs.is_empty() => {
            ferron_core::log_debug!("Health check: no native root CA certificates found");
        }
        cert_result => {
            for cert in cert_result.certs {
                if let Err(err) = root_store.add(cert) {
                    ferron_core::log_debug!("Health check: certificate parsing failed: {:?}", err);
                } else {
                    found_any = true;
                }
            }
        }
    }

    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    if !found_any {
        ferron_core::log_debug!(
            "Health check: using webpki-roots as fallback (no native root CAs available)"
        );
    }

    let builder = if root_store.is_empty() {
        rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("failed to initialize Rustls client builder")
        .with_root_certificates(rustls::RootCertStore::empty())
    } else {
        rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("failed to initialize Rustls client builder")
        .with_root_certificates(root_store)
    };
    let tls_config = if let Some(mtls) = mtls {
        builder
            .clone()
            .with_client_auth_cert(mtls.certs.clone(), mtls.key.clone_key())
            .unwrap_or_else(|_| builder.with_no_client_auth())
    } else {
        builder.with_no_client_auth()
    };

    HttpsConnectorBuilder::new()
        .with_tls_config(tls_config)
        .https_or_http()
        .enable_http1()
        .enable_http2()
        .wrap_connector(
            hyper_util::client::legacy::connect::HttpConnector::new_with_resolver(
                PinnedResolver::pinned(pinned),
            ),
        )
}

#[inline]
fn build_no_verify_https_connector(
    mtls: Option<Arc<MtlsCredentials>>,
    pinned: Option<std::net::SocketAddr>,
) -> HttpsConnector {
    #[derive(Debug)]
    struct NoServerVerifier;
    impl ServerCertVerifier for NoServerVerifier {
        #[inline]
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
        #[inline]
        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &rustls::DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }
        #[inline]
        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &rustls::DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }
        #[inline]
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

    let builder = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("failed to initialize Rustls client builder")
    .dangerous()
    .with_custom_certificate_verifier(std::sync::Arc::new(NoServerVerifier));
    let tls_config = if let Some(mtls) = mtls {
        builder
            .clone()
            .with_client_auth_cert(mtls.certs.clone(), mtls.key.clone_key())
            .unwrap_or_else(|_| builder.with_no_client_auth())
    } else {
        builder.with_no_client_auth()
    };

    HttpsConnectorBuilder::new()
        .with_tls_config(tls_config)
        .https_or_http()
        .enable_http1()
        .enable_http2()
        .wrap_connector(
            hyper_util::client::legacy::connect::HttpConnector::new_with_resolver(
                PinnedResolver::pinned(pinned),
            ),
        )
}

/// Type of upstream health check to perform.
#[derive(Hash, Eq, PartialEq)]
enum UpstreamHealthCheckType {
    Static(String),
    Srv((String, Vec<std::net::IpAddr>, u32)),
    StrictDns((String, u16, Vec<std::net::IpAddr>)),
}

/// Health check probe result.
#[derive(Clone, Debug)]
struct ProbeResult {
    status_code: Option<u16>,
    response_time: Duration,
    body: Option<Vec<u8>>,
    error: Option<String>,
}

/// Execute a single health check probe against an upstream.
///
/// Returns a `ProbeResult` containing the HTTP status, response time, optional body,
/// and any error that occurred.
#[inline]
async fn probe_upstream(
    upstream: &Arc<ResolvedUpstream>,
    config: &UpstreamHealthCheckConfig,
    mtls: Option<Arc<MtlsCredentials>>,
) -> ProbeResult {
    let start = SystemTime::now();
    let method = config.method.as_str();
    let uri = &config.uri;
    let timeout = config.timeout;
    let no_verification = config.no_verification;

    // The request keeps the hostname as its authority so the `Host` header and
    // the TLS server name stay correct, while the connector is pinned to the
    // address this probe reports on.
    let full_url = format!("{}{}", upstream.proxy_to.trim_end_matches('/'), uri);

    let result = execute_probe_request(
        &full_url,
        method,
        timeout,
        no_verification,
        config.body_match.as_deref(),
        mtls,
        upstream.connect_to,
    )
    .await;

    let response_time = start
        .elapsed()
        .unwrap_or(Duration::from_secs(timeout.as_secs() + 1));

    match result {
        Ok((status, body)) => ProbeResult {
            status_code: Some(status),
            response_time,
            body,
            error: None,
        },
        Err(e) => ProbeResult {
            status_code: None,
            response_time,
            body: None,
            error: Some(e),
        },
    }
}

/// Execute an HTTP probe request using hyper-util + hyper-rustls.
///
/// Supports both HTTP and HTTPS with native certificate store and webpki-roots fallback.
/// When `no_verification` is true, TLS certificate verification is disabled.
#[inline]
async fn execute_probe_request(
    url: &str,
    method: &str,
    timeout: Duration,
    no_verification: bool,
    body_match: Option<&str>,
    mtls: Option<Arc<MtlsCredentials>>,
    pinned: Option<std::net::SocketAddr>,
) -> Result<(u16, Option<Vec<u8>>), String> {
    use bytes::Bytes;
    use http_body_util::Full;
    use hyper::Request;

    let url_parsed_result: Result<http::Uri, _> =
        url.parse().map_err(|e| format!("Invalid URL: {e}"));
    let url_parsed = match url_parsed_result {
        Ok(uri) => uri,
        Err(e) => {
            if url.contains("://") {
                return Err(e);
            } else {
                let url = format!("http://{url}");
                url.parse::<http::Uri>()
                    .map_err(|e| format!("Invalid URL: {e}"))?
            }
        }
    };

    // Use cached client (the underlying connector supports both HTTP and HTTPS)
    let client = health_check_client(no_verification, mtls, pinned);
    let req = Request::builder()
        .method(method.to_uppercase().as_str())
        .uri(url_parsed)
        .header("User-Agent", "Ferron")
        .header("Connection", "close")
        .body(Full::new(Bytes::new()))
        .map_err(|e| format!("Failed to build request: {}", e))?;
    let resp = tokio::time::timeout(timeout, client.request(req)).await;

    let resp = match resp {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => return Err(format!("Request error: {}", e)),
        Err(_) => return Err("Request timeout".to_string()),
    };

    let status_code = resp.status().as_u16();

    // Only read body when necessary (GET + body_match present). This avoids
    // allocating and reading the full body when probes do not require it.
    let body = if method.eq_ignore_ascii_case("GET") && body_match.is_some() {
        use http_body_util::BodyExt;
        match resp.collect().await {
            Ok(body_bytes) => {
                let bytes = body_bytes.to_bytes();
                if bytes.is_empty() {
                    None
                } else {
                    Some(bytes.to_vec())
                }
            }
            Err(e) => return Err(format!("Body read error: {}", e)),
        }
    } else {
        None
    };

    Ok((status_code, body))
}

#[inline]
fn health_check_client(
    no_verification: bool,
    mtls: Option<Arc<MtlsCredentials>>,
    pinned: Option<std::net::SocketAddr>,
) -> hyper_util::client::legacy::Client<HttpsConnector, http_body_util::Full<bytes::Bytes>> {
    use hyper_util::client::legacy::Client;
    use hyper_util::rt::TokioExecutor;

    if no_verification {
        Client::builder(TokioExecutor::new()).build(build_no_verify_https_connector(mtls, pinned))
    } else {
        Client::builder(TokioExecutor::new()).build(build_default_https_connector(mtls, pinned))
    }
}

/// Process a probe result and update health check state.
#[allow(clippy::type_complexity)]
#[inline]
fn process_probe_result(
    upstream: &Arc<ResolvedUpstream>,
    config: &UpstreamHealthCheckConfig,
    result: &[ProbeResult],
    state_map: &HealthCheckStateMap,
    event_sink: &ferron_observability::CompositeEventSink,
    metrics_resolved_ip: bool,
) {
    if result.is_empty() {
        // Nothing to process!
        return;
    }

    let upstream_url = upstream.proxy_to.as_str();
    let mut state = state_map.entry(Arc::clone(upstream)).or_default();

    let now = SystemTime::now();
    let mut successes: usize = 0;
    let mut failures: usize = 0;

    for result in result {
        let probe_success = if let Some(status) = result.status_code {
            let status_ok = config.expect_status.matches(status);

            let time_ok = config
                .response_time_threshold
                .map(|threshold| result.response_time <= threshold)
                .unwrap_or(true);

            let body_ok = if config.method == HealthCheckMethod::Get {
                if let Some(ref body_match) = config.body_match {
                    if let Some(ref body) = result.body {
                        String::from_utf8_lossy(body).contains(body_match)
                    } else {
                        false
                    }
                } else {
                    true
                }
            } else {
                true
            };

            status_ok && time_ok && body_ok
        } else {
            false
        };

        let mut health_attrs = Vec::with_capacity(4);
        health_attrs.push((
            "upstream.address",
            LogAttributeValue::String(upstream_url.to_string()),
        ));
        health_attrs.push((
            "ferron.proxy.backend_url",
            LogAttributeValue::String(upstream_url.to_string()),
        ));
        if let Some(ref unix_path) = upstream.proxy_unix {
            health_attrs.push((
                "ferron.proxy.backend_unix_path",
                LogAttributeValue::String(unix_path.clone()),
            ));
        }
        if let Some(ref connect_to) = upstream.connect_to {
            health_attrs.push((
                "ferron.proxy.backend_resolved_ip",
                LogAttributeValue::String(connect_to.to_string()),
            ));
        }
        let upstream_log_id = if let Some(ref connect_to) = upstream.connect_to {
            format!("{upstream_url} (at {connect_to})")
        } else {
            upstream_url.to_owned()
        };

        if probe_success {
            successes += 1;
            if state.is_healthy {
                state.consecutive_pass_count = 0;
            } else {
                state.consecutive_pass_count += 1;
                if state.consecutive_pass_count >= config.consecutive_passes {
                    state.is_healthy = true;
                    state.consecutive_pass_count = 0;
                    state.consecutive_fail_count = 0;
                    event_sink.emit(ferron_observability::Event::Log(
                        ferron_observability::LogEvent {
                            level: ferron_observability::LogLevel::Info,
                            message: format!(
                                "Upstream {} recovered after {} consecutive successes",
                                upstream_log_id, config.consecutive_passes
                            ),
                            summary: "Upstream recovered".into(),
                            target: super::LOG_TARGET,
                            attributes: health_attrs,
                            trace_context: None,
                        },
                    ));
                }
            }
            state.last_success_time = Some(now);
            state.last_probe_status = result.status_code;
            state.last_probe_error = None;
        } else {
            failures += 1;
            state.consecutive_fail_count += 1;
            state.consecutive_pass_count = 0;

            if state.is_healthy && state.consecutive_fail_count >= config.consecutive_fails {
                state.is_healthy = false;
                let error_msg = result.error.clone().unwrap_or_else(|| {
                    format!(
                        "Status {} (expected {})",
                        result.status_code.unwrap_or(0),
                        match &config.expect_status {
                            ExpectedStatusCodes::Successful => "2xx",
                            ExpectedStatusCodes::SuccessfulOrRedirect => "2xx/3xx",
                            _ => "custom",
                        }
                    )
                });
                event_sink.emit(ferron_observability::Event::Log(
                    ferron_observability::LogEvent {
                        level: ferron_observability::LogLevel::Warn,
                        message: format!(
                            "Upstream {} marked unhealthy: {} ({}/{})",
                            upstream_log_id,
                            error_msg,
                            state.consecutive_fail_count,
                            config.consecutive_fails
                        ),
                        summary: "Upstream marked unhealthy".into(),
                        target: super::LOG_TARGET,
                        attributes: health_attrs,
                        trace_context: None,
                    },
                ));
            }

            state.last_failure_time = Some(now);
            state.last_probe_error = result.error.clone();
        }
    }

    use ferron_observability::{Event, MetricAttributeValue, MetricEvent, MetricType, MetricValue};

    let duration_secs = result
        .iter()
        .map(|r| r.response_time.as_secs_f64())
        .sum::<f64>()
        / result.len() as f64; // Average of all response times (result.len() == 0 would return earlier anyway)

    // Same attribute set as the other backend scoped proxy metrics, so the
    // metric name maps to one Prometheus series per backend.
    let mut health_attrs = Vec::with_capacity(4);
    health_attrs.push((
        "ferron.proxy.backend_url",
        MetricAttributeValue::String(upstream_url.to_string()),
    ));
    if let Some(ref unix_path) = upstream.proxy_unix {
        health_attrs.push((
            "ferron.proxy.backend_unix_path",
            MetricAttributeValue::String(unix_path.clone()),
        ));
    }
    health_attrs.extend(crate::metrics::resolved_ip_attrs(
        metrics_resolved_ip,
        upstream,
    ));

    event_sink.emit(Event::Metric(MetricEvent {
        name: "ferron.proxy.health.duration",
        attributes: health_attrs.clone(),
        ty: MetricType::Histogram(None),
        value: MetricValue::F64(duration_secs),
        unit: Some("s"),
        description: Some("Duration of active health check probe."),
        trace_context: None,
    }));

    if successes == 0 {
        event_sink.emit(Event::Metric(MetricEvent {
            name: "ferron.proxy.health.failure",
            attributes: health_attrs,
            ty: MetricType::Counter,
            value: MetricValue::U64(1),
            unit: Some("{probe}"),
            description: Some("Failed active health check probes."),
            trace_context: None,
        }));
        crate::metrics::emit_backend_unhealthy(
            event_sink,
            upstream,
            "active",
            None,
            metrics_resolved_ip,
        );
    } else if failures == 0 {
        event_sink.emit(Event::Metric(MetricEvent {
            name: "ferron.proxy.health.success",
            attributes: health_attrs,
            ty: MetricType::Counter,
            value: MetricValue::U64(1),
            unit: Some("{probe}"),
            description: Some("Successful active health check probes."),
            trace_context: None,
        }));
    } else {
        let ratio = if successes + failures == 0 {
            0.0
        } else {
            (successes) as f64 / (successes + failures) as f64
        };
        event_sink.emit(Event::Metric(MetricEvent {
            name: "ferron.proxy.health.partial_success",
            attributes: health_attrs,
            ty: MetricType::Counter,
            value: MetricValue::F64(ratio),
            unit: Some("{probe}"),
            description: Some("Partially successful active health check probes (percentage of successes in 0.0-1.0 scale)."),
            trace_context: None,
        }));
    }
}

/// Check if an upstream URL is healthy based on active health checks.
///
/// Returns true if health checks are disabled for this upstream or if it's currently healthy.
/// Returns false if health checks are enabled and the upstream is marked unhealthy.
#[inline]
pub fn is_upstream_healthy(
    state_map: &HealthCheckStateMap,
    upstream: &Arc<ResolvedUpstream>,
) -> bool {
    state_map
        .get(upstream)
        .map(|state| state.is_healthy)
        .unwrap_or(true)
}
///
/// This task will periodically probe all upstreams with health checks enabled
/// and update the health state map accordingly.
///
/// The task is spawned on the provided runtime handle (typically the secondary runtime)
/// to avoid requiring a Tokio context in the pipeline stage.
#[inline]
pub fn spawn_health_check_task(
    upstreams: Vec<Upstream>,
    state_map: HealthCheckStateMap,
    runtime_handle: &tokio::runtime::Handle,
    event_sink: Arc<ferron_observability::CompositeEventSink>,
    metrics_resolved_ip: bool,
) -> tokio::task::JoinHandle<()> {
    runtime_handle.spawn(async move {
        let mut probe_configs: Vec<(
            UpstreamHealthCheckType,
            UpstreamHealthCheckConfig,
            Option<Arc<MtlsCredentials>>,
        )> = Vec::new();

        for upstream in &upstreams {
            match upstream {
                Upstream::Static(cfg) => {
                    if cfg.health_check_config.enabled {
                        if let Some((host, port)) =
                            crate::types::strict_dns::parse_host_port(&cfg.url)
                        {
                            let is_ip = host.parse::<std::net::IpAddr>().is_ok();
                            let is_logical = cfg.logical_dns;
                            if !is_ip && !is_logical && !cfg.url.starts_with("unix:") {
                                probe_configs.push((
                                    UpstreamHealthCheckType::StrictDns((
                                        host,
                                        port,
                                        cfg.dns_servers.clone(),
                                    )),
                                    cfg.health_check_config.clone(),
                                    cfg.mtls.clone(),
                                ));
                            } else {
                                probe_configs.push((
                                    UpstreamHealthCheckType::Static(cfg.url.clone()),
                                    cfg.health_check_config.clone(),
                                    cfg.mtls.clone(),
                                ));
                            }
                        } else {
                            probe_configs.push((
                                UpstreamHealthCheckType::Static(cfg.url.clone()),
                                cfg.health_check_config.clone(),
                                cfg.mtls.clone(),
                            ));
                        }
                    }
                }
                Upstream::Srv(cfg) => {
                    if cfg.health_check_config.enabled {
                        probe_configs.push((
                            UpstreamHealthCheckType::Srv((
                                cfg.srv_name.clone(),
                                cfg.dns_servers.clone(),
                                cfg.weight,
                            )),
                            cfg.health_check_config.clone(),
                            cfg.mtls.clone(),
                        ));
                    }
                }
            }
        }

        if probe_configs.is_empty() {
            std::future::pending::<()>().await;
            return;
        }

        for probe_config in &probe_configs {
            let (upstream_type, config, _) = probe_config;
            let upstream_desc = match upstream_type {
                UpstreamHealthCheckType::Static(url) => format!("Static({})", url),
                UpstreamHealthCheckType::Srv((srv_name, _, _)) => format!("SRV({})", srv_name),
                UpstreamHealthCheckType::StrictDns((host, port, _)) => {
                    format!("StrictDns({}:{})", host, port)
                }
            };
            event_sink.emit(ferron_observability::Event::Log(
                ferron_observability::LogEvent {
                    level: ferron_observability::LogLevel::Debug,
                    message: format!(
                        "Initializing health check for upstream {}: {} {}",
                        upstream_desc,
                        config.method.as_str(),
                        config.uri
                    ),
                    summary: "Initializing health check".into(),
                    target: super::LOG_TARGET,
                    attributes: vec![
                        (
                            "ferron.proxy.health.address",
                            ferron_observability::LogAttributeValue::String(upstream_desc),
                        ),
                        (
                            "ferron.proxy.health.method",
                            ferron_observability::LogAttributeValue::String(
                                config.method.as_str().to_string(),
                            ),
                        ),
                        (
                            "ferron.proxy.health.uri",
                            ferron_observability::LogAttributeValue::String(config.uri.clone()),
                        ),
                    ],
                    trace_context: None,
                },
            ));
        }

        let mut last_probe_times: HashMap<Arc<ResolvedUpstream>, tokio::time::Instant> =
            HashMap::new();
        loop {
            let now = tokio::time::Instant::now();
            let mut next_wake = now + Duration::from_secs(60);

            let mut probes_due = Vec::new();

            for (upstream_url, config, mtls) in &probe_configs {
                let upstreams = match upstream_url {
                    UpstreamHealthCheckType::Static(url) => {
                        vec![Arc::new(ResolvedUpstream {
                            proxy_to: url.clone(),
                            connect_to: None,
                            proxy_unix: None,
                            inner: crate::types::upstream::UpstreamInner {
                                weight: 1,
                                mtls: None,
                                priority: 0,
                                connection_timeout: None,
                                idle_timeout: Duration::from_secs(60),
                                limit: None,
                            },
                            dns_status: crate::types::upstream::DnsResolutionStatus::NotApplicable,
                        })]
                    }
                    UpstreamHealthCheckType::Srv((srv_name, dns_servers, weight)) => {
                        let timeout_result = tokio::time::timeout(
                            Duration::from_secs(5),
                            crate::types::srv::resolve_srv_inner(SrvUpstream {
                                srv_name: srv_name.clone(),
                                dns_servers: dns_servers.clone(),
                                // Use default health check config (SrvUpstream is only used for resolving SRV records)
                                health_check_config: UpstreamHealthCheckConfig::default(),
                                inner: crate::types::upstream::UpstreamInner {
                                    weight: *weight,
                                    limit: None,
                                    // mTLS isn't applicable for resolution only
                                    mtls: None,
                                    priority: 0,
                                    connection_timeout: None,
                                    idle_timeout: Duration::from_secs(60),
                                },
                            }),
                        )
                        .await;
                        if timeout_result.is_err() {
                            event_sink.emit(ferron_observability::Event::Log(
                                ferron_observability::LogEvent {
                                    level: ferron_observability::LogLevel::Warn,
                                    message: format!(
                                        "Timeout (5s) while resolving SRV record for upstream {}",
                                        srv_name
                                    ),
                                    summary: "Timeout while resolving SRV record".into(),
                                    target: super::LOG_TARGET,
                                    attributes: vec![(
                                        "dns.name",
                                        ferron_observability::LogAttributeValue::String(
                                            srv_name.to_string(),
                                        ),
                                    )],
                                    trace_context: None,
                                },
                            ));
                        }
                        timeout_result
                            .unwrap_or_default()
                            .into_iter()
                            .map(|(upstream, _, _)| upstream)
                            .collect()
                    }
                    UpstreamHealthCheckType::StrictDns((host, port, dns_servers)) => {
                        let temp_cfg = crate::types::upstream::StaticUpstream {
                            url: format!("http://{}:{}", host, port),
                            unix_socket: None,
                            inner: crate::types::upstream::UpstreamInner {
                                limit: None,
                                weight: 1,
                                mtls: None,
                                priority: 0,
                                connection_timeout: None,
                                idle_timeout: Duration::from_secs(60),
                            },
                            health_check_config:
                                crate::types::health::UpstreamHealthCheckConfig::default(),
                            logical_dns: false,
                            dns_servers: dns_servers.clone(),
                        };
                        let timeout_result = tokio::time::timeout(
                            Duration::from_secs(5),
                            crate::types::strict_dns::resolve_strict_dns(temp_cfg),
                        )
                        .await;
                        if timeout_result.is_err() {
                            event_sink.emit(ferron_observability::Event::Log(
                                ferron_observability::LogEvent {
                                    level: ferron_observability::LogLevel::Warn,
                                    message: format!(
                                        "Timeout (5s) while resolving DNS for upstream {}:{}",
                                        host, port
                                    ),
                                    summary: "Timeout while resolving DNS".into(),
                                    target: super::LOG_TARGET,
                                    attributes: vec![(
                                        "dns.name",
                                        ferron_observability::LogAttributeValue::String(
                                            host.to_string(),
                                        ),
                                    )],
                                    trace_context: None,
                                },
                            ));
                        }
                        timeout_result.unwrap_or_default().into_iter().collect()
                    }
                };

                for upstream in upstreams {
                    let last_probe = last_probe_times.get(&upstream);
                    let elapsed = last_probe.map_or(Duration::MAX, |t| t.elapsed());

                    if elapsed >= config.interval {
                        probes_due.push((upstream.clone(), config.clone(), mtls.clone()));
                        next_wake = now;
                    } else {
                        let time_until_due = config.interval - elapsed;
                        if time_until_due < next_wake - now {
                            next_wake = now + time_until_due;
                        }
                    }
                }
            }

            if !probes_due.is_empty() {
                let mut probe_tasks = Vec::new();
                let aggregated_probe_results: Arc<
                    dashmap::DashMap<
                        (String, Option<String>),
                        (
                            Arc<ResolvedUpstream>,
                            UpstreamHealthCheckConfig,
                            Vec<ProbeResult>,
                        ),
                    >,
                > = Arc::new(Default::default());

                for (upstream, config, mtls) in probes_due {
                    let state_map = Arc::clone(&state_map);
                    let probe_target = Arc::clone(&upstream);
                    let aggregated_probe_results = Arc::clone(&aggregated_probe_results);

                    last_probe_times.insert(upstream, now);

                    let event_sink = event_sink.clone();
                    probe_tasks.push(tokio::spawn(async move {
                        let result = probe_upstream(&probe_target, &config, mtls).await;
                        if metrics_resolved_ip {
                            process_probe_result(
                                &probe_target,
                                &config,
                                &[result],
                                &state_map,
                                &event_sink,
                                metrics_resolved_ip,
                            );
                        } else {
                            let key = (
                                probe_target.proxy_to.clone(),
                                probe_target.proxy_unix.clone(),
                            );
                            aggregated_probe_results
                                .entry(key)
                                .or_insert_with(|| {
                                    (probe_target.clone(), config.clone(), Vec::new())
                                })
                                .2
                                .push(result);
                        }
                    }));
                }

                for task in probe_tasks {
                    let _ = task.await;
                }

                for value in aggregated_probe_results.iter() {
                    let (upstream, config, results) = &*value;
                    process_probe_result(
                        upstream,
                        config,
                        results,
                        &state_map,
                        &event_sink,
                        metrics_resolved_ip,
                    );
                }
            }

            sleep(next_wake - now).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::health::{HealthCheckMethod, HealthCheckState};
    use dashmap::DashMap;
    use rustc_hash::FxBuildHasher;

    fn backend(url: &str) -> Arc<ResolvedUpstream> {
        Arc::new(ResolvedUpstream {
            proxy_to: url.to_string(),
            connect_to: None,
            proxy_unix: None,
            inner: crate::types::upstream::UpstreamInner {
                weight: 1,
                mtls: None,
                priority: 0,
                connection_timeout: None,
                idle_timeout: Duration::from_secs(60),
                limit: None,
            },
            dns_status: crate::types::upstream::DnsResolutionStatus::NotApplicable,
        })
    }

    /// A backend for one address behind a hostname, as strict DNS produces.
    fn backend_at(url: &str, ip: &str) -> Arc<ResolvedUpstream> {
        Arc::new(ResolvedUpstream {
            connect_to: Some(format!("{ip}:80").parse().expect("test address")),
            ..(*backend(url)).clone()
        })
    }

    #[test]
    fn test_health_state_transition_to_unhealthy() {
        let event_sink = ferron_observability::CompositeEventSink::new(vec![]);
        let state_map: HealthCheckStateMap = Arc::new(DashMap::with_hasher(FxBuildHasher));
        let config = UpstreamHealthCheckConfig {
            consecutive_fails: 2,
            ..Default::default()
        };

        let result = vec![ProbeResult {
            status_code: Some(500),
            response_time: Duration::from_millis(100),
            body: None,
            error: None,
        }];

        process_probe_result(
            &backend("http://localhost:8080"),
            &config,
            &result,
            &state_map,
            &event_sink,
            false,
        );
        process_probe_result(
            &backend("http://localhost:8080"),
            &config,
            &result,
            &state_map,
            &event_sink,
            false,
        );

        let state = state_map.get(&backend("http://localhost:8080")).unwrap();
        assert!(!state.is_healthy);
        assert_eq!(state.consecutive_fail_count, 2);
    }

    #[test]
    fn test_health_state_recovery() {
        let event_sink = ferron_observability::CompositeEventSink::new(vec![]);
        let state_map: HealthCheckStateMap = Arc::new(DashMap::with_hasher(FxBuildHasher));
        let config = UpstreamHealthCheckConfig {
            consecutive_fails: 2,
            consecutive_passes: 2,
            ..Default::default()
        };

        let fail_result = vec![ProbeResult {
            status_code: Some(500),
            response_time: Duration::from_millis(100),
            body: None,
            error: None,
        }];
        process_probe_result(
            &backend("http://localhost:8080"),
            &config,
            &fail_result,
            &state_map,
            &event_sink,
            false,
        );
        process_probe_result(
            &backend("http://localhost:8080"),
            &config,
            &fail_result,
            &state_map,
            &event_sink,
            false,
        );

        let success_result = vec![ProbeResult {
            status_code: Some(200),
            response_time: Duration::from_millis(100),
            body: None,
            error: None,
        }];

        process_probe_result(
            &backend("http://localhost:8080"),
            &config,
            &success_result,
            &state_map,
            &event_sink,
            false,
        );
        process_probe_result(
            &backend("http://localhost:8080"),
            &config,
            &success_result,
            &state_map,
            &event_sink,
            false,
        );

        let state = state_map.get(&backend("http://localhost:8080")).unwrap();
        assert!(state.is_healthy);
        assert_eq!(state.consecutive_pass_count, 0);
        assert_eq!(state.consecutive_fail_count, 0);
    }

    #[test]
    fn test_response_time_threshold() {
        let event_sink = ferron_observability::CompositeEventSink::new(vec![]);
        let state_map: HealthCheckStateMap = Arc::new(DashMap::with_hasher(FxBuildHasher));
        let config = UpstreamHealthCheckConfig {
            response_time_threshold: Some(Duration::from_millis(50)),
            consecutive_fails: 1,
            ..Default::default()
        };

        let result_fast = vec![ProbeResult {
            status_code: Some(200),
            response_time: Duration::from_millis(30),
            body: None,
            error: None,
        }];
        process_probe_result(
            &backend("http://localhost:8080"),
            &config,
            &result_fast,
            &state_map,
            &event_sink,
            false,
        );

        {
            let state = state_map.get(&backend("http://localhost:8080")).unwrap();
            assert!(state.is_healthy);
        }

        let result_slow = vec![ProbeResult {
            status_code: Some(200),
            response_time: Duration::from_millis(100),
            body: None,
            error: None,
        }];
        process_probe_result(
            &backend("http://localhost:8080"),
            &config,
            &result_slow,
            &state_map,
            &event_sink,
            false,
        );

        let state = state_map.get(&backend("http://localhost:8080")).unwrap();
        assert!(!state.is_healthy);
        assert_eq!(state.consecutive_fail_count, 1);
    }

    #[test]
    fn test_body_match_success() {
        let event_sink = ferron_observability::CompositeEventSink::new(vec![]);
        let state_map: HealthCheckStateMap = Arc::new(DashMap::with_hasher(FxBuildHasher));
        let config = UpstreamHealthCheckConfig {
            body_match: Some("ok".to_string()),
            method: HealthCheckMethod::Get,
            consecutive_fails: 1,
            ..Default::default()
        };

        let result = vec![ProbeResult {
            status_code: Some(200),
            response_time: Duration::from_millis(50),
            body: Some(b"status: ok".to_vec()),
            error: None,
        }];
        process_probe_result(
            &backend("http://localhost:8080"),
            &config,
            &result,
            &state_map,
            &event_sink,
            false,
        );

        let state = state_map.get(&backend("http://localhost:8080")).unwrap();
        assert!(state.is_healthy);
    }

    #[test]
    fn test_body_match_failure() {
        let event_sink = ferron_observability::CompositeEventSink::new(vec![]);
        let state_map: HealthCheckStateMap = Arc::new(DashMap::with_hasher(FxBuildHasher));
        let config = UpstreamHealthCheckConfig {
            body_match: Some("ok".to_string()),
            method: HealthCheckMethod::Get,
            consecutive_fails: 1,
            ..Default::default()
        };

        let result = vec![ProbeResult {
            status_code: Some(200),
            response_time: Duration::from_millis(50),
            body: Some(b"status: fail".to_vec()),
            error: None,
        }];
        process_probe_result(
            &backend("http://localhost:8080"),
            &config,
            &result,
            &state_map,
            &event_sink,
            false,
        );

        let state = state_map.get(&backend("http://localhost:8080")).unwrap();
        assert!(!state.is_healthy);
    }

    #[test]
    fn test_is_upstream_healthy() {
        let state_map: HealthCheckStateMap = Arc::new(DashMap::with_hasher(FxBuildHasher));

        assert!(is_upstream_healthy(
            &state_map,
            &backend("http://localhost:8080")
        ));

        state_map.insert(
            backend("http://localhost:8080"),
            HealthCheckState {
                is_healthy: false,
                ..Default::default()
            },
        );

        assert!(!is_upstream_healthy(
            &state_map,
            &backend("http://localhost:8080")
        ));
    }

    #[test]
    fn health_state_is_tracked_per_resolved_address() {
        let state_map: HealthCheckStateMap = Arc::new(DashMap::with_hasher(FxBuildHasher));
        let event_sink = ferron_observability::CompositeEventSink::new(vec![]);
        let config = UpstreamHealthCheckConfig {
            consecutive_fails: 1,
            ..Default::default()
        };
        let failure = vec![ProbeResult {
            status_code: Some(503),
            response_time: Duration::from_millis(10),
            body: None,
            error: None,
        }];

        // Two addresses behind one hostname, which is what strict DNS produces.
        let first = backend_at("http://backend:3000", "10.0.0.1");
        let second = backend_at("http://backend:3000", "10.0.0.2");

        process_probe_result(&first, &config, &failure, &state_map, &event_sink, false);

        assert!(!is_upstream_healthy(&state_map, &first));
        assert!(
            is_upstream_healthy(&state_map, &second),
            "one unreachable address must not mark the other address behind the same hostname unhealthy"
        );
    }

    #[test]
    fn a_hostname_without_addresses_keys_on_the_configured_url() {
        let state_map: HealthCheckStateMap = Arc::new(DashMap::with_hasher(FxBuildHasher));
        let event_sink = ferron_observability::CompositeEventSink::new(vec![]);
        let config = UpstreamHealthCheckConfig {
            consecutive_fails: 1,
            ..Default::default()
        };
        let failure = vec![ProbeResult {
            status_code: Some(503),
            response_time: Duration::from_millis(10),
            body: None,
            error: None,
        }];

        let upstream = backend("http://backend:3000");
        process_probe_result(&upstream, &config, &failure, &state_map, &event_sink, false);

        assert!(!is_upstream_healthy(&state_map, &upstream));
        assert!(
            is_upstream_healthy(&state_map, &backend("http://other:3000")),
            "a different upstream must keep its own state"
        );
    }
}
