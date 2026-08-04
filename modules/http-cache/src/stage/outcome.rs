use ferron_http::access_log::{custom_access_log_fields, CustomAccessLogField};
use ferron_http::span::HttpContextSpanExt;
use ferron_http::trace_context;
use ferron_http::HttpContext;
use ferron_observability::{
    Event, LogAttributeValue, LogEvent, LogLevel, MetricAttributeValue, MetricEvent, MetricType,
    MetricValue, TraceAttributeValue,
};

use crate::config::CacheZoneId;
use crate::policy::CacheScope;
use crate::store::{CacheStore, StoreStats};

use super::key::cache_key_fingerprint;

const LOG_TARGET: &str = "ferron-http-cache";

/// Everything one cache outcome needs to report: request/store/eviction
/// metrics, span attributes, and access-log fields.
pub(super) struct CacheOutcome<'a> {
    /// Result label for the span attribute and access-log field
    /// (`hit`, `stale`, `miss`, `bypass`, `revalidate`, `purge`, ...).
    pub(super) result: &'static str,
    pub(super) zone_id: &'a CacheZoneId,
    /// Cache key the access-log fingerprint is derived from.
    pub(super) key: &'a str,
    pub(super) scope: Option<CacheScope>,
    /// Store size for the entries gauge; `Some` also emits the request counter.
    pub(super) items: Option<usize>,
    /// A response was stored in the cache: (scope, status code).
    pub(super) stored: Option<(CacheScope, u16)>,
    pub(super) evictions: Option<StoreStats>,
    /// Extra span detail attribute (`response-too-large`, bypass reason, ...).
    pub(super) detail: Option<&'a str>,
    pub(super) key_uri: Option<&'a str>,
    pub(super) key_method: Option<&'a str>,
    pub(super) bypass_reason: Option<&'a str>,
    /// Vary cookies that were part of the entry key.
    pub(super) evaluated_cookies: Option<&'a [String]>,
    /// Singleflight wait in ms; also marks the request as coalesced.
    pub(super) coalesced_wait_ms: Option<f64>,
    /// Emits the `coalesced=false` marker for upstream-bound requests.
    pub(super) mark_uncoalesced: bool,
    /// Metric label when it differs from the span/log label.
    pub(super) metric_result: Option<&'static str>,
}

/// Report a cache outcome: metrics first, then span attributes, then
/// access-log fields.
#[inline]
pub(super) fn report(ctx: &mut HttpContext, outcome: CacheOutcome<'_>) {
    let CacheOutcome {
        result,
        zone_id,
        key,
        scope,
        items,
        stored,
        evictions,
        detail,
        key_uri,
        key_method,
        bypass_reason,
        evaluated_cookies,
        coalesced_wait_ms,
        mark_uncoalesced,
        metric_result,
    } = outcome;

    let metric_result = metric_result.unwrap_or(result);

    if let Some(stats) = evictions {
        emit_eviction_metrics(ctx, zone_id, stats);
    }
    if let Some((scope, status)) = stored {
        emit_store_metric(ctx, zone_id, scope, status);
    }
    if let Some(items) = items {
        emit_request_metric(ctx, zone_id, metric_result, scope, items);
    }

    let sa = ctx.get_span_attributes();
    sa.insert(
        "ferron.cache.result",
        TraceAttributeValue::String(result.to_string()),
    );
    sa.insert(
        "ferron.cache.zone",
        TraceAttributeValue::String(zone_id.label().to_string()),
    );
    if let Some(scope) = scope {
        sa.insert(
            "ferron.cache.scope",
            TraceAttributeValue::String(scope.as_str().to_string()),
        );
    }
    if let Some(detail) = detail {
        sa.insert(
            "ferron.cache.detail",
            TraceAttributeValue::String(detail.to_string()),
        );
    }
    if let Some(uri) = key_uri {
        sa.insert(
            "ferron.cache.key.uri",
            TraceAttributeValue::String(uri.to_string()),
        );
    }
    if let Some(method) = key_method {
        sa.insert(
            "ferron.cache.key.method",
            TraceAttributeValue::String(method.to_string()),
        );
    }
    if let Some(reason) = bypass_reason {
        sa.insert(
            "ferron.cache.bypass_reason",
            TraceAttributeValue::String(reason.to_string()),
        );
    }
    if let Some(cookies) = evaluated_cookies {
        sa.insert(
            "ferron.cache.key.evaluated_cookies",
            TraceAttributeValue::String(cookies.join(";")),
        );
    }

    let log_fields = custom_access_log_fields(ctx);
    log_fields.insert(
        "ferron.cache.result".into(),
        CustomAccessLogField::String(result.into()),
    );
    log_fields.insert(
        "ferron.cache.zone".into(),
        CustomAccessLogField::String(zone_id.label().to_string()),
    );
    log_fields.insert(
        "ferron.cache.key_fingerprint".into(),
        CustomAccessLogField::String(cache_key_fingerprint(key)),
    );
    if let Some(wait_ms) = coalesced_wait_ms {
        log_fields.insert(
            "ferron.cache.coalesced".into(),
            CustomAccessLogField::Bool(true),
        );
        log_fields.insert(
            "ferron.cache.coalesce_wait_duration_ms".into(),
            CustomAccessLogField::F64(wait_ms),
        );
    } else if mark_uncoalesced {
        log_fields.insert(
            "ferron.cache.coalesced".into(),
            CustomAccessLogField::Bool(false),
        );
        log_fields.insert(
            "ferron.cache.coalesce_wait_duration_ms".into(),
            CustomAccessLogField::F64(0.0),
        );
    }
}

#[inline]
pub(super) fn emit_request_metric(
    ctx: &HttpContext,
    zone_id: &CacheZoneId,
    result: &'static str,
    scope: Option<CacheScope>,
    items: usize,
) {
    let mut attrs = vec![
        (
            "ferron.cache.zone",
            MetricAttributeValue::String(zone_id.label().to_string()),
        ),
        (
            "ferron.cache.result",
            MetricAttributeValue::StaticStr(result),
        ),
    ];
    if let Some(scope) = scope {
        attrs.push((
            "ferron.cache.scope",
            MetricAttributeValue::StaticStr(scope.as_str()),
        ));
    }
    ctx.events.emit(Event::Metric(MetricEvent {
        name: "ferron.cache.requests",
        attributes: attrs,
        ty: MetricType::Counter,
        value: MetricValue::U64(1),
        unit: Some("{request}"),
        description: Some("Number of cache lookups handled by the HTTP cache."),
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

#[inline]
pub(super) fn emit_store_metric(
    ctx: &HttpContext,
    zone_id: &CacheZoneId,
    scope: CacheScope,
    status_code: u16,
) {
    ctx.events.emit(Event::Metric(MetricEvent {
        name: "ferron.cache.stores",
        attributes: vec![
            (
                "ferron.cache.zone",
                MetricAttributeValue::String(zone_id.label().to_string()),
            ),
            (
                "ferron.cache.scope",
                MetricAttributeValue::StaticStr(scope.as_str()),
            ),
            (
                "http.response.status_code",
                MetricAttributeValue::I64(status_code as i64),
            ),
        ],
        ty: MetricType::Counter,
        value: MetricValue::U64(1),
        unit: Some("{response}"),
        description: Some("Number of responses stored in the HTTP cache."),
        trace_context: trace_context::current_event_trace_context(ctx),
    }));
}

/// Wait-site helper: reports singleflight deduplication activity. Called
/// directly at the coalescing wait site, not through `report`.
#[inline]
pub(super) fn emit_singleflight_metrics(ctx: &HttpContext, store: &CacheStore) {
    ctx.events.emit(Event::Metric(MetricEvent {
        name: "ferron.cache.coalesced_requests",
        attributes: Vec::new(),
        ty: MetricType::Counter,
        value: MetricValue::U64(1),
        unit: Some("{request}"),
        description: Some(
            "Number of requests intercepted by the singleflight deduplication layer.",
        ),
        trace_context: trace_context::current_event_trace_context(ctx),
    }));
    ctx.events.emit(Event::Metric(MetricEvent {
        name: "ferron.cache.singleflight_active_locks",
        attributes: Vec::new(),
        ty: MetricType::Gauge,
        value: MetricValue::U64(store.active_locks() as u64),
        unit: Some("{lock}"),
        description: Some(
            "Number of active in-flight upstream fetches coordinated by singleflight.",
        ),
        trace_context: trace_context::current_event_trace_context(ctx),
    }));
}

#[inline]
pub(super) fn emit_eviction_metrics(ctx: &HttpContext, zone_id: &CacheZoneId, stats: StoreStats) {
    if stats.expired_evictions > 0 {
        ctx.events.emit(Event::Metric(MetricEvent {
            name: "ferron.cache.evictions",
            attributes: vec![
                (
                    "ferron.cache.zone",
                    MetricAttributeValue::String(zone_id.label().to_string()),
                ),
                (
                    "ferron.cache.reason",
                    MetricAttributeValue::StaticStr("expired"),
                ),
            ],
            ty: MetricType::Counter,
            value: MetricValue::U64(stats.expired_evictions as u64),
            unit: Some("{entry}"),
            description: Some("Number of cache entries evicted from the HTTP cache."),
            trace_context: trace_context::current_event_trace_context(ctx),
        }));
        ctx.events.emit(Event::Log(LogEvent {
            level: LogLevel::Debug,
            target: LOG_TARGET,
            message: format!(
                "Evicted {} expired cache entries from zone {}",
                stats.expired_evictions,
                zone_id.label()
            ),
            summary: "Cache entries evicted".into(),
            attributes: vec![
                (
                    "eviction.reason",
                    LogAttributeValue::String("ttl_expired".into()),
                ),
                (
                    "eviction.count",
                    LogAttributeValue::I64(stats.expired_evictions as i64),
                ),
                (
                    "ferron.cache.zone",
                    LogAttributeValue::String(zone_id.label().to_string()),
                ),
            ],
            trace_context: trace_context::current_event_trace_context(ctx),
        }));
    }
    if stats.size_evictions > 0 {
        ctx.events.emit(Event::Metric(MetricEvent {
            name: "ferron.cache.evictions",
            attributes: vec![
                (
                    "ferron.cache.zone",
                    MetricAttributeValue::String(zone_id.label().to_string()),
                ),
                (
                    "ferron.cache.reason",
                    MetricAttributeValue::StaticStr("size"),
                ),
            ],
            ty: MetricType::Counter,
            value: MetricValue::U64(stats.size_evictions as u64),
            unit: Some("{entry}"),
            description: Some("Number of cache entries evicted from the HTTP cache."),
            trace_context: trace_context::current_event_trace_context(ctx),
        }));
        ctx.events.emit(Event::Log(LogEvent {
            level: LogLevel::Debug,
            target: LOG_TARGET,
            message: format!(
                "Evicted {} cache entries (capacity) from zone {}",
                stats.size_evictions,
                zone_id.label()
            ),
            summary: "Cache entries evicted".into(),
            attributes: vec![
                (
                    "eviction.reason",
                    LogAttributeValue::String("capacity_reached_lru".into()),
                ),
                (
                    "eviction.count",
                    LogAttributeValue::I64(stats.size_evictions as i64),
                ),
                (
                    "ferron.cache.zone",
                    LogAttributeValue::String(zone_id.label().to_string()),
                ),
            ],
            trace_context: trace_context::current_event_trace_context(ctx),
        }));
    }
}
