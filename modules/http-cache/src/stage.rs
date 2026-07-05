use std::convert::TryFrom;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use ahash::{AHashMap, AHashSet};
use async_trait::async_trait;
use bytes::{Bytes, BytesMut};
use dashmap::DashMap;
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::StageConstraint;
use ferron_http::access_log::{custom_access_log_fields, CustomAccessLogField};
use ferron_http::span::HttpContextSpanExt;
use ferron_http::{HttpContext, HttpResponse};
use ferron_observability::{
    Event, LogAttributeValue, LogEvent, LogLevel, MetricAttributeValue, MetricEvent, MetricType,
    MetricValue, TraceAttributeValue,
};
use futures_util::stream::{self, StreamExt};
use http::header::{self, HeaderName, HeaderValue};
use http::{HeaderMap, Method, Response, StatusCode};
use http_body::{Body as _, Frame};
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, BodyStream, Empty, Full, StreamBody};
#[cfg(test)]
use rustc_hash::FxHashMap;
use typemap_rev::TypeMapKey;

use crate::config::{
    has_host_max_entries, parse_cache_config, parse_max_entries, CacheConfig, CacheZoneId,
};
use crate::lscache::{
    collect_lsc_cookies, parse_litespeed_cache_control, parse_litespeed_purge,
    parse_litespeed_tags, parse_litespeed_vary, PurgeOperation, PurgeSelector, LS_CACHE,
    LS_CACHE_CONTROL, LS_COOKIE, LS_PURGE, LS_TAG, LS_VARY,
};
use crate::policy::{
    evaluate_response_policy, parse_request_policy, CacheScope, RequestCachePolicy,
};
use crate::store::{
    strip_store_headers, CacheStore, LookupEntry, StoreStats, StoredEntry, VaryRule,
};
use crate::SECONDARY_RUNTIME;

const LOG_TARGET: &str = "ferron-http-cache";
const CACHE_STATUS_HEADER: HeaderName = HeaderName::from_static("cache-status");
const PRIVATE_COOKIE_NAMES: &[&str] = &["frontend", "phpsessid", "xf_session", "lsc_private"];
const PURGE_SOURCE_HEADER: HeaderName = HeaderName::from_static("x-purge-source");
/// Header sent in outbound purge webhooks to identify the originating edge node.
/// The external control-plane uses this to avoid broadcasting back to the origin.
#[allow(dead_code)]
const PURGE_ORIGIN_HEADER: HeaderName = HeaderName::from_static("x-purge-origin");
const PURGE_SECRET_HEADER: HeaderName = HeaderName::from_static("x-purge-secret");

struct RequestStateKey;

impl TypeMapKey for RequestStateKey {
    type Value = RequestState;
}

struct RequestState {
    config: CacheConfig,
    zone_id: CacheZoneId,
    base_key: String,
    request_headers: HeaderMap,
    request_cookies: AHashMap<String, String>,
    private_key: Option<String>,
    purge_url: String,
    request_policy: RequestCachePolicy,
    has_authorization: bool,
    head_only: bool,
    lookup_result: LookupResult,
    store: Arc<CacheStore>,
    /// When present, notifies coalesced waiters when the leader completes.
    _inflight_guard: Option<InflightGuard>,
}

enum LookupResult {
    Hit,
    #[expect(dead_code)]
    StaleWhileRevalidate {
        entry: Box<LookupEntry>,
        cache_key: String,
        stats: StoreStats,
        /// Key for inflight coalescing on expired-entry misses.
        inflight_key: Option<String>,
        scope: Option<CacheScope>,
        items: usize,
    },
    Revalidate {
        entry: Box<LookupEntry>,
        cache_key: String,
        stats: StoreStats,
    },
    Miss {
        stats: StoreStats,
        /// Key for inflight coalescing on expired-entry misses.
        inflight_key: Option<String>,
    },
    Bypass,
}

/// RAII guard that calls `complete_fetch` when dropped, notifying coalesced waiters.
struct InflightGuard {
    store: Arc<CacheStore>,
    cache_key: String,
}

impl Drop for InflightGuard {
    #[inline]
    fn drop(&mut self) {
        self.store.complete_fetch(&self.cache_key);
    }
}

enum CollectBodyOutcome {
    Complete(Option<Bytes>),
    Overflow {
        prefix: Bytes,
        remainder: UnsyncBoxBody<Bytes, io::Error>,
    },
}

/// Tracks the config generation at which a zone's `max_entries` was last applied.
struct ZoneGeneration {
    generation: AtomicU64,
}

/// Pipeline stage for HTTP response caching.
pub struct HttpCacheStage {
    /// Cache stores keyed by `CacheZoneId`.
    zones: Arc<DashMap<CacheZoneId, Arc<CacheStore>>>,
    /// Config generation at which each zone's `max_entries` was last applied.
    zone_generations: Arc<DashMap<CacheZoneId, ZoneGeneration>>,
}

impl HttpCacheStage {
    #[inline]
    pub fn new() -> Self {
        Self {
            zones: Arc::new(DashMap::new()),
            zone_generations: Arc::new(DashMap::new()),
        }
    }

    /// Get or create a `CacheStore` for the given zone, updating `max_entries`
    /// only when the configuration generation changes (not on every request).
    fn get_or_create_zone(
        &self,
        zone_id: &CacheZoneId,
        configuration: &ferron_core::config::layer::LayeredConfiguration,
    ) -> Arc<CacheStore> {
        let store = self
            .zones
            .entry(zone_id.clone())
            .or_insert_with(|| Arc::new(CacheStore::new(crate::config::DEFAULT_MAX_CACHE_ENTRIES)))
            .value()
            .clone();

        // Only update max_entries when the config generation changes.
        // This prevents per-request LRU eviction when different host blocks
        // specify different capacities for the same zone.
        let current_gen = ferron_core::admin::ADMIN_METRICS
            .reload_metrics
            .read()
            .active_generation;

        let should_update = match self.zone_generations.get(zone_id) {
            Some(entry) => entry.generation.load(Ordering::Relaxed) != current_gen,
            None => true,
        };

        if should_update {
            let new_max = match zone_id {
                // Named zones: read max_entries from the global zone definition.
                CacheZoneId::Named(name) => {
                    crate::config::parse_global_zone_max_entries(configuration, name)
                        .unwrap_or(crate::config::DEFAULT_MAX_CACHE_ENTRIES)
                }
                // Global zone: read from the global cache block's max_entries.
                CacheZoneId::Global => parse_max_entries(configuration),
                // Per-host zones: read from the host's layered config.
                CacheZoneId::Host(_) => parse_max_entries(configuration),
            };
            store.set_max_entries(new_max);
            self.zone_generations
                .entry(zone_id.clone())
                .or_insert_with(|| ZoneGeneration {
                    generation: AtomicU64::new(current_gen),
                })
                .generation
                .store(current_gen, Ordering::Relaxed);
        }

        store
    }

    #[inline]
    fn emit_request_metric(
        &self,
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
    fn emit_store_metric(
        &self,
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
    fn emit_eviction_metrics(&self, ctx: &HttpContext, zone_id: &CacheZoneId, stats: StoreStats) {
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
        }
    }

    #[inline]
    fn emit_purge_metric(
        &self,
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
}

impl Default for HttpCacheStage {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait(?Send)]
impl Stage<HttpContext> for HttpCacheStage {
    #[inline]
    fn name(&self) -> &str {
        "cache"
    }

    #[inline]
    fn constraints(&self) -> Vec<StageConstraint> {
        vec![
            StageConstraint::After("https_redirect".to_string()),
            StageConstraint::After("rewrite".to_string()),
            StageConstraint::After("rate_limit".to_string()),
            StageConstraint::After("http_response".to_string()),
            StageConstraint::After("abuse_protection".to_string()),
            StageConstraint::After("basicauth".to_string()),
            StageConstraint::Before("forward_proxy".to_string()),
            StageConstraint::Before("reverse_proxy".to_string()),
            StageConstraint::Before("static_file".to_string()),
        ]
    }

    #[inline]
    fn is_applicable(
        &self,
        config: Option<&ferron_core::config::ServerConfigurationBlock>,
    ) -> bool {
        config.is_some_and(|config| config.has_directive("cache"))
    }

    #[inline]
    async fn run(&self, ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        let config = parse_cache_config(&ctx.configuration);

        if !config.enabled {
            return Ok(true);
        }

        let zone_id = resolve_zone_id(&ctx.hostname, &config, &ctx.configuration);
        let store = self.get_or_create_zone(&zone_id, &ctx.configuration);

        let Some(request) = ctx.req.as_ref() else {
            return Ok(true);
        };

        let request_headers = request.headers().clone();
        let request_cookies = parse_cookies(&request_headers);
        let request_policy = if config.ignore_request_cache_control {
            RequestCachePolicy {
                allow_lookup: true,
                allow_store: true,
                reason: "eligible",
            }
        } else {
            parse_request_policy(&request_headers)
        };
        let has_authorization = request_headers.contains_key(header::AUTHORIZATION);
        let purge_url = request
            .uri()
            .path_and_query()
            .map(|value| value.as_str().to_string())
            .unwrap_or_else(|| request.uri().path().to_string());
        let base_key = build_base_key(
            ctx.encrypted,
            &request_headers,
            ctx.original_uri.as_ref(),
            request.uri(),
        );
        let private_key = Some(build_private_cache_key(
            &request_cookies,
            ctx.remote_address.ip(),
            ctx.auth_user.as_deref(),
        ));
        let head_only = request.method() == Method::HEAD;

        let method_cacheable = matches!(request.method(), &Method::GET | &Method::HEAD);
        let method_purge = request.method() == "PURGE";

        // Handle PURGE method — cache invalidation
        if method_purge {
            if !config.purge_method {
                // PURGE not enabled — fall through to 405 from downstream stages
            } else {
                // Security check: must be authenticated or from an allowed IP
                let ip_allowed = if !config.purge_allowed_ips.is_empty() {
                    config
                        .purge_allowed_ips
                        .iter()
                        .any(|cidr| cidr.contains(&ctx.remote_address.ip().to_canonical()))
                } else {
                    false
                };
                let purge_allowed = ctx.auth_user.is_some() || ip_allowed;

                if !purge_allowed {
                    ctx.res = Some(HttpResponse::BuiltinError(403, None));
                    ctx.get_span_attributes().insert(
                        "ferron.cache.result",
                        TraceAttributeValue::String("purge_rejected".to_string()),
                    );
                    let log_fields = custom_access_log_fields(ctx);
                    log_fields.insert(
                        "ferron.cache.result".into(),
                        CustomAccessLogField::String("purge_rejected".into()),
                    );
                    return Ok(false);
                }

                // Purge both public and private scopes matching the URL
                let mut purged = 0;
                for scope in [CacheScope::Public, CacheScope::Private] {
                    let purge_ops = vec![PurgeOperation {
                        scope,
                        selectors: vec![PurgeSelector::UrlPath(request.uri().path().to_string())],
                        stale: false,
                    }];
                    let (stats, items) = store.purge(&purge_ops, None);
                    if stats.purged > 0 {
                        self.emit_purge_metric(ctx, &zone_id, scope, stats.purged, items);
                    }
                    purged += stats.purged;
                }

                ctx.events.emit(Event::Log(LogEvent {
                    level: LogLevel::Debug,
                    target: LOG_TARGET,
                    message: format!("Purged {} cache entries via PURGE method", purged),
                    summary: "Cache purged via PURGE method".into(),
                    attributes: vec![("cache.purged.count", LogAttributeValue::I64(purged as i64))],
                    trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
                }));

                // Propagate purge to external control-plane unless this is a
                // propagated request (X-Purge-Source: propagation).
                let is_propagated = request_headers
                    .get(&PURGE_SOURCE_HEADER)
                    .and_then(|v| v.to_str().ok())
                    .is_some_and(|v| v.eq_ignore_ascii_case("propagation"));

                if !is_propagated {
                    if let Some(url) = &config.purge_propagation.control_plane_url {
                        let url = url.clone();
                        let secret = config.purge_propagation.shared_secret.clone();
                        let node_id = config.purge_propagation.node_id.clone();
                        let path = request
                            .uri()
                            .path_and_query()
                            .map(|pq| pq.as_str().to_string())
                            .unwrap_or_else(|| request.uri().path().to_string());
                        if let Some(handle) = SECONDARY_RUNTIME.get() {
                            handle.spawn(async move {
                                if let Err(e) = propagate_purge_webhook(
                                    &url,
                                    secret.as_deref(),
                                    node_id.as_deref(),
                                    &path,
                                )
                                .await
                                {
                                    ferron_core::log_warn!(
                                        "Purge propagation to control-plane failed: {}",
                                        e
                                    );
                                }
                            });
                        } else {
                            ctx.events.emit(Event::Log(LogEvent {
                                level: LogLevel::Warn,
                                target: LOG_TARGET,
                                message: "Distributed cache purge not yet available".to_string(),
                                summary: "Distributed cache purge not yet available".into(),
                                attributes: Vec::new(),
                                trace_context:
                                    ferron_http::trace_context::current_event_trace_context(ctx),
                            }));
                        }
                    }
                }

                let response = Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, "text/plain")
                    .body(
                        Full::new(Bytes::from_static(b"Purged"))
                            .map_err(|_: std::convert::Infallible| unreachable!())
                            .boxed_unsync(),
                    )
                    .map_err(|e| PipelineError::custom(e.to_string()))?;
                ctx.res = Some(HttpResponse::Custom(response));
                ctx.get_span_attributes().insert(
                    "ferron.cache.result",
                    TraceAttributeValue::String("purge".to_string()),
                );
                ctx.get_span_attributes().insert(
                    "ferron.cache.zone",
                    TraceAttributeValue::String(zone_id.label().to_string()),
                );
                {
                    let log_fields = custom_access_log_fields(ctx);
                    log_fields.insert(
                        "ferron.cache.result".into(),
                        CustomAccessLogField::String("purge".into()),
                    );
                    log_fields.insert(
                        "ferron.cache.zone".into(),
                        CustomAccessLogField::String(zone_id.label().to_string()),
                    );
                }
                // Don't insert RequestState — run_inverse will skip
                return Ok(false);
            }
        }

        let request_is_lookup_eligible = method_cacheable
            && !request_headers.contains_key(header::RANGE)
            && !request_headers.contains_key(header::UPGRADE)
            && request_policy.allow_lookup;

        let lookup_result = if request_is_lookup_eligible {
            let (lookup, stats, items, had_expired) = store.lookup(
                &base_key,
                &request_headers,
                &request_cookies,
                private_key.as_deref(),
            );
            if let Some((entry, cache_key, hit_kind)) = lookup {
                let scope = entry.scope;
                self.emit_eviction_metrics(ctx, &zone_id, stats);

                if request_policy.reason == "request-revalidation" {
                    self.emit_request_metric(ctx, &zone_id, "hit", Some(scope), items);
                    LookupResult::Revalidate {
                        entry: Box::new(entry),
                        cache_key,
                        stats,
                    }
                } else if let crate::store::LookupHit::StaleWhileRevalidate = hit_kind {
                    if config.enable_stale_while_revalidate {
                        let (is_leader, _notify) = store.begin_fetch(&base_key);

                        LookupResult::StaleWhileRevalidate {
                            entry: Box::new(entry),
                            cache_key,
                            stats,
                            inflight_key: is_leader.then_some(base_key.clone()),
                            scope: Some(scope),
                            items,
                        }
                    } else {
                        // SWR disabled — treat stale entry as revalidation
                        self.emit_request_metric(ctx, &zone_id, "hit", Some(scope), items);
                        LookupResult::Revalidate {
                            entry: Box::new(entry),
                            cache_key,
                            stats,
                        }
                    }
                } else {
                    self.emit_request_metric(ctx, &zone_id, "hit", Some(scope), items);
                    {
                        let sa = ctx.get_span_attributes();
                        sa.insert(
                            "ferron.cache.result",
                            TraceAttributeValue::String("hit".to_string()),
                        );
                        sa.insert(
                            "ferron.cache.zone",
                            TraceAttributeValue::String(zone_id.label().to_string()),
                        );
                        sa.insert(
                            "ferron.cache.scope",
                            TraceAttributeValue::String(scope.as_str().to_string()),
                        );
                        let log_fields = custom_access_log_fields(ctx);
                        log_fields.insert(
                            "ferron.cache.result".into(),
                            CustomAccessLogField::String("hit".into()),
                        );
                        log_fields.insert(
                            "ferron.cache.zone".into(),
                            CustomAccessLogField::String(zone_id.label().to_string()),
                        );
                    }
                    ctx.res = Some(if entry.body.is_none() {
                        HttpResponse::BuiltinError(
                            entry.status.as_u16(),
                            Some(entry.headers.clone()),
                        )
                    } else {
                        HttpResponse::Custom(build_cached_response(
                            entry,
                            head_only,
                            config.emit_litespeed_headers,
                        )?)
                    });
                    LookupResult::Hit
                }
            } else {
                self.emit_eviction_metrics(ctx, &zone_id, stats);

                if had_expired {
                    // Thundering herd protection: coalesce concurrent requests
                    let (is_leader, notify) = store.begin_fetch(&base_key);

                    if !is_leader {
                        // Another request is already fetching this key.
                        // Wait for it to complete, then re-check the cache.
                        notify.notified().await;

                        let (retry_lookup, retry_stats, retry_items, _) = store.lookup(
                            &base_key,
                            &request_headers,
                            &request_cookies,
                            private_key.as_deref(),
                        );
                        if let Some((entry, _, _)) = retry_lookup {
                            // Leader populated the cache — serve from cache
                            let scope = entry.scope;
                            self.emit_eviction_metrics(ctx, &zone_id, retry_stats);
                            self.emit_request_metric(
                                ctx,
                                &zone_id,
                                "hit",
                                Some(scope),
                                retry_items,
                            );
                            {
                                let sa = ctx.get_span_attributes();
                                sa.insert(
                                    "ferron.cache.result",
                                    TraceAttributeValue::String("hit".to_string()),
                                );
                                sa.insert(
                                    "ferron.cache.zone",
                                    TraceAttributeValue::String(zone_id.label().to_string()),
                                );
                                sa.insert(
                                    "ferron.cache.scope",
                                    TraceAttributeValue::String(scope.as_str().to_string()),
                                );
                                let log_fields = custom_access_log_fields(ctx);
                                log_fields.insert(
                                    "ferron.cache.result".into(),
                                    CustomAccessLogField::String("hit".into()),
                                );
                                log_fields.insert(
                                    "ferron.cache.zone".into(),
                                    CustomAccessLogField::String(zone_id.label().to_string()),
                                );
                            }
                            ctx.res = Some(if entry.body.is_none() {
                                HttpResponse::BuiltinError(
                                    entry.status.as_u16(),
                                    Some(entry.headers.clone()),
                                )
                            } else {
                                HttpResponse::Custom(build_cached_response(
                                    entry,
                                    head_only,
                                    config.emit_litespeed_headers,
                                )?)
                            });
                            return Ok(false);
                        }
                        // Leader's response was non-cacheable — proceed normally
                        LookupResult::Miss {
                            stats: retry_stats,
                            inflight_key: None,
                        }
                    } else {
                        // We are the leader — proceed to downstream, guard will notify on drop
                        LookupResult::Miss {
                            stats,
                            inflight_key: Some(base_key.clone()),
                        }
                    }
                } else {
                    // First-time request or non-expiry miss — no coalescing
                    LookupResult::Miss {
                        stats,
                        inflight_key: None,
                    }
                }
            }
        } else {
            LookupResult::Bypass
        };

        let stop = match &lookup_result {
            LookupResult::Hit => true,
            LookupResult::StaleWhileRevalidate { inflight_key, .. } => inflight_key.is_none(),
            _ => false,
        };

        let inflight_guard = match lookup_result {
            LookupResult::Miss {
                ref inflight_key, ..
            }
            | LookupResult::StaleWhileRevalidate {
                ref inflight_key, ..
            } => inflight_key.as_ref().map(|key| InflightGuard {
                store: store.clone(),
                cache_key: key.clone(),
            }),
            _ => None,
        };

        // Inject conditional headers for revalidation
        if let LookupResult::Revalidate { ref entry, .. } = lookup_result {
            if let Some(ref mut request) = ctx.req {
                if let Some(etag) = &entry.etag {
                    request
                        .headers_mut()
                        .insert(header::IF_NONE_MATCH, etag.clone());
                }
                if let Some(last_modified) = &entry.last_modified {
                    request
                        .headers_mut()
                        .insert(header::IF_MODIFIED_SINCE, last_modified.clone());
                }
            }
        }

        if !stop {
            let result_label = match &lookup_result {
                LookupResult::Hit => "hit",
                LookupResult::StaleWhileRevalidate { inflight_key, .. }
                    if inflight_key.is_some() =>
                {
                    "stale"
                }
                LookupResult::StaleWhileRevalidate { .. } => "hit",
                LookupResult::Revalidate { .. } => "revalidate",
                LookupResult::Miss { .. } => "miss",
                LookupResult::Bypass => "bypass",
            };
            let sa = ctx.get_span_attributes();
            sa.insert(
                "ferron.cache.result",
                TraceAttributeValue::String(result_label.to_string()),
            );
            sa.insert(
                "ferron.cache.zone",
                TraceAttributeValue::String(zone_id.label().to_string()),
            );
            if let LookupResult::Bypass = &lookup_result {
                sa.insert(
                    "ferron.cache.bypass_reason",
                    TraceAttributeValue::String(request_policy.reason.to_string()),
                );
            }
            let log_fields = custom_access_log_fields(ctx);
            log_fields.insert(
                "ferron.cache.result".into(),
                CustomAccessLogField::String(result_label.to_string()),
            );
            log_fields.insert(
                "ferron.cache.zone".into(),
                CustomAccessLogField::String(zone_id.label().to_string()),
            );
        }

        ctx.extensions.insert::<RequestStateKey>(RequestState {
            config,
            zone_id,
            base_key,
            request_headers,
            request_cookies,
            private_key,
            purge_url,
            request_policy,
            has_authorization,
            head_only,
            lookup_result,
            store: store.clone(),
            _inflight_guard: inflight_guard,
        });

        Ok(!stop)
    }

    #[inline]
    async fn run_inverse(&self, ctx: &mut HttpContext) -> Result<(), PipelineError> {
        let Some(state) = ctx.extensions.remove::<RequestStateKey>() else {
            return Ok(());
        };

        match &state.lookup_result {
            LookupResult::Hit => return Ok(()),
            LookupResult::StaleWhileRevalidate {
                stats,
                inflight_key,
                entry,
                scope,
                items,
                ..
            } => {
                if inflight_key.is_some() {
                    let mut stats = *stats;
                    stats.expired_evictions += 1;
                    self.emit_eviction_metrics(ctx, &state.zone_id, stats);
                } else {
                    // Serve stale response immediately
                    ctx.res = Some(if entry.body.is_none() {
                        HttpResponse::BuiltinError(
                            entry.status.as_u16(),
                            Some(entry.headers.clone()),
                        )
                    } else {
                        HttpResponse::Custom(build_cached_response(
                            (**entry).clone(),
                            state.head_only,
                            state.config.emit_litespeed_headers,
                        )?)
                    });

                    // Annotate with stale-while-revalidate Cache-Status
                    if let Some(HttpResponse::Custom(ref mut resp)) = ctx.res {
                        annotate_response_headers(
                            resp.headers_mut(),
                            CacheHeaderState::StaleWhileRevalidate {
                                scope: entry.scope,
                                age: entry.age,
                            },
                            state.config.emit_litespeed_headers,
                        );
                    }

                    self.emit_request_metric(ctx, &state.zone_id, "hit", *scope, *items);
                    {
                        let sa = ctx.get_span_attributes();
                        sa.insert(
                            "ferron.cache.result",
                            TraceAttributeValue::String("stale".to_string()),
                        );
                        sa.insert(
                            "ferron.cache.zone",
                            TraceAttributeValue::String(state.zone_id.label().to_string()),
                        );
                        sa.insert(
                            "ferron.cache.scope",
                            TraceAttributeValue::String(entry.scope.as_str().to_string()),
                        );
                        let log_fields = custom_access_log_fields(ctx);
                        log_fields.insert(
                            "ferron.cache.result".into(),
                            CustomAccessLogField::String("stale".into()),
                        );
                        log_fields.insert(
                            "ferron.cache.zone".into(),
                            CustomAccessLogField::String(state.zone_id.label().to_string()),
                        );
                    }
                    return Ok(());
                }
            }
            LookupResult::Revalidate { stats, .. } => {
                self.emit_eviction_metrics(ctx, &state.zone_id, *stats);
            }
            LookupResult::Miss {
                stats,
                inflight_key: _,
            } => self.emit_eviction_metrics(ctx, &state.zone_id, *stats),
            LookupResult::Bypass => {}
        }

        let response = match ctx.res.take() {
            Some(HttpResponse::Custom(response)) => response.map(Some),
            Some(HttpResponse::BuiltinError(status, headers)) => {
                let mut response = Response::new(None);
                *response.status_mut() = StatusCode::from_u16(status)
                    .map_err(|e| PipelineError::custom(e.to_string()))?;
                *response.headers_mut() = headers.unwrap_or_default();
                response
            }
            None => {
                // No response would implicitly mean "404 Not Found"
                let mut response = Response::new(None);
                *response.status_mut() = StatusCode::NOT_FOUND;
                response
            }
            other => {
                ctx.res = other;
                return Ok(());
            }
        };

        // Handle 304 Not Modified from upstream during revalidation
        if let LookupResult::Revalidate {
            entry: ref cached_entry,
            ref cache_key,
            ..
        } = state.lookup_result
        {
            if response.status() == StatusCode::NOT_MODIFIED {
                // Update the cached entry's headers with fresh ones from upstream
                // (e.g., new Date, Cache-Control) but keep the stored body intact.
                let mut fresh_headers = response.headers().clone();
                strip_internal_headers(&mut fresh_headers);
                if let Some(new_fresh_headers) = state.store.update_entry_headers_by_key(
                    cache_key,
                    fresh_headers.clone(),
                    state.config.litespeed_override_cache_control,
                ) {
                    fresh_headers = new_fresh_headers;
                } else {
                    let mut new_fresh_headers = cached_entry.headers.clone();
                    new_fresh_headers.extend(fresh_headers);
                    fresh_headers = new_fresh_headers;
                }

                // Reconstruct a response using fresh headers + cached body
                let mut builder = Response::builder().status(cached_entry.status);
                for (name, value) in &fresh_headers {
                    builder = builder.header(name, value);
                }

                let head_only = state.head_only;
                let body = if head_only {
                    Empty::<Bytes>::new()
                        .map_err(|error| match error {})
                        .boxed_unsync()
                } else if let Some(body) = &cached_entry.body {
                    Full::new(body.clone())
                        .map_err(|error: std::convert::Infallible| match error {})
                        .boxed_unsync()
                } else {
                    Empty::<Bytes>::new()
                        .map_err(|error| match error {})
                        .boxed_unsync()
                };

                if head_only && !fresh_headers.contains_key(header::CONTENT_LENGTH) {
                    if let Some(body_bytes) = &cached_entry.body {
                        if let Ok(value) = HeaderValue::from_str(&body_bytes.len().to_string()) {
                            builder = builder.header(header::CONTENT_LENGTH, value);
                        }
                    }
                }

                let mut response_200 = builder
                    .body(body)
                    .map_err(|e| PipelineError::custom(e.to_string()))?;

                annotate_response_headers(
                    response_200.headers_mut(),
                    CacheHeaderState::Revalidated,
                    state.config.emit_litespeed_headers,
                );

                self.emit_request_metric(
                    ctx,
                    &state.zone_id,
                    "revalidated",
                    Some(cached_entry.scope),
                    state.store.len(),
                );

                ctx.res = Some(HttpResponse::Custom(response_200));
                return Ok(());
            }
        }

        // Handle stale-if-error — serve stale response on upstream 5xx
        if response.status().is_server_error() && state.config.enable_stale_if_error {
            if let (Some((stale_entry, _stale_key, _)), _stats, _len, _had_expired) =
                state.store.lookup(
                    &state.base_key,
                    &state.request_headers,
                    &state.request_cookies,
                    state.private_key.as_deref(),
                )
            {
                if let Some(sie_duration) = stale_entry.stale_if_error {
                    if !stale_entry.must_revalidate
                        && stale_entry.age <= stale_entry.ttl + sie_duration
                    {
                        // Serve the stale response instead of the error
                        let stale_response = if let Some(body) = stale_entry.body {
                            let mut builder = Response::builder().status(stale_entry.status);
                            let mut headers = stale_entry.headers.clone();
                            headers.remove(&LS_CACHE);
                            headers.remove(header::AGE);
                            headers.remove(CACHE_STATUS_HEADER);
                            append_lsc_cookies_as_set_cookie(
                                &mut headers,
                                &stale_entry.lsc_cookies,
                            );
                            annotate_response_headers(
                                &mut headers,
                                CacheHeaderState::StaleWhileRevalidate {
                                    scope: stale_entry.scope,
                                    age: stale_entry.age,
                                },
                                state.config.emit_litespeed_headers,
                            );
                            for (name, value) in &headers {
                                builder = builder.header(name, value);
                            }
                            let body = if state.head_only {
                                Empty::<Bytes>::new()
                                    .map_err(|error| match error {})
                                    .boxed_unsync()
                            } else {
                                Full::new(body)
                                    .map_err(|error: std::convert::Infallible| match error {})
                                    .boxed_unsync()
                            };
                            builder
                                .body(body)
                                .map_err(|e| PipelineError::custom(e.to_string()))?
                        } else {
                            Response::builder()
                                .status(stale_entry.status)
                                .body(
                                    Empty::<Bytes>::new()
                                        .map_err(|error| match error {})
                                        .boxed_unsync(),
                                )
                                .map_err(|e| PipelineError::custom(e.to_string()))?
                        };

                        self.emit_request_metric(
                            ctx,
                            &state.zone_id,
                            "hit",
                            Some(stale_entry.scope),
                            state.store.len(),
                        );

                        ctx.res = Some(HttpResponse::Custom(stale_response));
                        {
                            let sa = ctx.get_span_attributes();
                            sa.insert(
                                "ferron.cache.result",
                                TraceAttributeValue::String("stale".to_string()),
                            );
                            sa.insert(
                                "ferron.cache.zone",
                                TraceAttributeValue::String(state.zone_id.label().to_string()),
                            );
                            sa.insert(
                                "ferron.cache.scope",
                                TraceAttributeValue::String(stale_entry.scope.as_str().to_string()),
                            );
                            let log_fields = custom_access_log_fields(ctx);
                            log_fields.insert(
                                "ferron.cache.result".into(),
                                CustomAccessLogField::String("stale".into()),
                            );
                            log_fields.insert(
                                "ferron.cache.zone".into(),
                                CustomAccessLogField::String(state.zone_id.label().to_string()),
                            );
                        }
                        return Ok(());
                    }
                }
            }
        }

        let mut purge_scope = None;
        let purge_ops = parse_litespeed_purge(response.headers());
        if purge_ops.iter().any(|operation| operation.stale) {
            ctx.events.emit(Event::Log(LogEvent {
                level: LogLevel::Debug,
                target: LOG_TARGET,
                message:
                    "Ignoring unsupported LSCache stale purge marker and performing a hard purge"
                        .to_string(),
                summary: "LSCache stale purge marker ignored".into(),
                attributes: Vec::new(),
                trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
            }));
        }
        if !purge_ops.is_empty() {
            let (stats, items) = state.store.purge(&purge_ops, state.private_key.as_deref());
            for operation in &purge_ops {
                purge_scope = Some(operation.scope);
                self.emit_purge_metric(ctx, &state.zone_id, operation.scope, stats.purged, items);
            }
            if stats.purged > 0 {
                ctx.events.emit(Event::Log(LogEvent {
                    level: LogLevel::Debug,
                    target: LOG_TARGET,
                    message: format!(
                        "Purged {} cache entrie(s) via LSCache controls",
                        stats.purged
                    ),
                    summary: "Cache purged via LSCache controls".into(),
                    attributes: vec![(
                        "cache.purged.count",
                        LogAttributeValue::I64(stats.purged as i64),
                    )],
                    trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
                }));

                // Propagate each purged path to the external control-plane.
                if let Some(url) = &state.config.purge_propagation.control_plane_url {
                    // Collect unique paths from purge operations for propagation.
                    let mut paths: Vec<String> = Vec::new();
                    for op in &purge_ops {
                        for sel in &op.selectors {
                            let path = match sel {
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

                    let url = url.clone();
                    let secret = state.config.purge_propagation.shared_secret.clone();
                    let node_id = state.config.purge_propagation.node_id.clone();
                    if let Some(handle) = SECONDARY_RUNTIME.get() {
                        handle.spawn(async move {
                            for path in &paths {
                                if let Err(e) = propagate_purge_webhook(
                                    &url,
                                    secret.as_deref(),
                                    node_id.as_deref(),
                                    path,
                                )
                                .await
                                {
                                    ferron_core::log_warn!(
                                        "Purge propagation to control-plane failed: {}",
                                        e
                                    );
                                }
                            }
                        });
                    } else {
                        ctx.events.emit(Event::Log(LogEvent {
                            level: LogLevel::Warn,
                            target: LOG_TARGET,
                            message: "Distributed cache purge not yet available".to_string(),
                            summary: "Distributed cache purge not yet available".into(),
                            attributes: Vec::new(),
                            trace_context: ferron_http::trace_context::current_event_trace_context(
                                ctx,
                            ),
                        }));
                    }
                }
            }
        }

        let ls_control = parse_litespeed_cache_control(response.headers());
        let ls_vary = if ls_control.as_ref().is_some_and(|control| control.no_vary) {
            crate::lscache::LiteSpeedVary::default()
        } else {
            parse_litespeed_vary(response.headers())
        };
        let has_unsupported_vary_value = ls_vary.value.is_some();
        if has_unsupported_vary_value {
            ctx.events.emit(Event::Log(LogEvent {
                level: LogLevel::Debug,
                target: LOG_TARGET,
                message:
                    "Skipping cache store because X-LiteSpeed-Vary: value=... is not supported yet"
                        .to_string(),
                summary: "Skipping cache store because X-LiteSpeed-Vary is not supported yet"
                    .into(),
                attributes: Vec::new(),
                trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
            }));
        }
        let has_set_cookie = response.headers().contains_key(header::SET_COOKIE);
        let decision = if !state.request_policy.allow_store
            || (matches!(state.lookup_result, LookupResult::Bypass)
                && state.request_policy.allow_lookup)
            || state.head_only
            || has_unsupported_vary_value
        {
            crate::policy::ResponseCacheDecision {
                store: false,
                scope: None,
                ttl: None,
                stale_while_revalidate: None,
                stale_if_error: None,
                must_revalidate: false,
                reason: if state.head_only {
                    "head-no-store"
                } else if has_unsupported_vary_value {
                    "unsupported-litespeed-vary-value"
                } else {
                    state.request_policy.reason
                },
            }
        } else {
            evaluate_response_policy(
                response.status(),
                response.headers(),
                state.has_authorization,
                has_set_cookie,
                ls_control.as_ref(),
                state.config.litespeed_override_cache_control,
            )
        };

        let vary_rule = build_vary_rule(response.headers(), &state.config, &ls_vary)?;
        let lsc_cookies = collect_lsc_cookies(response.headers());
        let mut response = response;

        if decision.store && vary_rule.is_some() {
            let scope = decision.scope.expect("scope must be set when storing");
            let tags = parse_litespeed_tags(response.headers(), scope);
            let (mut parts, mut body) = response.into_parts();
            parts.extensions.clear(); // Prevent zerocopy from interfering with cache
            strip_internal_headers(&mut parts.headers);
            append_lsc_cookies_as_set_cookie(&mut parts.headers, &lsc_cookies);
            let body_result =
                collect_body_with_limit(body.as_mut(), state.config.max_response_size).await?;

            match body_result {
                CollectBodyOutcome::Complete(body_bytes) => {
                    let (mut outgoing_response, mut stored_headers, status) =
                        if let Some(b) = body_bytes.clone() {
                            let status = parts.status;
                            let headers = parts.headers.clone();
                            let res = response_from_parts(parts, b, state.head_only)?;
                            (HttpResponse::Custom(res), headers, status)
                        } else {
                            (
                                HttpResponse::BuiltinError(
                                    parts.status.as_u16(),
                                    Some(parts.headers.clone()),
                                ),
                                parts.headers,
                                parts.status,
                            )
                        };
                    for header_name in &state.config.ignored_store_headers {
                        stored_headers.remove(header_name);
                    }
                    strip_store_headers(&mut stored_headers);
                    let etag = stored_headers.get(header::ETAG).cloned();
                    let last_modified = stored_headers.get(header::LAST_MODIFIED).cloned();
                    let stored_entry = StoredEntry {
                        scope,
                        base_key: state.base_key.clone(),
                        #[allow(clippy::unnecessary_unwrap)]
                        vary: vary_rule.expect("vary rule must exist"),
                        status,
                        headers: stored_headers,
                        body: body_bytes,
                        lsc_cookies: lsc_cookies.clone(),
                        created_at: std::time::Instant::now(),
                        ttl: decision.ttl.unwrap_or_else(|| Duration::from_secs(0)),
                        access_at: 0,
                        private_key: None,
                        tags,
                        purge_url: state.purge_url,
                        etag,
                        last_modified,
                        stale_while_revalidate: decision.stale_while_revalidate,
                        stale_if_error: decision.stale_if_error,
                        must_revalidate: decision.must_revalidate,
                    };
                    let (stats, items) = state.store.insert_with_request(
                        stored_entry,
                        state.private_key.as_deref(),
                        &state.request_headers,
                        &state.request_cookies,
                    );
                    self.emit_eviction_metrics(ctx, &state.zone_id, stats);
                    self.emit_store_metric(ctx, &state.zone_id, scope, status.as_u16());

                    if let LookupResult::StaleWhileRevalidate {
                        entry,
                        scope,
                        items,
                        ..
                    } = &state.lookup_result
                    {
                        // Serve stale response immediately
                        ctx.res = Some(if entry.body.is_none() {
                            HttpResponse::BuiltinError(
                                entry.status.as_u16(),
                                Some(entry.headers.clone()),
                            )
                        } else {
                            HttpResponse::Custom(build_cached_response(
                                (**entry).clone(),
                                state.head_only,
                                state.config.emit_litespeed_headers,
                            )?)
                        });

                        // Annotate with stale-while-revalidate Cache-Status
                        if let Some(HttpResponse::Custom(ref mut resp)) = ctx.res {
                            annotate_response_headers(
                                resp.headers_mut(),
                                CacheHeaderState::StaleWhileRevalidate {
                                    scope: entry.scope,
                                    age: entry.age,
                                },
                                state.config.emit_litespeed_headers,
                            );
                        }

                        self.emit_request_metric(ctx, &state.zone_id, "hit", *scope, *items);
                        return Ok(());
                    }

                    annotate_response_headers(
                        match &mut outgoing_response {
                            HttpResponse::Custom(r) => r.headers_mut(),
                            HttpResponse::BuiltinError(_, Some(h)) => h,
                            _ => unreachable!(), // These would never be constructed...
                        },
                        CacheHeaderState::Miss {
                            stored: true,
                            detail: decision.reason,
                        },
                        state.config.emit_litespeed_headers,
                    );
                    self.emit_request_metric(ctx, &state.zone_id, "miss", Some(scope), items);
                    ctx.res = Some(outgoing_response);
                    {
                        let sa = ctx.get_span_attributes();
                        sa.insert(
                            "ferron.cache.result",
                            TraceAttributeValue::String("miss".to_string()),
                        );
                        sa.insert(
                            "ferron.cache.zone",
                            TraceAttributeValue::String(state.zone_id.label().to_string()),
                        );
                        sa.insert(
                            "ferron.cache.scope",
                            TraceAttributeValue::String(scope.as_str().to_string()),
                        );
                    }
                }
                CollectBodyOutcome::Overflow { prefix, remainder } => {
                    ctx.events.emit(Event::Log(LogEvent {
                        level: LogLevel::Debug,
                        target: LOG_TARGET,
                        message: "Skipping cache store because the response body exceeded cache.max_response_size".to_string(),
                        summary: "Skipping cache store because response body exceeded maximum size".into(),
                        attributes: Vec::new(),
                        trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
                    }));
                    let mut response = response_from_streaming_parts(parts, prefix, remainder)?;
                    annotate_response_headers(
                        response.headers_mut(),
                        CacheHeaderState::Miss {
                            stored: false,
                            detail: "response-too-large",
                        },
                        state.config.emit_litespeed_headers,
                    );
                    self.emit_request_metric(ctx, &state.zone_id, "miss", None, state.store.len());
                    ctx.res = Some(HttpResponse::Custom(response));
                    {
                        let sa = ctx.get_span_attributes();
                        sa.insert(
                            "ferron.cache.result",
                            TraceAttributeValue::String("miss".to_string()),
                        );
                        sa.insert(
                            "ferron.cache.zone",
                            TraceAttributeValue::String(state.zone_id.label().to_string()),
                        );
                        sa.insert(
                            "ferron.cache.detail",
                            TraceAttributeValue::String("response-too-large".to_string()),
                        );
                    }
                }
            }
        } else {
            strip_internal_headers(response.headers_mut());
            append_lsc_cookies_as_set_cookie(response.headers_mut(), &lsc_cookies);
            annotate_response_headers(
                response.headers_mut(),
                if matches!(state.lookup_result, LookupResult::Bypass) {
                    CacheHeaderState::Bypass {
                        detail: decision.reason,
                    }
                } else {
                    CacheHeaderState::Miss {
                        stored: false,
                        detail: decision.reason,
                    }
                },
                state.config.emit_litespeed_headers,
            );
            let result = if matches!(state.lookup_result, LookupResult::Bypass) {
                "bypass"
            } else {
                "miss"
            };
            self.emit_request_metric(
                ctx,
                &state.zone_id,
                result,
                purge_scope.or(decision.scope),
                state.store.len(),
            );
            {
                let sa = ctx.get_span_attributes();
                sa.insert(
                    "ferron.cache.result",
                    TraceAttributeValue::String(result.to_string()),
                );
                sa.insert(
                    "ferron.cache.zone",
                    TraceAttributeValue::String(state.zone_id.label().to_string()),
                );
                if let Some(scope) = purge_scope.or(decision.scope) {
                    sa.insert(
                        "ferron.cache.scope",
                        TraceAttributeValue::String(scope.as_str().to_string()),
                    );
                }
                if result == "bypass" {
                    sa.insert(
                        "ferron.cache.detail",
                        TraceAttributeValue::String(decision.reason.to_string()),
                    );
                }
            }
            let (parts, body) = response.into_parts();
            ctx.res = Some(if let Some(body) = body {
                HttpResponse::Custom(Response::from_parts(parts, body))
            } else {
                HttpResponse::BuiltinError(parts.status.as_u16(), Some(parts.headers))
            });
        }

        Ok(())
    }
}

enum CacheHeaderState<'a> {
    Hit { scope: CacheScope, age: Duration },
    StaleWhileRevalidate { scope: CacheScope, age: Duration },
    Revalidated,
    Miss { stored: bool, detail: &'a str },
    Bypass { detail: &'a str },
}

fn build_cached_response(
    entry: LookupEntry,
    head_only: bool,
    emit_ls_cache: bool,
) -> Result<Response<UnsyncBoxBody<Bytes, io::Error>>, PipelineError> {
    let body_len = entry.body.as_ref().map(|body| body.len());
    let mut response = Response::new(if head_only {
        Empty::<Bytes>::new()
            .map_err(|error| match error {})
            .boxed_unsync()
    } else if let Some(body) = entry.body {
        Full::new(body)
            .map_err(|error: std::convert::Infallible| match error {})
            .boxed_unsync()
    } else {
        Empty::<Bytes>::new()
            .map_err(|error| match error {})
            .boxed_unsync()
    });
    *response.status_mut() = entry.status;
    let mut headers = entry.headers.clone();
    headers.remove(&LS_CACHE);
    headers.remove(header::AGE);
    headers.remove(CACHE_STATUS_HEADER);
    append_lsc_cookies_as_set_cookie(&mut headers, &entry.lsc_cookies);
    annotate_response_headers(
        &mut headers,
        CacheHeaderState::Hit {
            scope: entry.scope,
            age: entry.age,
        },
        emit_ls_cache,
    );

    if head_only && !headers.contains_key(header::CONTENT_LENGTH) {
        if let Some(body_len) = body_len {
            let value = HeaderValue::from_str(&body_len.to_string())
                .map_err(|error| PipelineError::custom(error.to_string()))?;
            headers.insert(header::CONTENT_LENGTH, value);
        }
    }

    *response.headers_mut() = headers;
    Ok(response)
}

/// Resolve the cache zone ID for a request.
///
/// Resolution order:
/// 1. If the host specifies `zone "name"` → `CacheZoneId::Named(name)`
/// 2. If the host specifies `max_entries` in its cache block (without `zone`)
///    → `CacheZoneId::Host(hostname)` — the host wants its own capacity
/// 3. If a global `cache { max_entries = N }` block exists (no explicit `zone`
///    blocks in it) → `CacheZoneId::Global` (all hosts share one store)
/// 4. Otherwise → `CacheZoneId::Host(hostname)` (per-host store)
fn resolve_zone_id(
    hostname: &Option<String>,
    config: &CacheConfig,
    configuration: &ferron_core::config::layer::LayeredConfiguration,
) -> CacheZoneId {
    if let Some(ref zone) = config.zone {
        zone.clone()
    } else if has_host_max_entries(configuration) {
        // Host specifies max_entries → implicit per-host zone
        CacheZoneId::Host(hostname.clone().unwrap_or_else(|| "_default".to_string()))
    } else if crate::config::has_global_zone(configuration) {
        CacheZoneId::Global
    } else {
        CacheZoneId::Host(hostname.clone().unwrap_or_else(|| "_default".to_string()))
    }
}

fn build_base_key(
    encrypted: bool,
    headers: &HeaderMap,
    original_uri: Option<&http::Uri>,
    fallback_uri: &http::Uri,
) -> String {
    let uri = original_uri.unwrap_or(fallback_uri);
    let scheme = if encrypted { "https" } else { "http" };
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let path_and_query = uri
        .path_and_query()
        .map(|value| value.as_str())
        .unwrap_or("/");
    let mut key = String::with_capacity(scheme.len() + host.len() + path_and_query.len() + 3);
    key.push_str(scheme);
    key.push_str("://");
    key.push_str(host);
    key.push_str(path_and_query);
    key
}

fn parse_cookies(headers: &HeaderMap) -> AHashMap<String, String> {
    let mut cookies = AHashMap::default();
    for value in headers.get_all(header::COOKIE) {
        let Some(text) = value.to_str().ok() else {
            continue;
        };
        for cookie in text.split(';') {
            let Some((name, value)) = cookie.split_once('=') else {
                continue;
            };
            let name = name.trim();
            let value = value.trim();
            if !name.is_empty() {
                cookies.insert(name.to_string(), value.to_string());
            }
        }
    }
    cookies
}

fn build_private_cache_key(
    cookies: &AHashMap<String, String>,
    remote_ip: std::net::IpAddr,
    auth_user: Option<&str>,
) -> String {
    let mut components = Vec::with_capacity(cookies.len() + 2);
    components.push(format!("ip={remote_ip}"));
    if let Some(auth_user) = auth_user {
        components.push(format!("auth={auth_user}"));
    }

    let mut matched_private_cookie = false;
    for (name, value) in cookies {
        // Check if the cookie is a private cookie based on its name.
        let is_private = is_private_cookie_name(name);
        if is_private && value.len() >= 16 {
            matched_private_cookie = true;
            components.push(format!("cookie:{name}={value}"));
        }
    }

    if !matched_private_cookie {
        for (name, value) in cookies {
            components.push(format!("cookie:{name}={value}"));
        }
    }

    components.sort_unstable();
    components.join("\0")
}

fn build_vary_rule(
    headers: &HeaderMap,
    config: &CacheConfig,
    ls_vary: &crate::lscache::LiteSpeedVary,
) -> Result<Option<VaryRule>, PipelineError> {
    let mut header_names: AHashSet<HeaderName> = config.vary_headers.iter().cloned().collect();
    for value in headers.get_all(header::VARY) {
        let Some(text) = value.to_str().ok() else {
            continue;
        };
        for token in text.split(',') {
            let token = token.trim();
            if token == "*" {
                return Ok(None);
            }
            if token.is_empty() {
                continue;
            }
            let name = HeaderName::from_bytes(token.as_bytes())
                .map_err(|error| PipelineError::custom(error.to_string()))?;
            header_names.insert(name);
        }
    }
    let mut header_names: Vec<_> = header_names.into_iter().collect();
    header_names.sort_by(|left, right| left.as_str().cmp(right.as_str()));

    let mut cookie_names = ls_vary.cookies.clone();
    cookie_names.sort_unstable();

    Ok(Some(VaryRule {
        header_names,
        cookie_names,
        value: None,
    }))
}

async fn collect_body_with_limit(
    body: Option<&mut UnsyncBoxBody<Bytes, io::Error>>,
    max_size: usize,
) -> Result<CollectBodyOutcome, PipelineError> {
    let Some(body) = body else {
        return Ok(CollectBodyOutcome::Complete(None));
    };
    let initial_capacity = body
        .size_hint()
        .upper()
        .and_then(|upper| usize::try_from(upper).ok())
        .map(|cap| cap.min(max_size))
        .unwrap_or(0);
    let mut buffer = BytesMut::with_capacity(initial_capacity);
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|error| PipelineError::custom(error.to_string()))?;
        if let Some(data) = frame.data_ref() {
            buffer.extend_from_slice(data);
            if buffer.len() > max_size {
                let remainder = std::mem::replace(
                    body,
                    Empty::<Bytes>::new()
                        .map_err(|error| match error {})
                        .boxed_unsync(),
                );
                return Ok(CollectBodyOutcome::Overflow {
                    prefix: buffer.freeze(),
                    remainder,
                });
            }
        }
    }

    Ok(CollectBodyOutcome::Complete(Some(buffer.freeze())))
}

fn response_from_parts(
    parts: http::response::Parts,
    body: Bytes,
    head_only: bool,
) -> Result<Response<UnsyncBoxBody<Bytes, io::Error>>, PipelineError> {
    let body = if head_only {
        Empty::<Bytes>::new()
            .map_err(|error| match error {})
            .boxed_unsync()
    } else {
        Full::new(body)
            .map_err(|error: std::convert::Infallible| match error {})
            .boxed_unsync()
    };
    Ok(Response::from_parts(parts, body))
}

#[inline]
fn response_from_streaming_parts(
    parts: http::response::Parts,
    prefix: Bytes,
    remainder: UnsyncBoxBody<Bytes, io::Error>,
) -> Result<Response<UnsyncBoxBody<Bytes, io::Error>>, PipelineError> {
    let prefix_stream = stream::once(async move { Ok(Frame::data(prefix)) });
    let chained = prefix_stream.chain(BodyStream::new(remainder));
    let body = StreamBody::new(chained).boxed_unsync();
    Ok(Response::from_parts(parts, body))
}

fn annotate_response_headers(
    headers: &mut HeaderMap,
    state: CacheHeaderState<'_>,
    emit_ls_cache: bool,
) {
    if emit_ls_cache {
        headers.remove(&LS_CACHE);
    }
    headers.remove(CACHE_STATUS_HEADER);
    headers.remove(header::AGE);

    match state {
        CacheHeaderState::Hit { scope, age } => {
            if emit_ls_cache {
                let ls_value = if scope == CacheScope::Private {
                    "hit,private"
                } else {
                    "hit"
                };
                headers.insert(&LS_CACHE, HeaderValue::from_static(ls_value));
            }
            if let Ok(age_value) = HeaderValue::from_str(&age.as_secs().to_string()) {
                headers.insert(header::AGE, age_value);
            }
            let mut value = String::with_capacity(48 + scope.as_str().len());
            value.push_str("FerronCache; hit; detail=");
            value.push_str(scope.as_str());
            value.push_str("; age=");
            value.push_str(&age.as_secs().to_string());
            if let Ok(value) = HeaderValue::from_str(&value) {
                headers.insert(CACHE_STATUS_HEADER, value);
            }
        }
        CacheHeaderState::StaleWhileRevalidate { scope, age } => {
            if emit_ls_cache {
                let ls_value = if scope == CacheScope::Private {
                    "hit,private"
                } else {
                    "hit"
                };
                headers.insert(&LS_CACHE, HeaderValue::from_static(ls_value));
            }
            if let Ok(age_value) = HeaderValue::from_str(&age.as_secs().to_string()) {
                headers.insert(header::AGE, age_value);
            }
            let mut value = String::with_capacity(70 + scope.as_str().len());
            value.push_str("FerronCache; hit; detail=stale-while-revalidate,");
            value.push_str(scope.as_str());
            value.push_str("; age=");
            value.push_str(&age.as_secs().to_string());
            if let Ok(value) = HeaderValue::from_str(&value) {
                headers.insert(CACHE_STATUS_HEADER, value);
            }
        }
        CacheHeaderState::Revalidated => {
            if emit_ls_cache {
                headers.insert(&LS_CACHE, HeaderValue::from_static("hit"));
            }
            if let Ok(value) = HeaderValue::from_str("FerronCache; fwd=hit; detail=revalidated") {
                headers.insert(CACHE_STATUS_HEADER, value);
            }
        }
        CacheHeaderState::Miss { stored, detail } => {
            if emit_ls_cache {
                headers.insert(&LS_CACHE, HeaderValue::from_static("miss"));
            }
            let mut value = String::with_capacity(40 + detail.len());
            value.push_str("FerronCache; fwd=miss; stored=");
            value.push_str(if stored { "true" } else { "false" });
            value.push_str("; detail=");
            value.push_str(detail);
            if let Ok(value) = HeaderValue::from_str(&value) {
                headers.insert(CACHE_STATUS_HEADER, value);
            }
        }
        CacheHeaderState::Bypass { detail } => {
            if emit_ls_cache {
                headers.insert(&LS_CACHE, HeaderValue::from_static("bypass"));
            }
            let mut value = String::with_capacity(32 + detail.len());
            value.push_str("FerronCache; fwd=bypass; detail=");
            value.push_str(detail);
            if let Ok(value) = HeaderValue::from_str(&value) {
                headers.insert(CACHE_STATUS_HEADER, value);
            }
        }
    }
}

#[inline]
fn strip_internal_headers(headers: &mut HeaderMap) {
    headers.remove(&LS_CACHE_CONTROL);
    headers.remove(&LS_TAG);
    headers.remove(&LS_PURGE);
    headers.remove(&LS_VARY);
    headers.remove(&LS_COOKIE);
    headers.remove(&LS_CACHE);
    headers.remove(CACHE_STATUS_HEADER);
}

#[inline]
fn append_lsc_cookies_as_set_cookie(headers: &mut HeaderMap, lsc_cookies: &[HeaderValue]) {
    headers.remove(&LS_COOKIE);
    for cookie in lsc_cookies {
        headers.append(header::SET_COOKIE, cookie.clone());
    }
}

#[inline]
fn is_private_cookie_name(name: &str) -> bool {
    PRIVATE_COOKIE_NAMES
        .iter()
        .any(|candidate| name.eq_ignore_ascii_case(candidate))
        || starts_with_ignore_ascii_case(name, "wp_woocommerce_session_")
}

#[inline]
fn starts_with_ignore_ascii_case(value: &str, prefix: &str) -> bool {
    value
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

/// Build an HTTPS client for outbound purge propagation webhooks.
fn build_propagation_client() -> Result<
    hyper_util::client::legacy::Client<
        hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
        http_body_util::Full<bytes::Bytes>,
    >,
    Box<dyn std::error::Error + Send + Sync>,
> {
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
async fn propagate_purge_webhook(
    url: &str,
    shared_secret: Option<&str>,
    node_id: Option<&str>,
    path: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let client = build_propagation_client()?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use ferron_core::config::layer::LayeredConfiguration;
    use ferron_observability::CompositeEventSink;
    use http::Request;
    use std::net::SocketAddr;

    #[inline]
    fn test_context(path: &str) -> HttpContext {
        let request = Request::builder()
            .uri(path)
            .header(header::HOST, "example.com")
            .body(
                Empty::<Bytes>::new()
                    .map_err(|error: std::convert::Infallible| match error {})
                    .boxed_unsync(),
            )
            .unwrap();

        HttpContext {
            req: Some(request),
            res: None,
            events: CompositeEventSink::new(Vec::new()),
            configuration: LayeredConfiguration::default(),
            hostname: Some("example.com".to_string()),
            variables: FxHashMap::default(),
            previous_error: None,
            original_uri: None,
            routing_uri: None,
            encrypted: true,
            local_address: "127.0.0.1:443".parse::<SocketAddr>().unwrap(),
            remote_address: "127.0.0.2:12345".parse::<SocketAddr>().unwrap(),
            auth_user: None,
            https_port: Some(443),
            extensions: typemap_rev::TypeMap::new(),
        }
    }

    #[test]
    #[inline]
    fn parses_private_key_from_cookies() {
        let mut cookies = AHashMap::default();
        cookies.insert("PHPSESSID".to_string(), "1234567890abcdef".to_string());
        let key = build_private_cache_key(&cookies, "127.0.0.1".parse().unwrap(), Some("user"));
        assert!(key.contains("auth=user"));
        assert!(key.contains("cookie:PHPSESSID=1234567890abcdef"));
    }

    #[tokio::test]
    #[inline]
    async fn hit_response_uses_empty_body_for_head() {
        let entry = LookupEntry {
            scope: CacheScope::Public,
            status: http::StatusCode::OK,
            headers: HeaderMap::new(),
            body: Some(Bytes::from_static(b"hello")),
            lsc_cookies: Vec::new(),
            age: Duration::from_secs(5),
            etag: None,
            last_modified: None,
            stale_while_revalidate: None,
            stale_if_error: None,
            must_revalidate: false,
            ttl: Duration::from_secs(60),
        };
        let response = build_cached_response(entry, true, false).unwrap();
        let collected = response.into_body().collect().await.unwrap().to_bytes();
        assert!(collected.is_empty());
    }

    #[test]
    #[inline]
    fn base_key_uses_scheme_host_and_path() {
        let ctx = test_context("/test?q=1");
        let request = ctx.req.as_ref().unwrap();
        let key = build_base_key(ctx.encrypted, request.headers(), None, request.uri());
        assert_eq!(key, "https://example.com/test?q=1");
    }

    #[test]
    #[inline]
    fn base_key_prefers_original_uri() {
        let mut ctx = test_context("/rewritten/path");
        ctx.original_uri = Some("/canonical/path".parse().unwrap());
        let request = ctx.req.as_ref().unwrap();
        let key = build_base_key(
            ctx.encrypted,
            request.headers(),
            ctx.original_uri.as_ref(),
            request.uri(),
        );
        assert_eq!(key, "https://example.com/canonical/path");
    }
}
