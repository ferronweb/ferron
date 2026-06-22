//! Core proxy logic: request transformation, TLS, connection establishment, and forwarding.

mod affinity;
mod backend;
mod connect;
mod pool;
mod request;
mod response;
mod tls;

use std::collections::HashMap;
use std::time::Duration;

use ferron_http::{HttpContext, HttpResponse};
use ferron_observability::{Event, LogAttributeValue, LogEvent, LogLevel};
use http::StatusCode;
use parking_lot::RwLock;

use crate::config::ProxyConfig;
use crate::connections::ConnectionManager;
use crate::proxy::affinity::extract_affinity_key;
use crate::types::circuit::CircuitBreakerStateMap;
use crate::types::error::ProxyError;
use crate::types::health::HealthCheckStateMap;
use crate::types::upstream::UpstreamInner;
use crate::types::ConnectionsTrackState;
use crate::upstream::lb::{ConsistentHashRing, EwmaStateMap, LoadBalancerAlgorithmInner};
use crate::upstream::{
    determine_proxy_to, record_backend_response, record_backend_transport_failure,
    resolve_upstreams,
};
use crate::ProxyMetrics;

use self::affinity::maybe_set_affinity_cookie;
use self::backend::count_available_backends;
use self::tls::{cached_tls_config, io_error_status};

const LOG_TARGET: &str = "ferron-http-proxy";

#[inline]
fn idle_timeout_for_upstream(config: &ProxyConfig, upstream: &UpstreamInner) -> Duration {
    config
        .idle_timeout_map
        .get(&upstream.proxy_to)
        .copied()
        .unwrap_or(Duration::from_secs(60))
}

/// Main proxy execution.
///
/// Returns the HTTP response and collected metrics for post-request emission.
#[allow(clippy::too_many_arguments)]
pub async fn execute_proxy(
    ctx: &mut HttpContext,
    config: &ProxyConfig,
    cm: &ConnectionManager,
    circuit_breaker_state: CircuitBreakerStateMap,
    algorithm: &LoadBalancerAlgorithmInner,
    ring: &RwLock<ConsistentHashRing>,
    conn_state: Option<&ConnectionsTrackState>,
    ewma_state: Option<&EwmaStateMap>,
    health_check_state: Option<&HealthCheckStateMap>,
    active_unhealthy_counter: Option<&RwLock<HashMap<String, u64>>>,
) -> Result<(HttpResponse, ProxyMetrics), ProxyError> {
    let mut metrics = ProxyMetrics::new();

    // Resolve upstreams (SRV records are resolved here, static ones pass through)
    let upstreams = resolve_upstreams(
        &config.upstreams,
        health_check_state.cloned(),
    )
    .await;

    if upstreams.is_empty() {
        ctx.events.emit(Event::Log(LogEvent {
            level: LogLevel::Error,
            message: "Reverse proxy: no healthy upstream backends available".to_string(),
            summary: "Reverse proxy: no healthy upstream backends available".into(),
            target: LOG_TARGET,
            attributes: Vec::new(),
            trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
        }));
        // Collect active health check unhealthy metrics
        if let Some(counter) = active_unhealthy_counter {
            let guard = counter.read();
            metrics.active_unhealthy_backends =
                guard.iter().map(|(k, v)| (k.clone(), *v)).collect();
        }
        return Ok((HttpResponse::BuiltinError(502, None), metrics));
    }

    // Extract affinity key and resolve affinity index
    let affinity_key = extract_affinity_key(&config.affinity, ctx);

    // Backend selection loop — retries on connection failure when retry_connection is enabled
    loop {
        // Select upstream via load balancing (tracker already initialized inside)
        let Some(selected) = determine_proxy_to(
            &upstreams,
            algorithm,
            conn_state,
            ewma_state,
            health_check_state,
            &config.circuit_breaker,
            Some(&circuit_breaker_state),
            config.affinity.as_ref().map(|t| &t.affinity_type),
            affinity_key.as_deref(),
            ring,
            &ctx.events,
            &mut metrics,
            ferron_http::trace_context::current_event_trace_context(ctx),
        ) else {
            ctx.events.emit(Event::Log(LogEvent {
                level: LogLevel::Error,
                message: "Reverse proxy: all upstream backends are unhealthy".to_string(),
                summary: "Reverse proxy: all upstream backends are unhealthy".into(),
                target: LOG_TARGET,
                attributes: Vec::new(),
                trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
            }));
            // Collect active health check unhealthy metrics
            if let Some(counter) = active_unhealthy_counter {
                let guard = counter.read();
                metrics.active_unhealthy_backends =
                    guard.iter().map(|(k, v)| (k.clone(), *v)).collect();
            }
            return Ok((HttpResponse::BuiltinError(503, None), metrics));
        };

        metrics.selected_backends.insert(selected.upstream.clone());
        metrics.final_selected_backend = Some(selected.upstream.clone());

        let proxy_request_url: http::Uri = selected
            .upstream
            .proxy_to
            .parse()
            .map_err(|_| ProxyError::InvalidUpstreamUrl(selected.upstream.proxy_to.clone()))?;
        let is_https = proxy_request_url.scheme_str() == Some("https");
        let client_ip = config.proxy_header.map(|_| ctx.remote_address.ip());
        let local_limit = cm.get_local_limit(selected.upstream.clone());
        let idle_timeout = idle_timeout_for_upstream(config, &selected.upstream);

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
            selected.tracker,
            &mut metrics,
        )
        .await
        {
            Ok(resp) => {
                if let Some(status) = metrics.status_code {
                    record_backend_response(
                        Some(&circuit_breaker_state),
                        &config.circuit_breaker,
                        &selected.upstream,
                        status,
                        &mut metrics,
                        &ctx.events,
                        ferron_http::trace_context::current_event_trace_context(ctx),
                    );
                }

                // Update EWMA latency for P2C+EWMA algorithm
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

                // Collect active health check unhealthy metrics
                if let Some(counter) = active_unhealthy_counter {
                    let guard = counter.read();
                    metrics.active_unhealthy_backends =
                        guard.iter().map(|(k, v)| (k.clone(), *v)).collect();
                }

                // Set affinity cookie if needed
                let resp = maybe_set_affinity_cookie(
                    resp,
                    &config.affinity,
                    affinity_key.map(|k| String::from_utf8_lossy(&k).to_string()),
                );

                return Ok((resp, metrics));
            }
            Err(e) => {
                record_backend_transport_failure(
                    Some(&circuit_breaker_state),
                    &config.circuit_breaker,
                    &selected.upstream,
                    &mut metrics,
                    &ctx.events,
                    ferron_http::trace_context::current_event_trace_context(ctx),
                );

                // Check if we should retry with another backend
                if config.retry_connection {
                    // Count how many healthy backends remain
                    let healthy_count = count_available_backends(
                        &upstreams,
                        health_check_state,
                        Some(&circuit_breaker_state),
                        &config.circuit_breaker,
                        &metrics.selected_backends,
                    );

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
                        continue; // Loop back to select next backend
                    }
                }

                // No retry or no more backends — return error
                let (status, reason) = if let ProxyError::Io(io_err) = &e {
                    io_error_status(io_err)
                } else {
                    (
                        e.http_status_hint().unwrap_or(StatusCode::BAD_GATEWAY),
                        "Bad gateway",
                    )
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
                // Collect active health check unhealthy metrics
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
