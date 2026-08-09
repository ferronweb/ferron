mod key;
mod outcome;
mod purge;
mod response_helpers;
mod served;
#[cfg(test)]
mod tests;

use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use ahash::AHashMap;
use async_trait::async_trait;
use bytes::Bytes;
use cidr::IpCidr;
use dashmap::DashMap;
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::StageConstraint;
use ferron_http::{HttpContext, HttpResponse};
use ferron_observability::{Event, LogEvent, LogLevel};
use http::header;
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
    evaluate_response_policy, parse_request_policy, satisfies_freshness_constraints, CacheScope,
    RequestCachePolicy,
};
use crate::store::{
    merge_revalidation_headers, CacheStore, LookupEntry, LookupOutcome, StoreStats, StoredEntry,
};

use self::key::{build_base_key, build_private_cache_key, build_vary_rule, parse_cookies};
use self::outcome::{
    emit_eviction_metrics, emit_request_metric, emit_singleflight_metrics, emit_store_metric,
    report, CacheOutcome,
};
use self::purge::{purge, PURGE_SECRET_HEADER, PURGE_SOURCE_HEADER};
use self::response_helpers::{
    annotate_response_headers, append_lsc_cookies_as_set_cookie, collect_body_with_limit,
    response_from_parts, response_from_streaming_parts, strip_internal_headers, CacheHeaderState,
    CollectBodyOutcome,
};
use self::served::{serve, ServedState};

const LOG_TARGET: &str = "ferron-http-cache";
const CACHE_STATUS_HEADER: http::header::HeaderName =
    http::header::HeaderName::from_static("cache-status");

struct RequestStateKey;

impl TypeMapKey for RequestStateKey {
    type Value = RequestState;
}

struct RequestState {
    config: Arc<CacheConfig>,
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
    /// Parsed cache configs keyed by hostname, cleared on config reload.
    configs: Arc<DashMap<String, Arc<CacheConfig>>>,
    /// Config generation at which `configs` was last filled.
    config_generation: Arc<AtomicU64>,
}

impl HttpCacheStage {
    #[inline]
    pub fn new() -> Self {
        Self {
            zones: Arc::new(DashMap::new()),
            zone_generations: Arc::new(DashMap::new()),
            configs: Arc::new(DashMap::new()),
            config_generation: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Get the cache config for the request's hostname, parsing it once per
    /// hostname per configuration generation. The configuration can differ
    /// per host within a generation, so the cache is keyed by hostname
    /// rather than by generation alone.
    #[inline]
    fn get_config(&self, ctx: &HttpContext) -> Arc<CacheConfig> {
        let current_gen = active_config_generation();
        if self.config_generation.load(Ordering::Relaxed) != current_gen {
            self.configs.clear();
            self.config_generation.store(current_gen, Ordering::Relaxed);
        }
        let hostname = ctx
            .hostname
            .clone()
            .unwrap_or_else(|| "_default".to_string());
        if let Some(config) = self.configs.get(&hostname) {
            // Fast path (read lock instead of write lock)
            return config.clone();
        }
        self.configs
            .entry(hostname)
            .or_insert_with(|| Arc::new(parse_cache_config(&ctx.configuration)))
            .clone()
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

        let current_gen = active_config_generation();

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
        let config = self.get_config(ctx);

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
                ..Default::default()
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
                let is_propagated = request_headers
                    .get(&PURGE_SOURCE_HEADER)
                    .and_then(|v| v.to_str().ok())
                    .is_some_and(|v| v.eq_ignore_ascii_case("propagation"));

                // A propagation claim must prove knowledge of the shared
                // secret. Without this, any client could tag a PURGE as
                // propagated and bypass the normal authorization (B#9).
                if is_propagated
                    && !propagation_secret_verified(
                        &request_headers,
                        config.purge_propagation.shared_secret.as_deref(),
                    )
                {
                    ctx.res = Some(HttpResponse::BuiltinError(403, None));
                    report(
                        ctx,
                        CacheOutcome {
                            result: "purge_rejected",
                            zone_id: &zone_id,
                            key: &purge_url,
                            scope: None,
                            items: None,
                            stored: None,
                            evictions: None,
                            detail: Some("propagation-secret-mismatch"),
                            key_uri: None,
                            key_method: None,
                            bypass_reason: None,
                            evaluated_cookies: None,
                            coalesced_wait_ms: None,
                            mark_uncoalesced: false,
                            metric_result: None,
                        },
                    );
                    return Ok(false);
                }

                // Non-propagated purges require an allow-listed IP or an
                // authenticated user with a basic_auth block in scope. A
                // basic_auth block must be in scope for this request: an
                // authenticated user from a foreign host's basic_auth must not
                // be able to purge this host's cache.
                if !is_propagated {
                    let has_basic_auth_in_scope =
                        !ctx.configuration.get_entries("basic_auth", true).is_empty();
                    let purge_allowed = purge_allowed(
                        ctx.remote_address.ip().to_canonical(),
                        &config.purge_allowed_ips,
                        has_basic_auth_in_scope,
                        ctx.auth_user.as_deref(),
                    );

                    if !purge_allowed {
                        ctx.res = Some(HttpResponse::BuiltinError(403, None));
                        report(
                            ctx,
                            CacheOutcome {
                                result: "purge_rejected",
                                zone_id: &zone_id,
                                key: &purge_url,
                                scope: None,
                                items: None,
                                stored: None,
                                evictions: None,
                                detail: None,
                                key_uri: None,
                                key_method: None,
                                bypass_reason: None,
                                evaluated_cookies: None,
                                coalesced_wait_ms: None,
                                mark_uncoalesced: false,
                                metric_result: None,
                            },
                        );
                        return Ok(false);
                    }
                }

                let purge_ops: Vec<PurgeOperation> = [CacheScope::Public, CacheScope::Private]
                    .iter()
                    .map(|scope| PurgeOperation {
                        scope: *scope,
                        selectors: vec![PurgeSelector::UrlPath(request.uri().path().to_string())],
                        stale: false,
                    })
                    .collect();
                purge(
                    ctx,
                    &zone_id,
                    &store,
                    &purge_ops,
                    None,
                    entry_host(&ctx.hostname, &zone_id).as_deref(),
                    !is_propagated,
                    &config.purge_propagation,
                );

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
                report(
                    ctx,
                    CacheOutcome {
                        result: "purge",
                        zone_id: &zone_id,
                        key: &purge_url,
                        scope: None,
                        items: None,
                        stored: None,
                        evictions: None,
                        detail: None,
                        key_uri: None,
                        key_method: None,
                        bypass_reason: None,
                        evaluated_cookies: None,
                        coalesced_wait_ms: None,
                        mark_uncoalesced: false,
                        metric_result: None,
                    },
                );
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

                if request_policy.reason == "request-revalidation"
                    || !satisfies_freshness_constraints(&request_policy, entry.age, entry.ttl)
                {
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
                    report(
                        ctx,
                        CacheOutcome {
                            result: "hit",
                            zone_id: &zone_id,
                            key: &base_key,
                            scope: Some(scope),
                            items: Some(items),
                            stored: None,
                            evictions: Some(stats),
                            detail: None,
                            key_uri: None,
                            key_method: None,
                            bypass_reason: None,
                            evaluated_cookies: None,
                            coalesced_wait_ms: None,
                            mark_uncoalesced: false,
                            metric_result: None,
                        },
                    );
                    ctx.res = Some(if entry.body.is_none() {
                        HttpResponse::BuiltinError(
                            entry.status.as_u16(),
                            Some(entry.headers.clone()),
                        )
                    } else {
                        HttpResponse::Custom(serve(
                            entry,
                            ServedState::Hit,
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
                            report(
                                ctx,
                                CacheOutcome {
                                    result: "hit",
                                    zone_id: &zone_id,
                                    key: &base_key,
                                    scope: Some(scope),
                                    items: Some(retry_items),
                                    stored: None,
                                    evictions: Some(retry_stats),
                                    detail: None,
                                    key_uri: None,
                                    key_method: None,
                                    bypass_reason: None,
                                    evaluated_cookies: None,
                                    coalesced_wait_ms: Some(wait_ms),
                                    mark_uncoalesced: false,
                                    metric_result: None,
                                },
                            );
                            ctx.res = Some(if entry.body.is_none() {
                                HttpResponse::BuiltinError(
                                    entry.status.as_u16(),
                                    Some(entry.headers.clone()),
                                )
                            } else {
                                HttpResponse::Custom(serve(
                                    entry,
                                    ServedState::Hit,
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

        if request_policy.only_if_cached && !matches!(lookup_result, LookupResult::Hit) {
            report(
                ctx,
                CacheOutcome {
                    result: "bypass",
                    zone_id: &zone_id,
                    key: &base_key,
                    scope: None,
                    items: None,
                    stored: None,
                    evictions: None,
                    detail: Some("only-if-cached"),
                    key_uri: None,
                    key_method: None,
                    bypass_reason: Some("request-only-if-cached"),
                    evaluated_cookies: None,
                    coalesced_wait_ms: None,
                    mark_uncoalesced: true,
                    metric_result: None,
                },
            );
            let mut headers = HeaderMap::new();
            if let Ok(value) = http::header::HeaderValue::from_str(
                "FerronCache; fwd=bypass; detail=request-only-if-cached",
            ) {
                headers.insert(CACHE_STATUS_HEADER, value);
            }
            ctx.res = Some(HttpResponse::BuiltinError(
                StatusCode::GATEWAY_TIMEOUT.as_u16(),
                Some(headers),
            ));
            return Ok(false);
        }

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
            let bypass_reason = match lookup_result {
                LookupResult::Bypass => Some(request_policy.reason),
                _ => None,
            };
            report(
                ctx,
                CacheOutcome {
                    result: result_label,
                    zone_id: &zone_id,
                    key: &base_key,
                    scope: None,
                    items: None,
                    stored: None,
                    evictions: None,
                    detail: None,
                    key_uri: req_uri.as_deref(),
                    key_method: req_method.as_deref(),
                    bypass_reason,
                    evaluated_cookies: None,
                    coalesced_wait_ms: None,
                    mark_uncoalesced: true,
                    metric_result: None,
                },
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
                        HttpResponse::Custom(serve(
                            (**entry).clone(),
                            ServedState::StaleWhileRevalidate,
                            state.head_only,
                            state.config.emit_litespeed_headers,
                        )?)
                    });

                    report(
                        ctx,
                        CacheOutcome {
                            result: "stale",
                            zone_id: &state.zone_id,
                            key: &state.base_key,
                            scope: *scope,
                            items: Some(*items),
                            stored: None,
                            evictions: None,
                            detail: None,
                            key_uri: None,
                            key_method: None,
                            bypass_reason: None,
                            evaluated_cookies: None,
                            coalesced_wait_ms: None,
                            mark_uncoalesced: false,
                            metric_result: Some("hit"),
                        },
                    );
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
                    merge_revalidation_headers(&mut new_fresh_headers, fresh_headers);
                    fresh_headers = new_fresh_headers;
                }

                let mut entry = (**cached_entry).clone();
                entry.headers = fresh_headers;
                let response_200 = serve(
                    entry,
                    ServedState::Revalidated,
                    state.head_only,
                    state.config.emit_litespeed_headers,
                )?;

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

                        report(
                            ctx,
                            CacheOutcome {
                                result: "stale",
                                zone_id: &state.zone_id,
                                key: &state.base_key,
                                scope: Some(stale_entry.scope),
                                items: Some(state.store.len()),
                                stored: None,
                                evictions: None,
                                detail: None,
                                key_uri: None,
                                key_method: None,
                                bypass_reason: None,
                                evaluated_cookies: None,
                                coalesced_wait_ms: None,
                                mark_uncoalesced: false,
                                metric_result: Some("hit"),
                            },
                        );

                        ctx.res = Some(HttpResponse::Custom(stale_response));
                        return Ok(());
                    }
                }
            }
        }

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
        let purge_scope = if purge_ops.is_empty() {
            None
        } else {
            purge(
                ctx,
                &state.zone_id,
                &state.store,
                &purge_ops,
                state.private_key.as_deref(),
                entry_host(&ctx.hostname, &state.zone_id).as_deref(),
                true,
                &state.config.purge_propagation,
            );
            purge_ops.last().map(|operation| operation.scope)
        };

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
                no_cache_field_names: Vec::new(),
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
                    crate::policy::strip_no_cache_fields(
                        &mut stored_headers,
                        &decision.no_cache_field_names,
                    );
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
                        purge_host: entry_host(&ctx.hostname, &state.zone_id).unwrap_or_default(),
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

                    if let LookupResult::StaleWhileRevalidate {
                        entry,
                        scope: stale_scope,
                        items: stale_items,
                        ..
                    } = &state.lookup_result
                    {
                        ctx.res = Some(if entry.body.is_none() {
                            HttpResponse::BuiltinError(
                                entry.status.as_u16(),
                                Some(entry.headers.clone()),
                            )
                        } else {
                            HttpResponse::Custom(serve(
                                (**entry).clone(),
                                ServedState::StaleIfError,
                                state.head_only,
                                state.config.emit_litespeed_headers,
                            )?)
                        });

                        emit_eviction_metrics(ctx, &state.zone_id, stats);
                        emit_store_metric(ctx, &state.zone_id, scope, status.as_u16());
                        emit_request_metric(ctx, &state.zone_id, "hit", *stale_scope, *stale_items);
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
                    report(
                        ctx,
                        CacheOutcome {
                            result: "miss",
                            zone_id: &state.zone_id,
                            key: &state.base_key,
                            scope: Some(scope),
                            items: Some(items),
                            stored: Some((scope, status.as_u16())),
                            evictions: Some(stats),
                            detail: None,
                            key_uri: None,
                            key_method: None,
                            bypass_reason: None,
                            evaluated_cookies: vary_rule.as_ref().and_then(|vr| {
                                (!vr.cookie_names.is_empty()).then_some(vr.cookie_names.as_slice())
                            }),
                            coalesced_wait_ms: None,
                            mark_uncoalesced: false,
                            metric_result: None,
                        },
                    );
                    ctx.res = Some(outgoing_response);
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
                    report(
                        ctx,
                        CacheOutcome {
                            result: "miss",
                            zone_id: &state.zone_id,
                            key: &state.base_key,
                            scope: None,
                            items: Some(state.store.len()),
                            stored: None,
                            evictions: None,
                            detail: Some("response-too-large"),
                            key_uri: None,
                            key_method: None,
                            bypass_reason: None,
                            evaluated_cookies: None,
                            coalesced_wait_ms: None,
                            mark_uncoalesced: false,
                            metric_result: None,
                        },
                    );
                    ctx.res = Some(HttpResponse::Custom(response));
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
            report(
                ctx,
                CacheOutcome {
                    result,
                    zone_id: &state.zone_id,
                    key: &state.base_key,
                    scope: purge_scope.or(decision.scope),
                    items: Some(state.store.len()),
                    stored: None,
                    evictions: None,
                    detail: (result == "bypass").then_some(decision.reason),
                    key_uri: None,
                    key_method: None,
                    bypass_reason: None,
                    evaluated_cookies: None,
                    coalesced_wait_ms: None,
                    mark_uncoalesced: false,
                    metric_result: None,
                },
            );
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

/// Current configuration generation, bumped on every config reload.
#[inline]
fn active_config_generation() -> u64 {
    ferron_core::admin::ADMIN_METRICS
        .reload_metrics
        .read()
        .active_generation
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

/// The host associated with a cache entry or purge request.
///
/// Prefers the resolved vhost. When the request is host-ambiguous, falls back
/// to the zone's own host for per-host zones so a host guard still applies;
/// shared named/global zones without a host resolve to an empty value, which
/// never matches a populated host.
fn entry_host(hostname: &Option<String>, zone_id: &CacheZoneId) -> Option<String> {
    hostname.clone().or_else(|| match zone_id {
        CacheZoneId::Host(host) => Some(host.clone()),
        CacheZoneId::Named(_) | CacheZoneId::Global => None,
    })
}

/// Whether a `X-Purge-Source: propagation` purge proves knowledge of the
/// configured shared secret.
///
/// The secret is required: when none is configured, a propagation claim is
/// indistinguishable from a replay and is rejected. Comparison is
/// constant-time.
#[inline]
fn propagation_secret_verified(request_headers: &HeaderMap, shared_secret: Option<&str>) -> bool {
    use subtle::ConstantTimeEq;

    let Some(configured) = shared_secret else {
        return false;
    };
    let Some(received) = request_headers
        .get(&PURGE_SECRET_HEADER)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    configured.as_bytes().ct_eq(received.as_bytes()).into()
}

/// Whether a `PURGE` request is authorized to invalidate this cache.
///
/// An allow-listed client IP always authorizes a purge. Otherwise the request
/// must carry an authenticated user (`ctx.auth_user`) **and** a `basic_auth`
/// block must be in scope for the request. Requiring the in-scope `basic_auth`
/// block prevents a user authenticated by a foreign host's `basic_auth` from
/// purging a host that does not own those credentials.
#[inline]
fn purge_allowed(
    remote_ip: IpAddr,
    purge_allowed_ips: &[IpCidr],
    has_basic_auth_in_scope: bool,
    auth_user: Option<&str>,
) -> bool {
    if purge_allowed_ips
        .iter()
        .any(|cidr| cidr.contains(&remote_ip))
    {
        return true;
    }
    auth_user.is_some() && has_basic_auth_in_scope
}
