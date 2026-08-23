use std::sync::Arc;

use ferron_http::span::HttpContextSpanExt;
use ferron_http::HttpContext;
use ferron_observability::{
    CompositeEventSink, Event, MetricAttributeValue, MetricEvent, MetricType, MetricValue,
    TraceAttributeValue,
};
use types::circuit::circuit_breaker_state_label;

use crate::types;
use crate::types::ConnectionsTrackState;

pub(crate) static PROXY_POOL_BUCKETS: &[f64] = &[0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0];
pub(crate) static PROXY_TLS_BUCKETS: &[f64] = &[0.001, 0.01, 0.05, 0.1, 0.5, 1.0, 2.0, 5.0];

pub struct ProxyMetrics {
    pub selected_backends: rustc_hash::FxHashSet<Arc<types::upstream::UpstreamInner>>,
    pub final_selected_backend: Option<Arc<types::upstream::UpstreamInner>>,
    pub circuit_breaker_unhealthy_backends: Vec<Arc<types::upstream::UpstreamInner>>,
    pub active_unhealthy_backends: Vec<(String, u64)>,
    pub connection_reused: bool,
    pub tls_handshake_failures: u64,
    pub tls_handshake_time_secs: f64,
    pub pool_waits: u64,
    pub pool_wait_time_secs: f64,
    pub upstream_time_secs: f64,
    pub status_code: Option<u16>,
    pub excluded_circuit_open: Vec<Arc<types::upstream::UpstreamInner>>,
    pub excluded_already_tried: Vec<Arc<types::upstream::UpstreamInner>>,
    pub excluded_overloaded: Vec<Arc<types::upstream::UpstreamInner>>,
    pub retry_count: u64,
    pub same_upstream_retry_count: u64,
    pub retry_budget_exhausted: bool,
    pub retry_budget_tokens: Option<f64>,
    pub request_method: Option<&'static str>,
    pub method_idempotent: Option<bool>,
    pub pool_hit: bool,
    pub pool_miss: bool,
    pub connect_time_secs: f64,
    pub ttfb_secs: f64,
    pub candidate_scores: Vec<f64>,
    pub upstream_response_truncated: bool,
    pub upstream_bytes_received: Option<u64>,
    pub upstream_content_length: Option<u64>,
}

impl Default for ProxyMetrics {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl ProxyMetrics {
    #[inline]
    pub fn new() -> Self {
        Self {
            selected_backends: rustc_hash::FxHashSet::default(),
            final_selected_backend: None,
            circuit_breaker_unhealthy_backends: Vec::new(),
            active_unhealthy_backends: Vec::new(),
            connection_reused: false,
            tls_handshake_failures: 0,
            tls_handshake_time_secs: 0.0,
            pool_waits: 0,
            pool_wait_time_secs: 0.0,
            upstream_time_secs: 0.0,
            status_code: None,
            excluded_circuit_open: Vec::new(),
            excluded_already_tried: Vec::new(),
            excluded_overloaded: Vec::new(),
            retry_count: 0,
            same_upstream_retry_count: 0,
            retry_budget_exhausted: false,
            retry_budget_tokens: None,
            request_method: None,
            method_idempotent: None,
            pool_hit: false,
            pool_miss: false,
            connect_time_secs: 0.0,
            ttfb_secs: 0.0,
            candidate_scores: Vec::new(),
            upstream_response_truncated: false,
            upstream_bytes_received: None,
            upstream_content_length: None,
        }
    }
}

#[inline]
pub(crate) fn emit_proxy_failure_metric(
    ctx: &HttpContext,
    status_code: u16,
    error_type: &str,
    trace_context: Option<ferron_observability::EventTraceContext>,
) {
    use ferron_observability::{Event, MetricAttributeValue, MetricEvent, MetricType, MetricValue};

    ctx.events.emit(Event::Metric(MetricEvent {
        name: "ferron.proxy.failures",
        attributes: vec![
            (
                "http.response.status_code",
                MetricAttributeValue::I64(status_code as i64),
            ),
            (
                "error.type",
                MetricAttributeValue::String(error_type.to_string()),
            ),
        ],
        ty: MetricType::Counter,
        value: MetricValue::U64(1),
        unit: Some("{request}"),
        description: Some(
            "Number of reverse proxy requests that failed before a backend response was returned.",
        ),
        trace_context,
    }));
}

/// Inject upstream runtime state as span attributes on the current request span.
///
/// Called after backend selection (both success and failure paths) to give
/// client-side OTLP traces visibility into *why* the proxy chose a particular
/// backend and what state it was in at the time of the request.
#[inline]
pub(crate) fn inject_upstream_state_span_attributes(
    ctx: &mut HttpContext,
    backend: &Arc<types::upstream::UpstreamInner>,
    circuit_breaker_state: &types::circuit::CircuitBreakerStateMap,
    flapping_state: &types::flapping::FlappingStateMap,
    health_check_state: &types::health::HealthCheckStateMap,
    conn_state: &ConnectionsTrackState,
    slow_start_duration: std::time::Duration,
) {
    let sa = ctx.get_span_attributes();

    // Circuit breaker state
    if let Some(cb) = circuit_breaker_state.get(backend) {
        let status = cb.status.load(std::sync::atomic::Ordering::Relaxed);
        sa.insert(
            "ferron.proxy.upstream.circuit_state",
            TraceAttributeValue::StaticStr(circuit_breaker_state_label(status)),
        );
    }

    // Flapping state
    if let Some(flapping) = flapping_state.get(&backend.proxy_to) {
        sa.insert(
            "ferron.proxy.upstream.is_flapping",
            TraceAttributeValue::Bool(flapping.is_flapping()),
        );
    }

    // Health check state (only when a health check entry exists for this URL)
    if let Some(hc) = health_check_state.get(&backend.proxy_to) {
        sa.insert(
            "ferron.proxy.upstream.health_status",
            TraceAttributeValue::StaticStr(if hc.is_healthy {
                "healthy"
            } else {
                "unhealthy"
            }),
        );
        sa.insert(
            "ferron.proxy.upstream.consecutive_failures",
            TraceAttributeValue::I64(hc.consecutive_fail_count as i64),
        );
    }

    // Active connection count (approximate, same technique as P2C/least-conn selectors)
    if let Some(entry) = conn_state.get(backend) {
        let active = (Arc::strong_count(&*entry) as i64).saturating_sub(1);
        sa.insert(
            "ferron.proxy.upstream.active_connections",
            TraceAttributeValue::I64(active),
        );
    }

    // Slow-start state (circuit breaker recently closed)
    if !slow_start_duration.is_zero() {
        if let Some(cb) = circuit_breaker_state.get(backend) {
            let in_slow_start = cb
                .slow_start_recovery_at
                .as_ref()
                .and_then(|r| *r.read())
                .is_some_and(|t| t.elapsed() < slow_start_duration);
            sa.insert(
                "ferron.proxy.upstream.slow_start",
                TraceAttributeValue::Bool(in_slow_start),
            );
        }
    }
}

/// Build resolved IP and DNS status attributes for proxy metrics.
#[inline]
pub(crate) fn resolved_ip_attrs(
    metrics_resolved_ip: bool,
    backend: &Arc<types::upstream::UpstreamInner>,
) -> Vec<(&'static str, ferron_observability::MetricAttributeValue)> {
    use ferron_observability::MetricAttributeValue;
    let mut attrs = Vec::with_capacity(2);
    if metrics_resolved_ip {
        if let Some(ref ip) = backend.connect_to {
            attrs.push((
                "ferron.proxy.backend_resolved_ip",
                MetricAttributeValue::String(ip.to_string()),
            ));
        }
    }
    attrs.push((
        "ferron.proxy.dns_status",
        MetricAttributeValue::String(backend.dns_status.as_label().to_string()),
    ));
    attrs
}

#[inline]
pub(crate) fn emit_backend_excluded(
    events: &ferron_observability::CompositeEventSink,
    backend: &Arc<types::upstream::UpstreamInner>,
    reason: &'static str,
    trace_context: Option<ferron_observability::EventTraceContext>,
    metrics_resolved_ip: bool,
) {
    use ferron_observability::{MetricAttributeValue, MetricEvent, MetricType, MetricValue};
    let mut attrs = Vec::with_capacity(5);
    attrs.push((
        "ferron.proxy.backend_url",
        MetricAttributeValue::String(backend.proxy_to.clone()),
    ));
    if let Some(ref unix_path) = backend.proxy_unix {
        attrs.push((
            "ferron.proxy.backend_unix_path",
            MetricAttributeValue::String(unix_path.clone()),
        ));
    }
    attrs.extend(resolved_ip_attrs(metrics_resolved_ip, backend));
    attrs.push((
        "ferron.proxy.reason",
        MetricAttributeValue::StaticStr(reason),
    ));
    events.emit(ferron_observability::Event::Metric(MetricEvent {
        name: "ferron.proxy.backends.excluded",
        attributes: attrs,
        ty: MetricType::Counter,
        value: MetricValue::U64(1),
        unit: Some("{backend}"),
        description: Some(
            "Backend excluded from selection due to health, circuit breaker, or retry state.",
        ),
        trace_context,
    }));
}

/// Background task that periodically emits reverse proxy pool depth and DNS
/// cache metrics on the secondary runtime.
///
/// Spawned once from `ReverseProxyModule::start()`.
pub(crate) async fn emit_pool_and_dns_metrics(pool_sink: Arc<CompositeEventSink>) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
    loop {
        interval.tick().await;
        emit_pool_depth_gauges(&pool_sink);
        emit_pool_limit_gauges(&pool_sink);
        emit_dns_cache_metrics(&pool_sink);
    }
}

/// Background task that periodically purges expired DNS cache entries.
///
/// Spawned once from `ReverseProxyModule::start()`.
pub(crate) async fn cleanup_dns_cache_task() {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
    loop {
        interval.tick().await;
        crate::types::dns_cache::cleanup_expired();
    }
}

/// Attributes describing a backend: the upstream URL and, when present, the
/// unix socket path.
fn backend_attrs(
    upstream: &Arc<crate::types::upstream::UpstreamInner>,
) -> Vec<(&'static str, MetricAttributeValue)> {
    let mut attrs = Vec::with_capacity(2);
    attrs.push((
        "ferron.proxy.backend_url",
        MetricAttributeValue::String(upstream.proxy_to.clone()),
    ));
    if let Some(ref unix_path) = upstream.proxy_unix {
        attrs.push((
            "ferron.proxy.backend_unix_path",
            MetricAttributeValue::String(unix_path.clone()),
        ));
    }
    attrs
}

/// Emit idle/outstanding connection gauges for each backend and worker thread.
fn emit_pool_depth_gauges(pool_sink: &CompositeEventSink) {
    let snapshot = crate::connections::POOL_STATS.snapshot();
    for ((thread_id, upstream), (idle, outstanding)) in snapshot {
        let mut attrs = backend_attrs(&upstream);
        attrs.push((
            "worker",
            MetricAttributeValue::String(format!("{:?}", thread_id)),
        ));
        pool_sink.emit(Event::Metric(MetricEvent {
            name: "ferron.proxy.pool.idle",
            attributes: attrs.clone(),
            ty: MetricType::Gauge,
            value: MetricValue::U64(idle as u64),
            unit: Some("{connection}"),
            description: Some("Current number of idle connections in the pool."),
            trace_context: None,
        }));
        pool_sink.emit(Event::Metric(MetricEvent {
            name: "ferron.proxy.pool.outstanding",
            attributes: attrs,
            ty: MetricType::Gauge,
            value: MetricValue::U64(outstanding as u64),
            unit: Some("{connection}"),
            description: Some("Current number of outstanding (in-use) connections in the pool."),
            trace_context: None,
        }));
    }
}

/// Emit the per-upstream local connection limit and the global connection limit.
fn emit_pool_limit_gauges(pool_sink: &CompositeEventSink) {
    let local_limit_snapshot = crate::connections::POOL_STATS.snapshot_local_limits();
    for (upstream, limit) in local_limit_snapshot {
        pool_sink.emit(Event::Metric(MetricEvent {
            name: "ferron.proxy.pool.local_limit",
            attributes: vec![(
                "ferron.proxy.backend_url",
                MetricAttributeValue::String(upstream.proxy_to.clone()),
            )],
            ty: MetricType::Gauge,
            value: MetricValue::U64(limit as u64),
            unit: Some("{connection}"),
            description: Some("Current per-upstream local connection limit for this worker."),
            trace_context: None,
        }));
    }

    let global_limit =
        crate::GLOBAL_CONCURRENT_CONNECTIONS.load(std::sync::atomic::Ordering::Relaxed);
    pool_sink.emit(Event::Metric(MetricEvent {
        name: "ferron.proxy.pool.global_limit",
        attributes: Vec::new(),
        ty: MetricType::Gauge,
        value: MetricValue::U64(global_limit as u64),
        unit: Some("{connection}"),
        description: Some("Current global connection limit for reverse proxy."),
        trace_context: None,
    }));
}

/// Emit DNS result cache hit/miss counters and TTL gauges.
fn emit_dns_cache_metrics(pool_sink: &CompositeEventSink) {
    let hits =
        crate::types::dns_cache::DNS_CACHE_HITS.swap(0, std::sync::atomic::Ordering::Relaxed);
    let misses =
        crate::types::dns_cache::DNS_CACHE_MISSES.swap(0, std::sync::atomic::Ordering::Relaxed);
    if hits > 0 {
        pool_sink.emit(Event::Metric(MetricEvent {
            name: "ferron.proxy.dns.cache_hit",
            attributes: Vec::new(),
            ty: MetricType::Counter,
            value: MetricValue::U64(hits),
            unit: Some("{request}"),
            description: Some("DNS result cache hits."),
            trace_context: None,
        }));
    }
    if misses > 0 {
        pool_sink.emit(Event::Metric(MetricEvent {
            name: "ferron.proxy.dns.cache_miss",
            attributes: Vec::new(),
            ty: MetricType::Counter,
            value: MetricValue::U64(misses),
            unit: Some("{request}"),
            description: Some("DNS result cache misses."),
            trace_context: None,
        }));
    }

    if let Some(ttl_stats) = crate::types::dns_cache::strict_dns_ttl_stats() {
        emit_dns_ttl_gauges(pool_sink, ttl_stats);
    }
}

/// Emit remaining-TTL and entry-count gauges for the DNS result cache.
fn emit_dns_ttl_gauges(
    pool_sink: &CompositeEventSink,
    ttl_stats: crate::types::dns_cache::DnsCacheTtlStats,
) {
    let (min, max, avg) = (
        ttl_stats.min_remaining_secs,
        ttl_stats.max_remaining_secs,
        ttl_stats.avg_remaining_secs,
    );
    for (aggregation, value, description) in [
        (
            "min",
            min,
            "Minimum remaining TTL across all DNS cache entries.",
        ),
        (
            "max",
            max,
            "Maximum remaining TTL across all DNS cache entries.",
        ),
        (
            "avg",
            avg,
            "Average remaining TTL across all DNS cache entries.",
        ),
    ] {
        pool_sink.emit(Event::Metric(MetricEvent {
            name: "ferron.proxy.dns.cache_ttl_remaining_seconds",
            attributes: vec![(
                "aggregation",
                MetricAttributeValue::String(aggregation.into()),
            )],
            ty: MetricType::Gauge,
            value: MetricValue::F64(value),
            unit: Some("{second}"),
            description: Some(description),
            trace_context: None,
        }));
    }
    pool_sink.emit(Event::Metric(MetricEvent {
        name: "ferron.proxy.dns.cache_entries",
        attributes: Vec::new(),
        ty: MetricType::Gauge,
        value: MetricValue::U64(ttl_stats.entry_count as u64),
        unit: Some("{entry}"),
        description: Some("Number of active entries in the DNS cache."),
        trace_context: None,
    }));
}
