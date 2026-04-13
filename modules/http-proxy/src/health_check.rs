//! Active health check task for probing upstream backends.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use tokio::time::sleep;

use crate::upstream::{HealthCheckStateMap, Upstream, UpstreamHealthCheckConfig};

/// Callback invoked when a backend is marked unhealthy by active health check.
/// Arguments: (backend_url, is_active_health_check=true)
pub type UnhealthyCallback = Arc<dyn Fn(&str, bool) + Send + Sync>;

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
async fn probe_upstream(upstream_url: &str, config: &UpstreamHealthCheckConfig) -> ProbeResult {
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
) -> Result<(u16, Option<Vec<u8>>), String> {
    use bytes::Bytes;
    use http_body_util::Full;
    use hyper::Request;
    use hyper_util::client::legacy::Client;
    use hyper_util::rt::TokioExecutor;

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

    let client_is_https = url_parsed.scheme_str() == Some("https");

    // Build a client for the request. Building the TLS ClientConfig is cached in
    // build_https_connector to avoid repeated native cert loading.
    let resp = if client_is_https {
        let connector = build_https_connector(no_verification)
            .map_err(|e| format!("Failed to build TLS connector: {}", e))?;
        let client: Client<_, Full<Bytes>> = Client::builder(TokioExecutor::new()).build(connector);
        let req = Request::builder()
            .method(method.to_uppercase().as_str())
            .uri(url_parsed)
            .header("User-Agent", "Ferron")
            .header("Connection", "close")
            .body(Full::new(Bytes::new()))
            .map_err(|e| format!("Failed to build request: {}", e))?;
        tokio::time::timeout(timeout, client.request(req)).await
    } else {
        let http_connector = hyper_util::client::legacy::connect::HttpConnector::new();
        let client: Client<_, Full<Bytes>> =
            Client::builder(TokioExecutor::new()).build(http_connector);
        let req = Request::builder()
            .method(method.to_uppercase().as_str())
            .uri(url_parsed)
            .header("User-Agent", "Ferron")
            .header("Connection", "close")
            .body(Full::new(Bytes::new()))
            .map_err(|e| format!("Failed to build request: {}", e))?;
        tokio::time::timeout(timeout, client.request(req)).await
    };

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

/// Build an HTTPS connector with native certificate store and webpki-roots fallback.
/// When `no_verification` is true, disables TLS certificate verification.
fn build_https_connector(
    no_verification: bool,
) -> Result<
    hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
    Box<dyn std::error::Error + Send + Sync>,
> {
    use hyper_rustls::HttpsConnectorBuilder;
    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use std::sync::LazyLock;

    // Cache built HttpsConnector instances to avoid repeated TLS config and
    // connector builder work. Two variants: normal verification and no_verification.
    static DEFAULT_HTTPS_CONNECTOR: LazyLock<
        hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
    > = LazyLock::new(|| {
        // Build default root store once
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
                        ferron_core::log_debug!(
                            "Health check: certificate parsing failed: {:?}",
                            err
                        );
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

        let tls_config = if root_store.is_empty() {
            rustls::ClientConfig::builder()
                .with_root_certificates(rustls::RootCertStore::empty())
                .with_no_client_auth()
        } else {
            rustls::ClientConfig::builder()
                .with_root_certificates(root_store)
                .with_no_client_auth()
        };

        HttpsConnectorBuilder::new()
            .with_tls_config(tls_config)
            .https_or_http()
            .enable_http1()
            .enable_http2()
            .build()
    });

    static NO_VERIFY_HTTPS_CONNECTOR: LazyLock<
        hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
    > = LazyLock::new(|| {
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

        let tls_config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(std::sync::Arc::new(NoServerVerifier))
            .with_no_client_auth();

        HttpsConnectorBuilder::new()
            .with_tls_config(tls_config)
            .https_or_http()
            .enable_http1()
            .enable_http2()
            .build()
    });

    let connector = if no_verification {
        NO_VERIFY_HTTPS_CONNECTOR.clone()
    } else {
        DEFAULT_HTTPS_CONNECTOR.clone()
    };

    Ok(connector)
}

/// Process a probe result and update health check state.
#[allow(clippy::type_complexity)]
fn process_probe_result(
    upstream_url: &str,
    config: &UpstreamHealthCheckConfig,
    result: &ProbeResult,
    state_map: &HealthCheckStateMap,
    on_unhealthy: Option<&(dyn Fn(&str, bool) + Send + Sync)>,
) {
    let mut states = state_map.write();
    let state = states.entry(upstream_url.to_string()).or_default();

    let probe_success = if let Some(status) = result.status_code {
        let status_ok = config.expect_status.matches(status);

        let time_ok = config
            .response_time_threshold
            .map(|threshold| result.response_time <= threshold)
            .unwrap_or(true);

        let body_ok = if config.method == crate::upstream::HealthCheckMethod::Get {
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
                ferron_core::log_info!(
                    "Upstream {} recovered after {} consecutive successes",
                    upstream_url,
                    config.consecutive_passes
                );
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
                        crate::upstream::ExpectedStatusCodes::Successful => "2xx",
                        crate::upstream::ExpectedStatusCodes::SuccessfulOrRedirect => "2xx/3xx",
                        _ => "custom",
                    }
                )
            });
            ferron_core::log_warn!(
                "Upstream {} marked unhealthy: {} ({}/{})",
                upstream_url,
                error_msg,
                state.consecutive_fail_count,
                config.consecutive_fails
            );
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
    let states = state_map.read();
    states
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
) -> tokio::task::JoinHandle<()> {
    runtime_handle.spawn(async move {
        let mut probe_configs: Vec<(String, UpstreamHealthCheckConfig)> = Vec::new();

        for upstream in &upstreams {
            match upstream {
                Upstream::Static(cfg) => {
                    if cfg.health_check_config.enabled {
                        probe_configs.push((cfg.url.clone(), cfg.health_check_config.clone()));
                    }
                }
                #[cfg(feature = "srv-lookup")]
                Upstream::Srv(_) => {
                    // SRV upstreams: health checks not yet supported
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

            for (upstream_url, config) in &probe_configs {
                let last_probe = last_probe_times.get(upstream_url);
                let elapsed = last_probe.map_or(Duration::MAX, |t| t.elapsed());

                if elapsed >= config.interval {
                    probes_due.push((upstream_url.clone(), config.clone()));
                    next_wake = now;
                } else {
                    let time_until_due = config.interval - elapsed;
                    if time_until_due < next_wake - now {
                        next_wake = now + time_until_due;
                    }
                }
            }

            if !probes_due.is_empty() {
                let mut probe_tasks = Vec::new();

                for (upstream_url, config) in probes_due {
                    let state_map = Arc::clone(&state_map);
                    let on_unhealthy_clone = on_unhealthy.clone();
                    let probe_url = upstream_url.clone();

                    last_probe_times.insert(upstream_url, now);

                    probe_tasks.push(tokio::spawn(async move {
                        let result = probe_upstream(&probe_url, &config).await;
                        process_probe_result(
                            &probe_url,
                            &config,
                            &result,
                            &state_map,
                            on_unhealthy_clone.as_deref(),
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
    use crate::upstream::{ExpectedStatusCodes, HealthCheckMethod, HealthCheckState};
    use parking_lot::RwLock;

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
        let state_map: HealthCheckStateMap = Arc::new(RwLock::new(HashMap::new()));
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

        process_probe_result("http://localhost:8080", &config, &result, &state_map, None);
        process_probe_result("http://localhost:8080", &config, &result, &state_map, None);

        let states = state_map.read();
        let state = &states["http://localhost:8080"];
        assert!(!state.is_healthy);
        assert_eq!(state.consecutive_fail_count, 2);
    }

    #[test]
    fn test_health_state_recovery() {
        let state_map: HealthCheckStateMap = Arc::new(RwLock::new(HashMap::new()));
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
        );
        process_probe_result(
            "http://localhost:8080",
            &config,
            &fail_result,
            &state_map,
            None,
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
        );
        process_probe_result(
            "http://localhost:8080",
            &config,
            &success_result,
            &state_map,
            None,
        );

        let states = state_map.read();
        let state = &states["http://localhost:8080"];
        assert!(state.is_healthy);
        assert_eq!(state.consecutive_pass_count, 0);
        assert_eq!(state.consecutive_fail_count, 0);
    }

    #[test]
    fn test_response_time_threshold() {
        let state_map: HealthCheckStateMap = Arc::new(RwLock::new(HashMap::new()));
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
        );

        {
            let states = state_map.read();
            let state = &states["http://localhost:8080"];
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
        );

        let states = state_map.read();
        let state = &states["http://localhost:8080"];
        assert!(!state.is_healthy);
        assert_eq!(state.consecutive_fail_count, 1);
    }

    #[test]
    fn test_body_match_success() {
        let state_map: HealthCheckStateMap = Arc::new(RwLock::new(HashMap::new()));
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
        process_probe_result("http://localhost:8080", &config, &result, &state_map, None);

        let states = state_map.read();
        let state = &states["http://localhost:8080"];
        assert!(state.is_healthy);
    }

    #[test]
    fn test_body_match_failure() {
        let state_map: HealthCheckStateMap = Arc::new(RwLock::new(HashMap::new()));
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
        process_probe_result("http://localhost:8080", &config, &result, &state_map, None);

        let states = state_map.read();
        let state = &states["http://localhost:8080"];
        assert!(!state.is_healthy);
    }

    #[test]
    fn test_is_upstream_healthy() {
        let state_map: HealthCheckStateMap = Arc::new(RwLock::new(HashMap::new()));

        assert!(is_upstream_healthy(&state_map, "http://localhost:8080"));

        {
            let mut states = state_map.write();
            states.insert(
                "http://localhost:8080".to_string(),
                HealthCheckState {
                    is_healthy: false,
                    ..Default::default()
                },
            );
        }

        assert!(!is_upstream_healthy(&state_map, "http://localhost:8080"));
    }
}
