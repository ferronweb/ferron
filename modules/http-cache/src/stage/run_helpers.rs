use bytes::Bytes;
use ferron_observability::{Event, LogEvent, LogLevel};
use http::header;
use http::{HeaderMap, Method, Response, StatusCode};
use http_body_util::{BodyExt, Full};
use std::sync::Arc;

use ferron_core::pipeline::PipelineError;
use ferron_http::{HttpContext, HttpResponse};

use crate::lscache::{
    collect_lsc_cookies, parse_litespeed_cache_control, parse_litespeed_purge,
    parse_litespeed_tags, parse_litespeed_vary, PurgeOperation, PurgeSelector,
};
use crate::policy::{
    evaluate_response_policy, parse_request_policy, satisfies_freshness_constraints, CacheScope,
    RequestCachePolicy,
};
use crate::store::{merge_revalidation_headers, LookupOutcome, StoredEntry};

use super::helpers::{
    client_conditionals_indicate_not_modified, entry_host, propagation_secret_verified,
    purge_allowed, resolve_zone_id,
};
use super::key::{
    build_base_key, build_private_cache_key, build_vary_rule, parse_cookies_filtered,
};
use super::outcome::{
    emit_eviction_metrics, emit_request_metric, emit_singleflight_metrics, emit_store_metric,
    report, CacheOutcome,
};
use super::purge::{purge, PURGE_SOURCE_HEADER};
use super::response_helpers::{
    annotate_response_headers, append_lsc_cookies_as_set_cookie, collect_body_with_limit,
    response_from_parts, response_from_streaming_parts, strip_internal_headers, CacheHeaderState,
    CollectBodyOutcome, CACHE_STATUS_HEADER,
};
use super::served::{serve, serve_not_modified, ServedState};
use super::{HttpCacheStage, InflightGuard, LookupResult, RequestState, RequestStateKey};

const LOG_TARGET: &str = "ferron-http-cache";

/// Forward pass of the cache pipeline stage.
#[inline]
pub(super) async fn run_forward(
    stage: &HttpCacheStage,
    ctx: &mut HttpContext,
) -> Result<bool, PipelineError> {
    let config = stage.get_config(ctx);

    if !config.enabled {
        return Ok(true);
    }

    let zone_id = resolve_zone_id(&ctx.hostname, &config, &ctx.configuration);
    let store = stage.get_or_create_zone(&zone_id, &ctx.configuration).await;

    let Some(request) = ctx.req.as_ref() else {
        return Ok(true);
    };

    let headers_ref = request.headers();
    // Only parse cookies that could affect the cache key (vary cookies or
    // private-session cookies). For public, non-varying assets this avoids
    // allocating a map for every cookie in the request.
    let request_cookies = parse_cookies_filtered(headers_ref, &config.vary_cookies);
    let request_policy = if config.ignore_request_cache_control {
        RequestCachePolicy {
            allow_lookup: true,
            allow_store: true,
            reason: "eligible",
            ..Default::default()
        }
    } else {
        parse_request_policy(headers_ref)
    };
    let has_authorization = headers_ref.contains_key(header::AUTHORIZATION);
    let purge_url = request
        .uri()
        .path_and_query()
        .map(|value| value.as_str().to_string())
        .unwrap_or_else(|| request.uri().path().to_string());
    let base_key = build_base_key(
        ctx.encrypted,
        headers_ref,
        ctx.original_uri.as_ref(),
        request.uri(),
        ctx.hostname.as_deref(),
    );
    let private_key = build_private_cache_key(
        &request_cookies,
        ctx.auth_user.as_deref(),
        &config.vary_cookies,
    );
    let head_only = request.method() == Method::HEAD;

    let method_cacheable = matches!(request.method(), &Method::GET | &Method::HEAD);
    let method_purge = request.method() == "PURGE";

    if method_purge {
        if !config.purge_method {
            // PURGE not enabled: fall through to 405 from downstream stages
        } else {
            let is_propagated = headers_ref
                .get(&PURGE_SOURCE_HEADER)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.eq_ignore_ascii_case("propagation"));

            // A propagation claim must prove knowledge of the shared
            // secret. Without this, any client could tag a PURGE as
            // propagated and bypass the normal authorization (B#9).
            if is_propagated
                && !propagation_secret_verified(
                    headers_ref,
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
                // Without a client IP (e.g. Unix socket listeners) a non-propagated
                // purge cannot be allow-listed; deny it.
                let purge_allowed = ctx
                    .remote_address
                    .map(|a| {
                        purge_allowed(
                            a.ip().to_canonical(),
                            &config.purge_allowed_ips,
                            has_basic_auth_in_scope,
                            ctx.auth_user.as_deref(),
                        )
                    })
                    .unwrap_or(false);

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
        && !headers_ref.contains_key(header::RANGE)
        && !headers_ref.contains_key(header::UPGRADE)
        && request_policy.allow_lookup;

    let lookup_result = if request_is_lookup_eligible {
        let LookupOutcome {
            entry: lookup,
            stats,
            items,
            had_expired,
        } = store.lookup(
            &base_key,
            headers_ref,
            &request_cookies,
            private_key.as_deref(),
        );
        if let Some((entry, cache_key, hit_kind)) = lookup {
            let scope = entry.scope;

            if request_policy.reason == "request-revalidation"
                || !satisfies_freshness_constraints(&request_policy, entry.age, entry.ttl)
            {
                LookupResult::Revalidate {
                    entry: Box::new(entry),
                    cache_key,
                    stats,
                }
            } else if let crate::store::LookupHit::StaleWhileRevalidate = hit_kind {
                if config.enable_stale_while_revalidate {
                    let (is_leader, _notify) = store.begin_fetch(&cache_key);

                    LookupResult::StaleWhileRevalidate {
                        entry: Box::new(entry),
                        stats,
                        inflight_key: is_leader.then_some(cache_key.clone()),
                        scope: Some(scope),
                        items,
                    }
                } else {
                    LookupResult::Revalidate {
                        entry: Box::new(entry),
                        cache_key,
                        stats,
                    }
                }
            } else {
                let client_conditionals_match = client_conditionals_indicate_not_modified(
                    request.method(),
                    headers_ref,
                    entry.etag.as_ref(),
                    entry.last_modified.as_ref(),
                );
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
                ctx.res = Some(if client_conditionals_match {
                    HttpResponse::Custom(serve_not_modified(entry, config.emit_litespeed_headers)?)
                } else if entry.body.is_none() {
                    HttpResponse::BuiltinError(
                        entry.status.as_u16(),
                        Some((*entry.headers).clone()),
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
        } else {
            if had_expired {
                let coalesce_start = std::time::Instant::now();
                let coalesce_key = store
                    .primary_candidate_key(
                        &base_key,
                        headers_ref,
                        &request_cookies,
                        private_key.as_deref(),
                    )
                    .unwrap_or_else(|| base_key.clone());
                let (is_leader, notify) = store.begin_fetch(&coalesce_key);

                if !is_leader {
                    let coalesced =
                        tokio::time::timeout(config.coalesce_timeout, notify.notified())
                            .await
                            .is_ok();
                    let wait_ms = coalesce_start.elapsed().as_secs_f64() * 1000.0;
                    emit_singleflight_metrics(ctx, &store);

                    let LookupOutcome {
                        entry: retry_lookup,
                        stats: retry_stats,
                        items: retry_items,
                        ..
                    } = if coalesced {
                        store.lookup(
                            &base_key,
                            headers_ref,
                            &request_cookies,
                            private_key.as_deref(),
                        )
                    } else {
                        // Leader never completed. Stop coalescing and treat
                        // this as a normal miss that fetches from upstream.
                        LookupOutcome {
                            entry: None,
                            stats,
                            items: 0,
                            had_expired: false,
                        }
                    };
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
                                Some((*entry.headers).clone()),
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
                        inflight_key: Some(coalesce_key.clone()),
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

    let _ = request;
    let _ = headers_ref;
    if let LookupResult::Revalidate { ref entry, .. } = lookup_result {
        if entry.status != http::StatusCode::NOT_MODIFIED {
            // Don't add caching headers if status is 304, otherwise browsers won't load a page!
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
    }

    if !stop {
        let result_label = match &lookup_result {
            LookupResult::Hit => "hit",
            LookupResult::StaleWhileRevalidate { inflight_key, .. } if inflight_key.is_some() => {
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
        request_headers: ctx
            .req
            .as_ref()
            .expect("request state is invalid at this point")
            .headers()
            .clone(),
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

/// Inverse pass of the cache pipeline stage.
#[inline]
pub(super) async fn run_inverse_handler(
    _stage: &HttpCacheStage,
    ctx: &mut HttpContext,
) -> Result<(), PipelineError> {
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
            // The leader revalidates with the upstream; the store path
            // below emits the canonical eviction/store/request metrics
            // once. The follower serves the stale entry immediately.
            if inflight_key.is_none() {
                emit_eviction_metrics(ctx, &state.zone_id, *stats);
                ctx.res = Some(if entry.body.is_none() {
                    HttpResponse::BuiltinError(
                        entry.status.as_u16(),
                        Some((*entry.headers).clone()),
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
            *response.status_mut() =
                StatusCode::from_u16(status).map_err(|e| PipelineError::custom(e.to_string()))?;
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
                let mut new_fresh_headers = (*cached_entry.headers).clone();
                merge_revalidation_headers(&mut new_fresh_headers, fresh_headers);
                fresh_headers = new_fresh_headers;
            }

            let mut entry = (**cached_entry).clone();
            entry.headers = Arc::new(fresh_headers);
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
                if !stale_entry.must_revalidate && stale_entry.age <= stale_entry.ttl + sie_duration
                {
                    let stale_response = serve(
                        stale_entry.clone(),
                        ServedState::StaleIfError,
                        state.head_only,
                        state.config.emit_litespeed_headers,
                    )?;

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
            message: "Ignoring unsupported LSCache stale purge marker and performing a hard purge"
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
            summary: "Skipping cache store because X-LiteSpeed-Vary is not supported yet".into(),
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
    // Private scope must never be keyed on the client IP alone: behind
    // CGNAT one user's private response would be served to another user on
    // the same public address. Without an identifying component the
    // response cannot be partitioned per client, so serve it uncached.
    let decision = if decision.store
        && decision.scope == Some(CacheScope::Private)
        && state.private_key.is_none()
    {
        crate::policy::ResponseCacheDecision {
            store: false,
            scope: None,
            ttl: None,
            stale_while_revalidate: None,
            stale_if_error: None,
            must_revalidate: false,
            no_cache_field_names: Vec::new(),
            reason: "private-no-identity",
        }
    } else {
        decision
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
                    headers: Arc::new(stored_headers),
                    body: body_bytes,
                    lsc_cookies: Arc::new(lsc_cookies.clone()),
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
                    scope: stale_scope,
                    items: stale_items,
                    ..
                } = &state.lookup_result
                {
                    // This request is the SWR leader: it revalidated the
                    // entry with the upstream and stored the fresh
                    // response. Serve the fresh response to the leader
                    // instead of the stale entry it triggered the
                    // revalidation for.
                    annotate_response_headers(
                        match &mut outgoing_response {
                            HttpResponse::Custom(r) => r.headers_mut(),
                            HttpResponse::BuiltinError(_, Some(h)) => h,
                            _ => unreachable!(),
                        },
                        CacheHeaderState::Revalidated,
                        state.config.emit_litespeed_headers,
                    );
                    ctx.res = Some(outgoing_response);

                    emit_eviction_metrics(ctx, &state.zone_id, stats);
                    emit_store_metric(ctx, &state.zone_id, scope, status.as_u16());
                    emit_request_metric(
                        ctx,
                        &state.zone_id,
                        "revalidated",
                        *stale_scope,
                        *stale_items,
                    );
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
