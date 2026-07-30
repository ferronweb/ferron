use std::sync::Arc;

use ferron_http::span::HttpContextSpanExt;
use ferron_http::HttpContext;
use ferron_observability::TraceAttributeValue;
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
        control_plane_metadata: None,
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

    // Active connection count (approximate — same technique as P2C/least-conn selectors)
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
        control_plane_metadata: None,
    }));
}
