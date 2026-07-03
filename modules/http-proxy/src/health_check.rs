//! Active health check task for probing upstream backends.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use tokio::time::sleep;

use crate::types::health::{
    ExpectedStatusCodes, HealthCheckMethod, HealthCheckStateMap, UpstreamHealthCheckConfig,
};
use crate::types::upstream::{MtlsCredentials, SrvUpstreamData, Upstream};

use hyper_rustls::HttpsConnectorBuilder;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};

/// Concrete HTTPS connector type used for health check probes.
type HttpsConnector =
    hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>;

fn build_default_https_connector(mtls: Option<Arc<MtlsCredentials>>) -> HttpsConnector {
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
        rustls::ClientConfig::builder().with_root_certificates(rustls::RootCertStore::empty())
    } else {
        rustls::ClientConfig::builder().with_root_certificates(root_store)
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
        .build()
}

fn build_no_verify_https_connector(mtls: Option<Arc<MtlsCredentials>>) -> HttpsConnector {
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
            _dss: &rustls::DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }
        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &rustls::DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
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

    let builder = rustls::ClientConfig::builder()
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
        .build()
}

/// Callback invoked when a backend is marked unhealthy by active health check.
/// Arguments: (backend_url, is_active_health_check=true)
pub type UnhealthyCallback = Arc<dyn Fn(&str, bool) + Send + Sync>;

/// Type of upstream health check to perform.
#[derive(Hash, Eq, PartialEq)]
enum UpstreamHealthCheckType {
    Static(String),
    #[cfg(feature = "srv-lookup")]
    Srv((String, Vec<std::net::IpAddr>, u32)),
    #[cfg(feature = "srv-lookup")]
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
async fn probe_upstream(
    upstream_url: &str,
    config: &UpstreamHealthCheckConfig,
    mtls: Option<Arc<MtlsCredentials>>,
) -> ProbeResult {
    let start = SystemTime::now();
    let method = config.method.as_str();
    let uri = &config.uri;
    let timeout = config.timeout;
    let no_verification = config.no_verification;

    let full_url = format!("{}{}", upstream_url.trim_end_matches('/'), uri);

    let result = execute_probe_request(
        &full_url,
        method,
        timeout,
        no_verification,
        config.body_match.as_deref(),
        mtls,
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
async fn execute_probe_request(
    url: &str,
    method: &str,
    timeout: Duration,
    no_verification: bool,
    body_match: Option<&str>,
    mtls: Option<Arc<MtlsCredentials>>,
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
                // Maybe let's try prepending "http://"?
                let url = format!("http://{url}");
                url.parse::<http::Uri>()
                    .map_err(|e| format!("Invalid URL: {e}"))?
            }
        }
    };

    // Use cached client — the underlying connector supports both HTTP and HTTPS
    let client = health_check_client(no_verification, mtls);
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

fn health_check_client(
    no_verification: bool,
    mtls: Option<Arc<MtlsCredentials>>,
) -> hyper_util::client::legacy::Client<HttpsConnector, http_body_util::Full<bytes::Bytes>> {
    use hyper_util::client::legacy::Client;
    use hyper_util::rt::TokioExecutor;

    if no_verification {
        Client::builder(TokioExecutor::new()).build(build_no_verify_https_connector(mtls))
    } else {
        Client::builder(TokioExecutor::new()).build(build_default_https_connector(mtls))
    }
}

/// Process a probe result and update health check state.
#[allow(clippy::type_complexity)]
fn process_probe_result(
    upstream_url: &str,
    config: &UpstreamHealthCheckConfig,
    result: &ProbeResult,
    state_map: &HealthCheckStateMap,
    on_unhealthy: Option<&(dyn Fn(&str, bool) + Send + Sync)>,
    event_sink: &ferron_observability::CompositeEventSink,
) {
    let mut state = state_map.entry(upstream_url.to_string()).or_default();

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

    // Emit health check metrics
    use ferron_observability::{Event, MetricAttributeValue, MetricEvent, MetricType, MetricValue};
    let duration_secs = result.response_time.as_secs_f64();
    let health_attrs = vec![(
        "ferron.proxy.backend_url",
        MetricAttributeValue::String(upstream_url.to_string()),
    )];

    event_sink.emit(Event::Metric(MetricEvent {
        name: "ferron.proxy.health.duration",
        attributes: health_attrs.clone(),
        ty: MetricType::Histogram(None),
        value: MetricValue::F64(duration_secs),
        unit: Some("s"),
        description: Some("Duration of active health check probe."),
        trace_context: None,
    }));

    if probe_success {
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
        event_sink.emit(Event::Metric(MetricEvent {
            name: "ferron.proxy.health.failure",
            attributes: health_attrs,
            ty: MetricType::Counter,
            value: MetricValue::U64(1),
            unit: Some("{probe}"),
            description: Some("Failed active health check probes."),
            trace_context: None,
        }));
    }

    let now = SystemTime::now();
    let _was_healthy = state.is_healthy;

    if probe_success {
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
                            upstream_url, config.consecutive_passes
                        ),
                        summary: "Upstream recovered".into(),
                        target: super::LOG_TARGET,
                        attributes: vec![(
                            "upstream.address",
                            ferron_observability::LogAttributeValue::String(
                                upstream_url.to_string(),
                            ),
                        )],
                        trace_context: None,
                    },
                ));
            }
        }
        state.last_success_time = Some(now);
        state.last_probe_status = result.status_code;
        state.last_probe_error = None;
    } else {
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
                        upstream_url,
                        error_msg,
                        state.consecutive_fail_count,
                        config.consecutive_fails
                    ),
                    summary: "Upstream marked unhealthy".into(),
                    target: super::LOG_TARGET,
                    attributes: vec![(
                        "upstream.address",
                        ferron_observability::LogAttributeValue::String(upstream_url.to_string()),
                    )],
                    trace_context: None,
                },
            ));
            if let Some(callback) = on_unhealthy {
                callback(upstream_url, true);
            }
        }

        state.last_failure_time = Some(now);
        state.last_probe_error = result.error.clone();
    }
}

/// Check if an upstream URL is healthy based on active health checks.
///
/// Returns true if health checks are disabled for this upstream or if it's currently healthy.
/// Returns false if health checks are enabled and the upstream is marked unhealthy.
pub fn is_upstream_healthy(state_map: &HealthCheckStateMap, upstream_url: &str) -> bool {
    state_map
        .get(upstream_url)
        .map(|state| state.is_healthy)
        .unwrap_or(true)
}
///
/// This task will periodically probe all upstreams with health checks enabled
/// and update the health state map accordingly.
///
/// The task is spawned on the provided runtime handle (typically the secondary runtime)
/// to avoid requiring a Tokio context in the pipeline stage.
pub fn spawn_health_check_task(
    upstreams: Vec<Upstream>,
    state_map: HealthCheckStateMap,
    on_unhealthy: Option<UnhealthyCallback>,
    runtime_handle: &tokio::runtime::Handle,
    event_sink: Arc<ferron_observability::CompositeEventSink>,
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
                #[cfg(feature = "srv-lookup")]
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
            sleep(Duration::from_secs(u64::MAX)).await;
            return;
        }

        let mut last_probe_times: HashMap<String, tokio::time::Instant> = HashMap::new();

        loop {
            let now = tokio::time::Instant::now();
            let mut next_wake = now + Duration::from_secs(60);

            let mut probes_due = Vec::new();

            for (upstream_url, config, mtls) in &probe_configs {
                let upstreams = match upstream_url {
                    UpstreamHealthCheckType::Static(url) => vec![url.clone()],
                    #[cfg(feature = "srv-lookup")]
                    UpstreamHealthCheckType::Srv((srv_name, dns_servers, weight)) => {
                        let timeout_result = tokio::time::timeout(
                            Duration::from_secs(5),
                            crate::types::srv::resolve_srv_inner(&SrvUpstreamData {
                                srv_name: srv_name.clone(),
                                dns_servers: dns_servers.clone(),
                                weight: *weight,
                                limit: None,
                                // Use default health check config (SrvUpstreamData is only used for resolving SRV records)
                                health_check_config: UpstreamHealthCheckConfig::default(),
                                // mTLS isn't applicable for resolution only
                                mtls: None,
                                priority: None,
                                connection_timeout: None,
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
                            .map(|upstream| upstream.0.proxy_to.clone())
                            .collect()
                    }
                    #[cfg(feature = "srv-lookup")]
                    UpstreamHealthCheckType::StrictDns((host, port, dns_servers)) => {
                        let temp_cfg = crate::types::upstream::UpstreamConfig {
                            url: format!("http://{}:{}", host, port),
                            unix_socket: None,
                            limit: None,
                            health_check_config:
                                crate::types::health::UpstreamHealthCheckConfig::default(),
                            weight: 1,
                            mtls: None,
                            priority: 0,
                            logical_dns: false,
                            dns_servers: dns_servers.clone(),
                            connection_timeout: None,
                        };
                        let timeout_result = tokio::time::timeout(
                            Duration::from_secs(5),
                            crate::types::strict_dns::resolve_strict_dns_inner(&temp_cfg),
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
                        timeout_result
                            .unwrap_or_default()
                            .into_iter()
                            .map(|upstream| upstream.proxy_to.clone())
                            .collect()
                    }
                };

                for upstream_url in upstreams {
                    let last_probe = last_probe_times.get(&upstream_url);
                    let elapsed = last_probe.map_or(Duration::MAX, |t| t.elapsed());

                    if elapsed >= config.interval {
                        probes_due.push((upstream_url.clone(), config.clone(), mtls.clone()));
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

                for (upstream_url, config, mtls) in probes_due {
                    let state_map = Arc::clone(&state_map);
                    let on_unhealthy_clone = on_unhealthy.clone();
                    let probe_url = upstream_url.clone();

                    last_probe_times.insert(upstream_url, now);

                    let event_sink = event_sink.clone();
                    probe_tasks.push(tokio::spawn(async move {
                        let result = probe_upstream(&probe_url, &config, mtls).await;
                        process_probe_result(
                            &probe_url,
                            &config,
                            &result,
                            &state_map,
                            on_unhealthy_clone.as_deref(),
                            &event_sink,
                        );
                    }));
                }

                for task in probe_tasks {
                    let _ = task.await;
                }
            }

            sleep(next_wake - now).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::health::{ExpectedStatusCodes, HealthCheckMethod, HealthCheckState};
    use dashmap::DashMap;
    use rustc_hash::FxBuildHasher;

    #[test]
    fn test_status_code_matching() {
        let config_2xx = UpstreamHealthCheckConfig {
            expect_status: ExpectedStatusCodes::Successful,
            ..Default::default()
        };
        assert!(config_2xx.expect_status.matches(200));
        assert!(config_2xx.expect_status.matches(299));
        assert!(!config_2xx.expect_status.matches(300));
        assert!(!config_2xx.expect_status.matches(199));

        let config_2xx_3xx = UpstreamHealthCheckConfig {
            expect_status: ExpectedStatusCodes::SuccessfulOrRedirect,
            ..Default::default()
        };
        assert!(config_2xx_3xx.expect_status.matches(200));
        assert!(config_2xx_3xx.expect_status.matches(399));
        assert!(!config_2xx_3xx.expect_status.matches(400));
    }

    #[test]
    fn test_health_state_transition_to_unhealthy() {
        let event_sink = ferron_observability::CompositeEventSink::new(vec![]);
        let state_map: HealthCheckStateMap = Arc::new(DashMap::with_hasher(FxBuildHasher));
        let config = UpstreamHealthCheckConfig {
            consecutive_fails: 2,
            ..Default::default()
        };

        let result = ProbeResult {
            status_code: Some(500),
            response_time: Duration::from_millis(100),
            body: None,
            error: None,
        };

        process_probe_result(
            "http://localhost:8080",
            &config,
            &result,
            &state_map,
            None,
            &event_sink,
        );
        process_probe_result(
            "http://localhost:8080",
            &config,
            &result,
            &state_map,
            None,
            &event_sink,
        );

        let state = state_map.get("http://localhost:8080").unwrap();
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

        let fail_result = ProbeResult {
            status_code: Some(500),
            response_time: Duration::from_millis(100),
            body: None,
            error: None,
        };
        process_probe_result(
            "http://localhost:8080",
            &config,
            &fail_result,
            &state_map,
            None,
            &event_sink,
        );
        process_probe_result(
            "http://localhost:8080",
            &config,
            &fail_result,
            &state_map,
            None,
            &event_sink,
        );

        let success_result = ProbeResult {
            status_code: Some(200),
            response_time: Duration::from_millis(100),
            body: None,
            error: None,
        };

        process_probe_result(
            "http://localhost:8080",
            &config,
            &success_result,
            &state_map,
            None,
            &event_sink,
        );
        process_probe_result(
            "http://localhost:8080",
            &config,
            &success_result,
            &state_map,
            None,
            &event_sink,
        );

        let state = state_map.get("http://localhost:8080").unwrap();
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

        let result_fast = ProbeResult {
            status_code: Some(200),
            response_time: Duration::from_millis(30),
            body: None,
            error: None,
        };
        process_probe_result(
            "http://localhost:8080",
            &config,
            &result_fast,
            &state_map,
            None,
            &event_sink,
        );

        {
            let state = state_map.get("http://localhost:8080").unwrap();
            assert!(state.is_healthy);
        }

        let result_slow = ProbeResult {
            status_code: Some(200),
            response_time: Duration::from_millis(100),
            body: None,
            error: None,
        };
        process_probe_result(
            "http://localhost:8080",
            &config,
            &result_slow,
            &state_map,
            None,
            &event_sink,
        );

        let state = state_map.get("http://localhost:8080").unwrap();
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

        let result = ProbeResult {
            status_code: Some(200),
            response_time: Duration::from_millis(50),
            body: Some(b"status: ok".to_vec()),
            error: None,
        };
        process_probe_result(
            "http://localhost:8080",
            &config,
            &result,
            &state_map,
            None,
            &event_sink,
        );

        let state = state_map.get("http://localhost:8080").unwrap();
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

        let result = ProbeResult {
            status_code: Some(200),
            response_time: Duration::from_millis(50),
            body: Some(b"status: fail".to_vec()),
            error: None,
        };
        process_probe_result(
            "http://localhost:8080",
            &config,
            &result,
            &state_map,
            None,
            &event_sink,
        );

        let state = state_map.get("http://localhost:8080").unwrap();
        assert!(!state.is_healthy);
    }

    #[test]
    fn test_is_upstream_healthy() {
        let state_map: HealthCheckStateMap = Arc::new(DashMap::with_hasher(FxBuildHasher));

        assert!(is_upstream_healthy(&state_map, "http://localhost:8080"));

        state_map.insert(
            "http://localhost:8080".to_string(),
            HealthCheckState {
                is_healthy: false,
                ..Default::default()
            },
        );

        assert!(!is_upstream_healthy(&state_map, "http://localhost:8080"));
    }
}
