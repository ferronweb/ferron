//! Core proxy logic: request transformation, TLS, connection establishment, and forwarding.

mod affinity;
mod connect;
mod pool;
mod request;
mod response;
mod tls;

use std::collections::HashMap;
use std::sync::Arc;

use ferron_http::{HttpContext, HttpResponse};
use ferron_observability::{Event, LogAttributeValue, LogEvent, LogLevel};
use http::StatusCode;
use parking_lot::RwLock;

use crate::config::ProxyConfig;
use crate::connections::ConnectionManager;
use crate::proxy::affinity::extract_affinity_key;
use crate::types::circuit::CircuitBreakerStateMap;
use crate::types::error::ProxyError;
use crate::types::flapping::FlappingStateMap;
use crate::types::health::HealthCheckStateMap;
use crate::types::retry_budget::SharedRetryBudget;
use crate::types::upstream::UpstreamInner;
use crate::types::ConnectionsTrackState;
use crate::upstream::circuit::CircuitBreaker;
use crate::upstream::lb::{ConsistentHashRing, EwmaStateMap, LoadBalancerAlgorithmInner};
use crate::upstream::{record_backend_response, record_backend_transport_failure, BackendSet};
use crate::ProxyMetrics;

use self::affinity::maybe_set_affinity_cookie;
use self::tls::cached_tls_config;

const LOG_TARGET: &str = "ferron-http-proxy";

/// Categorize an HTTP method into a bounded set for metric labels.
///
/// Standard methods are kept as-is; unknown methods are collapsed into `_other`
/// to prevent high-cardinality label explosion from custom/fuzzed HTTP methods.
#[inline]
pub(crate) fn categorize_http_method(method: &http::Method) -> &'static str {
    match *method {
        http::Method::GET => "GET",
        http::Method::HEAD => "HEAD",
        http::Method::POST => "POST",
        http::Method::PUT => "PUT",
        http::Method::DELETE => "DELETE",
        http::Method::CONNECT => "CONNECT",
        http::Method::OPTIONS => "OPTIONS",
        http::Method::TRACE => "TRACE",
        http::Method::PATCH => "PATCH",
        _ => "_other",
    }
}

/// Main proxy execution.
///
/// Returns the HTTP response and collected metrics for post-request emission.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::type_complexity)]
#[inline]
pub async fn execute_proxy(
    ctx: &mut HttpContext,
    config: &ProxyConfig,
    cm: &ConnectionManager,
    circuit_breaker_state: CircuitBreakerStateMap,
    flapping_state: FlappingStateMap,
    algorithm: &LoadBalancerAlgorithmInner,
    ring: &RwLock<ConsistentHashRing>,
    conn_state: Option<&ConnectionsTrackState>,
    ewma_state: Option<&EwmaStateMap>,
    health_check_state: Option<&HealthCheckStateMap>,
    active_unhealthy_counter: Option<&RwLock<HashMap<String, u64>>>,
    upstreams: Vec<Arc<UpstreamInner>>,
    retry_budget: Option<&SharedRetryBudget>,
) -> Result<(HttpResponse, ProxyMetrics), ProxyError> {
    let mut metrics = ProxyMetrics::new();

    if upstreams.is_empty() {
        ctx.events.emit(Event::Log(LogEvent {
            level: LogLevel::Error,
            message: "Reverse proxy: no healthy upstream backends available".to_string(),
            summary: "Reverse proxy: no healthy upstream backends available".into(),
            target: LOG_TARGET,
            attributes: Vec::new(),
            trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
        }));
        if let Some(counter) = active_unhealthy_counter {
            let guard = counter.read();
            metrics.active_unhealthy_backends =
                guard.iter().map(|(k, v)| (k.clone(), *v)).collect();
        }
        return Ok((HttpResponse::BuiltinError(502, None), metrics));
    }

    let affinity_key = extract_affinity_key(&config.affinity, ctx);

    let event_sink = ctx.events.clone();
    let mut backend_set = BackendSet::new(
        &upstreams,
        algorithm,
        conn_state,
        ewma_state,
        health_check_state,
        CircuitBreaker::new(
            Some(&circuit_breaker_state),
            Some(&flapping_state),
            &config.circuit_breaker,
            &event_sink,
            ferron_http::trace_context::current_event_trace_context(ctx),
            config.metrics_resolved_ip,
        ),
        config.affinity.as_ref().map(|t| &t.affinity_type),
        affinity_key.as_deref(),
        ring,
    );

    // Backend selection loop, retries on connection failure when retry_connection is enabled
    loop {
        // Select upstream via load balancing (tracker already initialized inside)
        let Some(selected) = backend_set.next_backend() else {
            ctx.events.emit(Event::Log(LogEvent {
                level: LogLevel::Error,
                message: "Reverse proxy: all upstream backends are unhealthy".to_string(),
                summary: "Reverse proxy: all upstream backends are unhealthy".into(),
                target: LOG_TARGET,
                attributes: Vec::new(),
                trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
            }));
            if let Some(counter) = active_unhealthy_counter {
                let guard = counter.read();
                metrics.active_unhealthy_backends =
                    guard.iter().map(|(k, v)| (k.clone(), *v)).collect();
            }
            let exclusions = backend_set.take_exclusions();
            metrics
                .excluded_already_tried
                .extend(exclusions.already_tried);
            metrics
                .excluded_circuit_open
                .extend(exclusions.circuit_open);
            metrics.excluded_overloaded.extend(exclusions.overloaded);
            return Ok((HttpResponse::BuiltinError(503, None), metrics));
        };

        metrics.selected_backends.insert(selected.upstream.clone());
        metrics.final_selected_backend = Some(selected.upstream.clone());
        metrics.candidate_scores = selected.candidate_scores;
        metrics
            .excluded_already_tried
            .extend(selected.exclusions.already_tried);
        metrics
            .excluded_circuit_open
            .extend(selected.exclusions.circuit_open);
        metrics
            .excluded_overloaded
            .extend(selected.exclusions.overloaded);

        let proxy_request_url: http::Uri = selected
            .upstream
            .proxy_to
            .parse()
            .map_err(|_| ProxyError::InvalidUpstreamUrl(selected.upstream.proxy_to.clone()))?;
        let is_https = proxy_request_url.scheme_str() == Some("https");
        let client_ip = config.proxy_header.map(|_| ctx.remote_address.ip());
        let local_limit = cm.get_local_limit(selected.upstream.clone());
        let idle_timeout = selected.upstream.idle_timeout;

        // Same-upstream retry loop: retry the selected backend on transport failure
        // before falling back to another backend via retry_connection.
        let mut same_upstream_attempt: u32 = 0;
        loop {
            let tracker_for_attempt = selected.tracker.clone();
            match pool::try_send_with_pool(
                ctx,
                config,
                cm,
                selected.upstream.clone(),
                &proxy_request_url,
                client_ip,
                local_limit,
                idle_timeout,
                is_https,
                conn_state,
                tracker_for_attempt,
                &mut metrics,
            )
            .await
            {
                Ok(resp) => {
                    if let Some(status) = metrics.status_code {
                        record_backend_response(
                            Some(&circuit_breaker_state),
                            Some(&flapping_state),
                            &config.circuit_breaker,
                            &selected.upstream,
                            status,
                            Some(metrics.upstream_time_secs),
                            &mut metrics,
                            &ctx.events,
                            ferron_http::trace_context::current_event_trace_context(ctx),
                            config.metrics_resolved_ip,
                        );
                    }

                    if metrics.upstream_time_secs > 0.0
                        && matches!(algorithm, LoadBalancerAlgorithmInner::P2cEwma)
                    {
                        if let Some(ewma_state) = ewma_state {
                            crate::upstream::lb::p2c_ewma::update_ewma(
                                ewma_state,
                                &selected.upstream,
                                metrics.upstream_time_secs,
                                &Default::default(),
                            );
                        }
                    }

                    if let Some(counter) = active_unhealthy_counter {
                        let guard = counter.read();
                        metrics.active_unhealthy_backends =
                            guard.iter().map(|(k, v)| (k.clone(), *v)).collect();
                    }

                    let resp = maybe_set_affinity_cookie(
                        resp,
                        &config.affinity,
                        affinity_key.map(|k| String::from_utf8_lossy(&k).to_string()),
                    );

                    if let Some(budget) = retry_budget {
                        budget.record_request();
                    }

                    return Ok((resp, metrics));
                }
                Err(e) => {
                    record_backend_transport_failure(
                        Some(&circuit_breaker_state),
                        Some(&flapping_state),
                        &config.circuit_breaker,
                        &selected.upstream,
                        &mut metrics,
                        &ctx.events,
                        ferron_http::trace_context::current_event_trace_context(ctx),
                        config.metrics_resolved_ip,
                    );

                    // First, try to retry the same upstream on intermittent failures.
                    // Only idempotent/replayable requests (ctx.req still present after
                    // body recycle in send_via_wrapper) are retried.
                    let can_retry_same = same_upstream_attempt < config.max_retries_per_upstream
                        && ctx.req.is_some();
                    if can_retry_same {
                        if let Some(budget) = retry_budget {
                            if !budget.try_consume_retry_token() {
                                metrics.retry_budget_exhausted = true;
                                ctx.events.emit(Event::Log(LogEvent {
                                    level: LogLevel::Warn,
                                    message: format!(
                                        "Reverse proxy: retry budget exhausted — upstream: {url}: {err}",
                                        url = selected.upstream.proxy_to,
                                        err = e
                                    ),
                                    summary: "Reverse proxy: retry budget exhausted".into(),
                                    target: LOG_TARGET,
                                    attributes: vec![(
                                        "upstream.address",
                                        LogAttributeValue::String(selected.upstream.proxy_to.clone()),
                                    ), (
                                        "error.message",
                                        LogAttributeValue::String(e.to_string()),
                                    )],
                                    trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
                                }));
                                if let Some(counter) = active_unhealthy_counter {
                                    let guard = counter.read();
                                    metrics.active_unhealthy_backends =
                                        guard.iter().map(|(k, v)| (k.clone(), *v)).collect();
                                }
                                let retry_after_secs = budget.time_until_available(1);
                                let retry_after_value =
                                    retry_after_secs.ceil().clamp(1.0, 3600.0) as u64;
                                let mut headers = http::HeaderMap::new();
                                headers.insert(
                                    http::header::RETRY_AFTER,
                                    http::HeaderValue::from_str(&retry_after_value.to_string())
                                        .expect("retry-after value should be valid"),
                                );
                                return Ok((
                                    HttpResponse::BuiltinError(503, Some(headers)),
                                    metrics,
                                ));
                            }
                            budget.record_retry();
                        }
                        same_upstream_attempt += 1;
                        metrics.same_upstream_retry_count += 1;
                        metrics.retry_count += 1;
                        ctx.events.emit(Event::Log(LogEvent {
                            level: LogLevel::Warn,
                            message: format!(
                                "Reverse proxy: backend failed, retrying same upstream ({}/{}) — upstream: {url}: {err}",
                                same_upstream_attempt,
                                config.max_retries_per_upstream,
                                url = selected.upstream.proxy_to,
                                err = e
                            ),
                            summary: "Reverse proxy: retrying same upstream".into(),
                            target: LOG_TARGET,
                            attributes: vec![(
                                "upstream.address",
                                LogAttributeValue::String(selected.upstream.proxy_to.clone()),
                            ), (
                                "error.message",
                                LogAttributeValue::String(e.to_string())
                            )],
                            trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
                        }));
                        continue; // retry same upstream
                    }

                    // Same-upstream retries exhausted, try another backend if enabled.
                    if config.retry_connection {
                        if let Some(budget) = retry_budget {
                            if !budget.try_consume_retry_token() {
                                metrics.retry_budget_exhausted = true;
                                ctx.events.emit(Event::Log(LogEvent {
                                    level: LogLevel::Warn,
                                    message: format!(
                                        "Reverse proxy: retry budget exhausted — upstream: {url}: {err}",
                                        url = selected.upstream.proxy_to,
                                        err = e
                                    ),
                                    summary: "Reverse proxy: retry budget exhausted".into(),
                                    target: LOG_TARGET,
                                    attributes: vec![(
                                        "upstream.address",
                                        LogAttributeValue::String(selected.upstream.proxy_to.clone()),
                                    ), (
                                        "error.message",
                                        LogAttributeValue::String(e.to_string()),
                                    )],
                                    trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
                                }));
                                if let Some(counter) = active_unhealthy_counter {
                                    let guard = counter.read();
                                    metrics.active_unhealthy_backends =
                                        guard.iter().map(|(k, v)| (k.clone(), *v)).collect();
                                }
                                let retry_after_secs = budget.time_until_available(1);
                                let retry_after_value =
                                    retry_after_secs.ceil().clamp(1.0, 3600.0) as u64;
                                let mut headers = http::HeaderMap::new();
                                headers.insert(
                                    http::header::RETRY_AFTER,
                                    http::HeaderValue::from_str(&retry_after_value.to_string())
                                        .expect("retry-after value should be valid"),
                                );
                                return Ok((
                                    HttpResponse::BuiltinError(503, Some(headers)),
                                    metrics,
                                ));
                            }
                            budget.record_retry();
                        }
                        let healthy_count = backend_set.available_count();
                        if healthy_count > 0
                            && metrics.selected_backends.len() < upstreams.len()
                            && ctx.req.is_some()
                        {
                            metrics.retry_count += 1;
                            ctx.events.emit(Event::Log(LogEvent {
                                level: LogLevel::Warn,
                                message: format!(
                                    "Reverse proxy: backend failed, retrying with another — upstream: {url}: {err}",
                                    url = selected.upstream.proxy_to,
                                    err = e
                                ),
                                summary: "Reverse proxy: retrying with another backend".into(),
                                target: LOG_TARGET,
                                attributes: vec![(
                                    "upstream.address",
                                    LogAttributeValue::String(selected.upstream.proxy_to.clone()),
                                ), (
                                    "error.message",
                                    LogAttributeValue::String(e.to_string())
                                )],
                                trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
                            }));
                            break; // outer select next backend
                        }
                    }

                    // No retry or no more backends...
                    let status = e.http_status_hint().unwrap_or(StatusCode::BAD_GATEWAY);
                    let reason = match status {
                        StatusCode::SERVICE_UNAVAILABLE => "Service unavailable",
                        StatusCode::GATEWAY_TIMEOUT => "Gateway timeout",
                        _ => "Bad gateway",
                    };
                    let attrs = vec![
                        (
                            "upstream.address",
                            LogAttributeValue::String(selected.upstream.proxy_to.clone()),
                        ),
                        (
                            "http.response.status_code",
                            LogAttributeValue::I64(status.as_u16() as i64),
                        ),
                        (
                            "error.type",
                            LogAttributeValue::String(e.error_type().to_string()),
                        ),
                        ("error.message", LogAttributeValue::String(e.to_string())),
                    ];
                    ctx.events.emit(Event::Log(LogEvent {
                        level: LogLevel::Error,
                        message: format!(
                            "Reverse proxy: {reason} — upstream: {url}: {err}",
                            url = selected.upstream.proxy_to,
                            err = e
                        ),
                        summary: e.summary().into(),
                        target: LOG_TARGET,
                        attributes: attrs,
                        trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
                    }));
                    if let Some(counter) = active_unhealthy_counter {
                        let guard = counter.read();
                        metrics.active_unhealthy_backends =
                            guard.iter().map(|(k, v)| (k.clone(), *v)).collect();
                    }
                    return Ok((HttpResponse::BuiltinError(status.as_u16(), None), metrics));
                }
            }
        }
    }
}
