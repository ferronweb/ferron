use ferron_http::HttpContext;
use ferron_observability::{
    Event, LogAttributeValue, LogEvent, LogLevel, MetricAttributeValue, MetricEvent, MetricType,
    MetricValue,
};

use crate::config::CacheZoneId;
use crate::policy::CacheScope;
use crate::store::{CacheStore, StoreStats};

const LOG_TARGET: &str = "ferron-http-cache";

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
        trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
        
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
        trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
        
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
        trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
        
    }));
}

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
        trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
        
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
        trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
        
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
            trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
            
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
            trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
            
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
            trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
            
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
            trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
            
        }));
    }
}

#[inline]
pub(super) fn emit_purge_metric(
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
        trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
        
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
        trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
        
    }));
}
