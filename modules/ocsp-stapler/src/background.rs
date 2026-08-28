//! Background OCSP fetching task owned by the ocsp-stapler module.
//!
//! This file contains the long-running task that periodically fetches and
//! refreshes OCSP responses. Keeping this code in the module crate keeps the
//! types crate lightweight and free of networking/parsing dependencies.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use ferron_observability::{
    CompositeEventSink, LogAttributeValue, LogLevel, MetricAttributeValue, MetricType, MetricValue,
};
use http_body_util::Full;
use hyper::body::Bytes;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use parking_lot::RwLock;
use rustls_pki_types::CertificateDer;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cert;
use crate::fetch::{build_https_connector, fetch_ocsp_response, OcspHttpClient};
use crate::telemetry::{emit_log, emit_metric};

/// Type alias for the OCSP cache to reduce type complexity.
type OcspCache = Arc<RwLock<HashMap<Vec<u8>, Option<Vec<u8>>>>>;

/// Maps certificate leaf bytes to hostname for per-host OCSP metrics.
type OcspHostMap = Arc<RwLock<HashMap<Vec<u8>, String>>>;

/// Logging target used by all events emitted from this module.
const LOG_TARGET: &str = "ferron-ocsp-stapler";

/// Resolve the metric label for a cert key, falling back to a global scope.
fn host_for_key(host_map: &OcspHostMap, key: &[u8]) -> String {
    host_map
        .read()
        .get(key)
        .cloned()
        .unwrap_or_else(|| "_global".to_string())
}

pub async fn background_ocsp_task(
    mut receiver: mpsc::UnboundedReceiver<Vec<CertificateDer<'static>>>,
    cache: OcspCache,
    host_map: OcspHostMap,
    cancel_token: CancellationToken,
    event_sink: Option<Arc<CompositeEventSink>>,
) {
    // Track next-update times per cert
    let mut next_updates: HashMap<Vec<u8>, SystemTime> = HashMap::new();
    // Track known cert chains
    let mut known_certs: HashMap<Vec<u8>, Vec<CertificateDer<'static>>> = HashMap::new();

    let Ok(https_connector) = build_https_connector() else {
        emit_log(
            &event_sink,
            LogLevel::Info,
            "OCSP HTTPS initialization failed",
            "Failed to initialize HTTPS for OCSP background task",
            LOG_TARGET,
            Vec::new(),
        );
        return;
    };

    let client: OcspHttpClient =
        Client::builder(TokioExecutor::new()).build::<_, Full<Bytes>>(https_connector);

    let sleep_duration = Duration::from_secs(60); // default check interval

    loop {
        let received_certified_key = tokio::select! {
            _ = cancel_token.cancelled() => {
                return;
            }
            _ = tokio::time::sleep(sleep_duration) => None,
            res = receiver.recv() => match res {
                Some(chain) => Some(chain),
                None => return, // channel closed
            },
        };

        if let Some(chain) = received_certified_key {
            if let Some(leaf) = chain.first() {
                let key: Vec<u8> = leaf.to_vec();
                if !known_certs.contains_key(&key) {
                    let ident = cert::cert_identifier(&chain);
                    emit_log(
                        &event_sink,
                        LogLevel::Debug,
                        "OCSP fetch triggered",
                        &format!("OCSP fetch triggered for certificate {ident}"),
                        LOG_TARGET,
                        vec![(
                            "ferron.ocsp.cert.subject",
                            LogAttributeValue::String(ident.clone()),
                        )],
                    );
                    known_certs.insert(key.clone(), chain.clone());
                    // Trigger immediate fetch (use time in the past to ensure it is fetched immediately)
                    next_updates.insert(key, SystemTime::now() - std::time::Duration::from_secs(1));
                }
            }
        }

        let now = SystemTime::now();
        let updates_to_fetch: Vec<Vec<u8>> = next_updates
            .iter()
            .filter(|(_, next_update)| **next_update <= now)
            .map(|(key, _)| key.clone())
            .collect();

        for key in updates_to_fetch {
            if let Some(cert) = known_certs.get(&key) {
                let start = std::time::Instant::now();
                match fetch_ocsp_response(&client, cert).await {
                    Ok(Some((response_der, next_update_time))) => {
                        let duration = start.elapsed().as_secs_f64();
                        let ident = cert::cert_identifier(cert);
                        let next_update_ts = next_update_time
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs() as i64;
                        let primary_san = cert::primary_san(cert);
                        let primary_san_formatted = primary_san
                            .as_ref()
                            .map(|san| format!(" ({san})"))
                            .unwrap_or_default();

                        let mut log_attributes = vec![
                            (
                                "ferron.ocsp.cert.subject",
                                LogAttributeValue::String(ident.clone()),
                            ),
                            (
                                "ferron.ocsp.next_update",
                                LogAttributeValue::I64(next_update_ts),
                            ),
                        ];
                        if let Some(san) = &primary_san {
                            log_attributes.push((
                                "ferron.ocsp.cert.primary_san",
                                LogAttributeValue::String(san.clone()),
                            ));
                        }
                        emit_log(
                            &event_sink,
                            LogLevel::Info,
                            "OCSP response cached",
                            &format!(
                                "OCSP response cached for {ident}{primary_san_formatted}, valid until {}",
                                chrono::DateTime::<chrono::Utc>::from(next_update_time)
                                    .format("%Y-%m-%d %H:%M:%S")
                            ),
                            LOG_TARGET,
                            log_attributes,
                        );
                        emit_metric(
                            &event_sink,
                            "ferron.ocsp.fetches_total",
                            MetricValue::U64(1),
                            MetricType::Counter,
                            Some("{fetch}"),
                            Some("Total OCSP fetch attempts"),
                            vec![
                                (
                                    "ferron.ocsp.status",
                                    MetricAttributeValue::StaticStr("success"),
                                ),
                                (
                                    "ferron.host",
                                    MetricAttributeValue::String(host_for_key(&host_map, &key)),
                                ),
                            ],
                        );
                        emit_metric(
                            &event_sink,
                            "ferron.ocsp.fetch_duration_seconds",
                            MetricValue::F64(duration),
                            MetricType::Histogram(None),
                            Some("s"),
                            Some("Time to fetch OCSP response"),
                            vec![(
                                "ferron.host",
                                MetricAttributeValue::String(host_for_key(&host_map, &key)),
                            )],
                        );

                        cache.write().insert(key.clone(), Some(response_der));
                        next_updates.insert(key, next_update_time);
                    }
                    Ok(None) => {
                        let ident = cert::cert_identifier(cert);
                        emit_log(
                            &event_sink,
                            LogLevel::Debug,
                            "OCSP stapling skipped",
                            &format!(
                                "OCSP stapling skipped — \
                                 no OCSP URL or incomplete chain in certificate {ident}"
                            ),
                            LOG_TARGET,
                            vec![
                                ("ferron.ocsp.cert.subject", LogAttributeValue::String(ident)),
                                (
                                    "ferron.ocsp.reason",
                                    LogAttributeValue::StaticStr("no_ocsp_url_or_incomplete_chain"),
                                ),
                            ],
                        );
                        emit_metric(
                            &event_sink,
                            "ferron.ocsp.fetches_total",
                            MetricValue::U64(1),
                            MetricType::Counter,
                            Some("{fetch}"),
                            Some("Total OCSP fetch attempts"),
                            vec![
                                (
                                    "ferron.ocsp.status",
                                    MetricAttributeValue::StaticStr("skipped"),
                                ),
                                (
                                    "ferron.host",
                                    MetricAttributeValue::String(host_for_key(&host_map, &key)),
                                ),
                            ],
                        );
                        // No OCSP possible (e.g. no OCSP URL in cert)
                        cache.write().insert(key.clone(), None);
                        next_updates.remove(&key);
                    }
                    Err(e) => {
                        let duration = start.elapsed().as_secs_f64();
                        let ident = cert::cert_identifier(cert);
                        emit_log(
                            &event_sink,
                            LogLevel::Warn,
                            "OCSP fetch failed",
                            &format!("OCSP fetch failed for {ident}: {e}"),
                            LOG_TARGET,
                            vec![
                                ("ferron.ocsp.cert.subject", LogAttributeValue::String(ident)),
                                ("error.message", LogAttributeValue::String(e.to_string())),
                            ],
                        );
                        emit_metric(
                            &event_sink,
                            "ferron.ocsp.fetches_total",
                            MetricValue::U64(1),
                            MetricType::Counter,
                            Some("{fetch}"),
                            Some("Total OCSP fetch attempts"),
                            vec![
                                (
                                    "ferron.ocsp.status",
                                    MetricAttributeValue::StaticStr("error"),
                                ),
                                (
                                    "ferron.host",
                                    MetricAttributeValue::String(host_for_key(&host_map, &key)),
                                ),
                            ],
                        );
                        emit_metric(
                            &event_sink,
                            "ferron.ocsp.fetch_duration_seconds",
                            MetricValue::F64(duration),
                            MetricType::Histogram(None),
                            Some("s"),
                            Some("Time to fetch OCSP response"),
                            vec![(
                                "ferron.host",
                                MetricAttributeValue::String(host_for_key(&host_map, &key)),
                            )],
                        );
                        // Retry later with randomness to avoid refresh storms
                        let jitter = rand::random_range(100..=500);
                        next_updates.insert(key, now + Duration::from_secs(jitter));
                    }
                }
            }
        }

        let stapled_count = cache.read().iter().filter(|(_, v)| v.is_some()).count();
        emit_metric(
            &event_sink,
            "ferron.ocsp.cached_certificates",
            MetricValue::U64(known_certs.len() as u64),
            MetricType::Gauge,
            Some("{certificate}"),
            Some("Number of certificates in OCSP cache"),
            vec![],
        );
        emit_metric(
            &event_sink,
            "ferron.ocsp.certificates_with_stapling",
            MetricValue::U64(stapled_count as u64),
            MetricType::Gauge,
            Some("{certificate}"),
            Some("Number of certificates with valid OCSP stapling"),
            vec![],
        );
    }
}
