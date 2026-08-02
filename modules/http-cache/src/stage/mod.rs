mod key;
mod metrics;
mod purge_propagation;
mod response_helpers;
#[cfg(test)]
mod tests;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use ahash::AHashMap;
use async_trait::async_trait;
use bytes::Bytes;
use dashmap::DashMap;
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::StageConstraint;
use ferron_http::access_log::{custom_access_log_fields, CustomAccessLogField};
use ferron_http::span::HttpContextSpanExt;
use ferron_http::{HttpContext, HttpResponse};
use ferron_observability::{Event, LogAttributeValue, LogEvent, LogLevel, TraceAttributeValue};
use http::header::{self, HeaderValue};
use http::{HeaderMap, Method, Response, StatusCode};
use http_body_util::{BodyExt, Empty, Full};
use typemap_rev::TypeMapKey;

use crate::config::{
    has_host_max_entries, parse_cache_config, parse_max_entries, CacheConfig, CacheZoneId,
};
use crate::lscache::{
    collect_lsc_cookies, parse_litespeed_cache_control, parse_litespeed_purge,
    parse_litespeed_tags, parse_litespeed_vary, PurgeOperation, PurgeSelector, LS_CACHE,
};
use crate::policy::{
    evaluate_response_policy, parse_request_policy, CacheScope, RequestCachePolicy,
};
use crate::store::{CacheStore, LookupEntry, LookupOutcome, StoreStats, StoredEntry};
use crate::SECONDARY_RUNTIME;

use self::key::{
    build_base_key, build_private_cache_key, build_vary_rule, cache_key_fingerprint, parse_cookies,
};
use self::metrics::{
    emit_eviction_metrics, emit_purge_metric, emit_request_metric, emit_singleflight_metrics,
    emit_store_metric,
};
use self::purge_propagation::{propagate_purge_webhook, PURGE_SOURCE_HEADER};
use self::response_helpers::{
    annotate_response_headers, append_lsc_cookies_as_set_cookie, build_cached_response,
    collect_body_with_limit, response_from_parts, response_from_streaming_parts,
    strip_internal_headers, CacheHeaderState, CollectBodyOutcome,
};

const LOG_TARGET: &str = "ferron-http-cache";
const CACHE_STATUS_HEADER: http::header::HeaderName =
    http::header::HeaderName::from_static("cache-status");

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
    StaleWhileRevalidate {
        entry: Box<LookupEntry>,
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
                CacheZoneId::Named(name) => {
                    crate::config::parse_global_zone_max_entries(configuration, name)
                        .unwrap_or(crate::config::DEFAULT_MAX_CACHE_ENTRIES)
                }
                CacheZoneId::Global => parse_max_entries(configuration),
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

        if method_purge {
            if !config.purge_method {
                // PURGE not enabled — fall through to 405 from downstream stages
            } else {
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
                    log_fields.insert(
                        "ferron.cache.key_fingerprint".into(),
                        CustomAccessLogField::String(cache_key_fingerprint(&purge_url)),
                    );
                    return Ok(false);
                }

                let mut purged = 0;
                for scope in [CacheScope::Public, CacheScope::Private] {
                    let purge_ops = vec![PurgeOperation {
                        scope,
                        selectors: vec![PurgeSelector::UrlPath(request.uri().path().to_string())],
                        stale: false,
                    }];
                    let (stats, items) = store.purge(&purge_ops, None);
                    if stats.purged > 0 {
                        emit_purge_metric(ctx, &zone_id, scope, stats.purged, items);
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
                            let events = ctx.events.clone();
                            let trace_context =
                                ferron_http::trace_context::current_event_trace_context(ctx);
                            handle.spawn(async move {
                                if let Err(e) = propagate_purge_webhook(
                                    &url,
                                    secret.as_deref(),
                                    node_id.as_deref(),
                                    &path,
                                )
                                .await
                                {
                                    events.emit(Event::Log(LogEvent {
                                        level: LogLevel::Warn,
                                        target: LOG_TARGET,
                                        message: format!(
                                            "Purge propagation to control-plane failed: {}",
                                            e
                                        ),
                                        summary: "Purge propagation failed".into(),
                                        attributes: vec![(
                                            "error.message",
                                            LogAttributeValue::String(e.to_string()),
                                        )],
                                        trace_context,
                                    }));
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
                    log_fields.insert(
                        "ferron.cache.key_fingerprint".into(),
                        CustomAccessLogField::String(cache_key_fingerprint(&purge_url)),
                    );
                }
                return Ok(false);
            }
        }

        let request_is_lookup_eligible = method_cacheable
            && !request_headers.contains_key(header::RANGE)
            && !request_headers.contains_key(header::UPGRADE)
            && request_policy.allow_lookup;

        let lookup_result = if request_is_lookup_eligible {
            let LookupOutcome {
                entry: lookup,
                stats,
                items,
                had_expired,
            } = store.lookup(
                &base_key,
                &request_headers,
                &request_cookies,
                private_key.as_deref(),
            );
            if let Some((entry, cache_key, hit_kind)) = lookup {
                let scope = entry.scope;

                if request_policy.reason == "request-revalidation" {
                    emit_request_metric(ctx, &zone_id, "hit", Some(scope), items);
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
                            stats,
                            inflight_key: is_leader.then_some(base_key.clone()),
                            scope: Some(scope),
                            items,
                        }
                    } else {
                        emit_request_metric(ctx, &zone_id, "hit", Some(scope), items);
                        LookupResult::Revalidate {
                            entry: Box::new(entry),
                            cache_key,
                            stats,
                        }
                    }
                } else {
                    emit_request_metric(ctx, &zone_id, "hit", Some(scope), items);
                    emit_eviction_metrics(ctx, &zone_id, stats);
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
                        log_fields.insert(
                            "ferron.cache.key_fingerprint".into(),
                            CustomAccessLogField::String(cache_key_fingerprint(&base_key)),
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
                if had_expired {
                    let coalesce_start = std::time::Instant::now();
                    let (is_leader, notify) = store.begin_fetch(&base_key);

                    if !is_leader {
                        notify.notified().await;
                        let wait_ms = coalesce_start.elapsed().as_secs_f64() * 1000.0;
                        emit_singleflight_metrics(ctx, &store);

                        let LookupOutcome {
                            entry: retry_lookup,
                            stats: retry_stats,
                            items: retry_items,
                            ..
                        } = store.lookup(
                            &base_key,
                            &request_headers,
                            &request_cookies,
                            private_key.as_deref(),
                        );
                        if let Some((entry, _, _)) = retry_lookup {
                            let scope = entry.scope;
                            emit_eviction_metrics(ctx, &zone_id, retry_stats);
                            emit_request_metric(ctx, &zone_id, "hit", Some(scope), retry_items);
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
                                log_fields.insert(
                                    "ferron.cache.key_fingerprint".into(),
                                    CustomAccessLogField::String(cache_key_fingerprint(&base_key)),
                                );
                                log_fields.insert(
                                    "ferron.cache.coalesced".into(),
                                    CustomAccessLogField::Bool(true),
                                );
                                log_fields.insert(
                                    "ferron.cache.coalesce_wait_duration_ms".into(),
                                    CustomAccessLogField::F64(wait_ms),
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
                        LookupResult::Miss {
                            stats: retry_stats,
                            inflight_key: None,
                        }
                    } else {
                        LookupResult::Miss {
                            stats,
                            inflight_key: Some(base_key.clone()),
                        }
                    }
                } else {
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
            let req_method = ctx.req.as_ref().map(|r| r.method().as_str().to_string());
            let req_uri = ctx.req.as_ref().map(|r| r.uri().to_string());
            let sa = ctx.get_span_attributes();
            sa.insert(
                "ferron.cache.result",
                TraceAttributeValue::String(result_label.to_string()),
            );
            sa.insert(
                "ferron.cache.zone",
                TraceAttributeValue::String(zone_id.label().to_string()),
            );
            if let Some(uri) = req_uri {
                sa.insert("ferron.cache.key.uri", TraceAttributeValue::String(uri));
            }
            if let Some(method) = req_method {
                sa.insert(
                    "ferron.cache.key.method",
                    TraceAttributeValue::String(method),
                );
            }
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
            log_fields.insert(
                "ferron.cache.key_fingerprint".into(),
                CustomAccessLogField::String(cache_key_fingerprint(&base_key)),
            );
            log_fields.insert(
                "ferron.cache.coalesced".into(),
                CustomAccessLogField::Bool(false),
            );
            log_fields.insert(
                "ferron.cache.coalesce_wait_duration_ms".into(),
                CustomAccessLogField::F64(0.0),
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
                    emit_eviction_metrics(ctx, &state.zone_id, stats);
                } else {
                    emit_eviction_metrics(ctx, &state.zone_id, *stats);
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

                    emit_request_metric(ctx, &state.zone_id, "hit", *scope, *items);
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
                        log_fields.insert(
                            "ferron.cache.key_fingerprint".into(),
                            CustomAccessLogField::String(cache_key_fingerprint(&state.base_key)),
                        );
                    }
                    return Ok(());
                }
            }
            LookupResult::Revalidate { stats, .. } => {
                emit_eviction_metrics(ctx, &state.zone_id, *stats);
            }
            LookupResult::Miss {
                stats,
                inflight_key: _,
            } => emit_eviction_metrics(ctx, &state.zone_id, *stats),
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
                let mut response = Response::new(None);
                *response.status_mut() = StatusCode::NOT_FOUND;
                response
            }
            other => {
                ctx.res = other;
                return Ok(());
            }
        };

        if let LookupResult::Revalidate {
            entry: ref cached_entry,
            ref cache_key,
            ..
        } = state.lookup_result
        {
            if response.status() == StatusCode::NOT_MODIFIED {
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

                emit_request_metric(
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

        if response.status().is_server_error() && state.config.enable_stale_if_error {
            if let LookupOutcome {
                entry: Some((stale_entry, _, _)),
                ..
            } = state.store.lookup(
                &state.base_key,
                &state.request_headers,
                &state.request_cookies,
                state.private_key.as_deref(),
            ) {
                if let Some(sie_duration) = stale_entry.stale_if_error {
                    if !stale_entry.must_revalidate
                        && stale_entry.age <= stale_entry.ttl + sie_duration
                    {
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

                        emit_request_metric(
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
                            log_fields.insert(
                                "ferron.cache.key_fingerprint".into(),
                                CustomAccessLogField::String(cache_key_fingerprint(
                                    &state.base_key,
                                )),
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
                emit_purge_metric(ctx, &state.zone_id, operation.scope, stats.purged, items);
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

                if let Some(url) = &state.config.purge_propagation.control_plane_url {
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
                        let events = ctx.events.clone();
                        let trace_context =
                            ferron_http::trace_context::current_event_trace_context(ctx);
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
                                    events.emit(Event::Log(LogEvent {
                                        level: LogLevel::Warn,
                                        target: LOG_TARGET,
                                        message: format!(
                                            "Purge propagation to control-plane failed: {}",
                                            e
                                        ),
                                        summary: "Purge propagation failed".into(),
                                        attributes: vec![(
                                            "error.message",
                                            LogAttributeValue::String(e.to_string()),
                                        )],
                                        trace_context: trace_context.clone(),
                                    }));
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
            let vary_rule_cloned = vary_rule.clone().expect("vary rule must exist");
            let tags = parse_litespeed_tags(response.headers(), scope);
            let (mut parts, mut body) = response.into_parts();
            parts.extensions.clear();
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
                    crate::store::strip_store_headers(&mut stored_headers);
                    let etag = stored_headers.get(header::ETAG).cloned();
                    let last_modified = stored_headers.get(header::LAST_MODIFIED).cloned();
                    let stored_entry = StoredEntry {
                        scope,
                        base_key: state.base_key.clone(),
                        vary: vary_rule_cloned,
                        status,
                        headers: stored_headers,
                        body: body_bytes,
                        lsc_cookies: lsc_cookies.clone(),
                        created_at: std::time::Instant::now(),
                        ttl: decision
                            .ttl
                            .unwrap_or_else(|| std::time::Duration::from_secs(0)),
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
                    emit_eviction_metrics(ctx, &state.zone_id, stats);
                    emit_store_metric(ctx, &state.zone_id, scope, status.as_u16());

                    if let LookupResult::StaleWhileRevalidate {
                        entry,
                        scope,
                        items,
                        ..
                    } = &state.lookup_result
                    {
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

                        emit_request_metric(ctx, &state.zone_id, "hit", *scope, *items);
                        return Ok(());
                    }

                    annotate_response_headers(
                        match &mut outgoing_response {
                            HttpResponse::Custom(r) => r.headers_mut(),
                            HttpResponse::BuiltinError(_, Some(h)) => h,
                            _ => unreachable!(),
                        },
                        CacheHeaderState::Miss {
                            stored: true,
                            detail: decision.reason,
                        },
                        state.config.emit_litespeed_headers,
                    );
                    emit_request_metric(ctx, &state.zone_id, "miss", Some(scope), items);
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
                        if let Some(ref vr) = vary_rule {
                            if !vr.cookie_names.is_empty() {
                                sa.insert(
                                    "ferron.cache.key.evaluated_cookies",
                                    TraceAttributeValue::String(vr.cookie_names.join(";")),
                                );
                            }
                        }
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
                    emit_request_metric(ctx, &state.zone_id, "miss", None, state.store.len());
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
            emit_request_metric(
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

/// Resolve the cache zone ID for a request.
fn resolve_zone_id(
    hostname: &Option<String>,
    config: &CacheConfig,
    configuration: &ferron_core::config::layer::LayeredConfiguration,
) -> CacheZoneId {
    if let Some(ref zone) = config.zone {
        zone.clone()
    } else if has_host_max_entries(configuration) {
        CacheZoneId::Host(hostname.clone().unwrap_or_else(|| "_default".to_string()))
    } else if crate::config::has_global_zone(configuration) {
        CacheZoneId::Global
    } else {
        CacheZoneId::Host(hostname.clone().unwrap_or_else(|| "_default".to_string()))
    }
}
