use std::sync::OnceLock;
use std::time::Duration;

use ferron_http::trace_context;
use ferron_http::HttpContext;
use ferron_observability::{Event, LogAttributeValue, LogEvent, LogLevel};
use http::header::HeaderName;

use crate::config::{CacheZoneId, PurgePropagationConfig};
use crate::lscache::{PurgeOperation, PurgeSelector};
use crate::policy::CacheScope;
use crate::store::CacheStore;
use crate::SECONDARY_RUNTIME;
use ferron_observability::{MetricAttributeValue, MetricEvent, MetricType, MetricValue};

pub(super) const PURGE_SOURCE_HEADER: HeaderName = HeaderName::from_static("x-purge-source");
pub(super) const PURGE_SECRET_HEADER: HeaderName = HeaderName::from_static("x-purge-secret");

const LOG_TARGET: &str = "ferron-http-cache";

type PropagationClient = hyper_util::client::legacy::Client<
    hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
    http_body_util::Full<bytes::Bytes>,
>;

/// Outbound HTTPS client for purge propagation webhooks, built once per
/// process.
static PROPAGATION_CLIENT: OnceLock<
    Result<PropagationClient, Box<dyn std::error::Error + Send + Sync>>,
> = OnceLock::new();

/// Result of a purge run against the store.
pub(super) struct PurgeStats {
    pub(super) purged: usize,
}

/// Emit the purge metric for one cache scope, together with the entries
/// gauge reflecting the store size after the purge.
#[inline]
fn emit_purge_metric(
    ctx: &HttpContext,
    zone_id: &CacheZoneId,
    scope: CacheScope,
    purged: usize,
    items: usize,
) {
    if purged == 0 {
        return;
    }
    ctx.events.emit(Event::Metric(MetricEvent {
        name: "ferron.cache.purges",
        attributes: vec![
            (
                "ferron.cache.zone",
                MetricAttributeValue::String(zone_id.label().to_string()),
            ),
            (
                "ferron.cache.scope",
                MetricAttributeValue::StaticStr(scope.as_str()),
            ),
        ],
        ty: MetricType::Counter,
        value: MetricValue::U64(purged as u64),
        unit: Some("{entry}"),
        description: Some("Number of cache entries purged via LSCache-compatible controls."),
        trace_context: trace_context::current_event_trace_context(ctx),
    }));
    ctx.events.emit(Event::Metric(MetricEvent {
        name: "ferron.cache.entries",
        attributes: vec![(
            "ferron.cache.zone",
            MetricAttributeValue::String(zone_id.label().to_string()),
        )],
        ty: MetricType::Gauge,
        value: MetricValue::U64(items as u64),
        unit: Some("{entry}"),
        description: Some("Number of entries currently stored in the HTTP cache."),
        trace_context: trace_context::current_event_trace_context(ctx),
    }));
}

/// Purge the store for the given operations and report the outcome.
///
/// This is the single seam through which every purge flows. It owns the
/// store purge, the per-scope purge metrics, the purge debug log, and the
/// optional control-plane webhook fan-out (deriving the propagated paths
/// from the operations' selectors and deduplicating them).
#[allow(clippy::too_many_arguments)]
pub(super) fn purge(
    ctx: &mut HttpContext,
    zone_id: &CacheZoneId,
    store: &CacheStore,
    operations: &[PurgeOperation],
    private_key: Option<&str>,
    requesting_host: Option<&str>,
    propagate: bool,
    propagation: &PurgePropagationConfig,
) -> PurgeStats {
    let mut total_purged = 0;
    for scope in [CacheScope::Public, CacheScope::Private] {
        let scope_operations: Vec<PurgeOperation> = operations
            .iter()
            .filter(|operation| operation.scope == scope)
            .cloned()
            .collect();
        if scope_operations.is_empty() {
            continue;
        }
        let (stats, remaining) = store.purge(&scope_operations, private_key, requesting_host);
        emit_purge_metric(ctx, zone_id, scope, stats.purged, remaining);
        total_purged += stats.purged;
    }

    if total_purged > 0 {
        let stats = PurgeStats {
            purged: total_purged,
        };
        ctx.events.emit(Event::Log(LogEvent {
            level: LogLevel::Debug,
            target: LOG_TARGET,
            message: format!("Purged {} cache entries", stats.purged),
            summary: "Cache purged".into(),
            attributes: vec![(
                "cache.purged.count",
                LogAttributeValue::I64(stats.purged as i64),
            )],
            trace_context: trace_context::current_event_trace_context(ctx),
        }));

        if propagate {
            if let Some(url) = &propagation.control_plane_url {
                let paths = collect_propagation_paths(operations);
                if !paths.is_empty() {
                    spawn_propagation_webhooks(
                        url.clone(),
                        propagation.shared_secret.clone(),
                        propagation.node_id.clone(),
                        paths,
                        ctx,
                    );
                }
            }
        }
    }

    PurgeStats {
        purged: total_purged,
    }
}

/// Map purge selectors to the paths sent to the control-plane, deduplicated.
pub(super) fn collect_propagation_paths(operations: &[PurgeOperation]) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    for operation in operations {
        for selector in &operation.selectors {
            let path = match selector {
                PurgeSelector::All => "*".to_string(),
                PurgeSelector::Url(url) => url.clone(),
                PurgeSelector::UrlPath(path) => path.clone(),
                PurgeSelector::Tag(tag) => format!("tag={tag}"),
            };
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    paths
}

fn spawn_propagation_webhooks(
    url: String,
    shared_secret: Option<String>,
    node_id: Option<String>,
    paths: Vec<String>,
    ctx: &mut HttpContext,
) {
    let Some(handle) = SECONDARY_RUNTIME.get() else {
        ctx.events.emit(Event::Log(LogEvent {
            level: LogLevel::Warn,
            target: LOG_TARGET,
            message: "Distributed cache purge not yet available".to_string(),
            summary: "Distributed cache purge not yet available".into(),
            attributes: Vec::new(),
            trace_context: trace_context::current_event_trace_context(ctx),
        }));
        return;
    };

    let client = PROPAGATION_CLIENT.get_or_init(build_propagation_client);
    let client = match client {
        Ok(client) => client,
        Err(error) => {
            ctx.events.emit(Event::Log(LogEvent {
                level: LogLevel::Warn,
                target: LOG_TARGET,
                message: format!("Purge propagation to control-plane failed: {}", error),
                summary: "Purge propagation failed".into(),
                attributes: vec![(
                    "error.message",
                    LogAttributeValue::String(error.to_string()),
                )],
                trace_context: trace_context::current_event_trace_context(ctx),
            }));
            return;
        }
    };

    let events = ctx.events.clone();
    let trace_context = trace_context::current_event_trace_context(ctx);
    handle.spawn(async move {
        for path in &paths {
            if let Err(error) = send_propagation_webhook(
                client,
                &url,
                shared_secret.as_deref(),
                node_id.as_deref(),
                path,
            )
            .await
            {
                events.emit(Event::Log(LogEvent {
                    level: LogLevel::Warn,
                    target: LOG_TARGET,
                    message: format!("Purge propagation to control-plane failed: {}", error),
                    summary: "Purge propagation failed".into(),
                    attributes: vec![(
                        "error.message",
                        LogAttributeValue::String(error.to_string()),
                    )],
                    trace_context: trace_context.clone(),
                }));
            }
        }
    });
}

/// Build an HTTPS client for outbound purge propagation webhooks.
fn build_propagation_client() -> Result<PropagationClient, Box<dyn std::error::Error + Send + Sync>>
{
    use hyper_rustls::HttpsConnectorBuilder;

    let root_store = build_root_cert_store()?;

    let tls_config = rustls::ClientConfig::builder_with_provider(
        rustls::crypto::aws_lc_rs::default_provider().into(),
    )
    .with_safe_default_protocol_versions()?
    .with_root_certificates(root_store)
    .with_no_client_auth();

    let https = HttpsConnectorBuilder::new()
        .with_tls_config(tls_config)
        .https_or_http()
        .enable_http1()
        .enable_http2()
        .build();

    Ok(
        hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
            .build(https),
    )
}

fn build_root_cert_store() -> Result<rustls::RootCertStore, Box<dyn std::error::Error + Send + Sync>>
{
    let mut root_store = rustls::RootCertStore::empty();
    let mut found_any = false;

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

    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    if !found_any {
        ferron_core::log_warn!("using webpki-roots as fallback (no native root CAs available)");
    }

    if root_store.is_empty() {
        return Err("No root certificates available".into());
    }

    Ok(root_store)
}

/// Send a purge webhook to the external control-plane service.
///
/// The webhook is a `POST` with a JSON body containing the purged path and the
/// originating node ID. The control-plane is expected to fan out `PURGE`
/// requests to all other registered edges.
async fn send_propagation_webhook(
    client: &PropagationClient,
    url: &str,
    shared_secret: Option<&str>,
    node_id: Option<&str>,
    path: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let body = serde_json::json!({
        "path": path,
        "origin": node_id.unwrap_or("unknown"),
    });

    let mut request = http::Request::builder()
        .method(http::Method::POST)
        .uri(url)
        .header(http::header::CONTENT_TYPE, "application/json");

    if let Some(secret) = shared_secret {
        request = request.header(&PURGE_SECRET_HEADER, secret);
    }

    let request = request.body(http_body_util::Full::new(bytes::Bytes::from(
        serde_json::to_vec(&body)?,
    )))?;

    let response = tokio::time::timeout(Duration::from_secs(5), client.request(request)).await??;

    if !response.status().is_success() {
        return Err(format!("control-plane returned HTTP {}", response.status()).into());
    }

    Ok(())
}
