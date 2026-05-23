//! Core proxy logic: request transformation, TLS, connection establishment, and forwarding.

mod affinity;
mod backend;
mod connect;
mod pool;
mod request;
mod response;
mod tls;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use ferron_http::{HttpContext, HttpResponse};
use ferron_observability::{Event, LogEvent, LogLevel};
use http::StatusCode;
use parking_lot::{Mutex, RwLock};

use crate::config::ProxyConfig;
use crate::connections::ConnectionManager;
use crate::types::circuit::CircuitBreakerStateMap;
use crate::types::health::HealthCheckStateMap;
use crate::types::upstream::UpstreamInner;
use crate::types::ConnectionsTrackState;
use crate::upstream::lb::LoadBalancerAlgorithmInner;
use crate::upstream::{
    determine_proxy_to, record_backend_response, record_backend_transport_failure,
    resolve_upstreams,
};
use crate::util::TtlCache;
use crate::ProxyMetrics;

use self::affinity::{extract_affinity_index, maybe_set_affinity_cookie};
use self::backend::count_available_backends;
use self::tls::{cached_tls_config, io_error_status};

const LOG_TARGET: &str = "ferron-http-proxy";

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
    failed_backends: Arc<RwLock<TtlCache<UpstreamInner, u64>>>,
    circuit_breaker_state: CircuitBreakerStateMap,
    algorithm: &LoadBalancerAlgorithmInner,
    conn_state: Option<&ConnectionsTrackState>,
    health_check_state: Option<&HealthCheckStateMap>,
    active_unhealthy_counter: Option<&Mutex<HashMap<String, u64>>>,
) -> Result<(HttpResponse, ProxyMetrics), Box<dyn std::error::Error + Send + Sync>> {
    let mut metrics = ProxyMetrics::new();

    // Resolve upstreams (SRV records are resolved here, static ones pass through)
    let upstreams = resolve_upstreams(
        &config.upstreams,
        Arc::clone(&failed_backends),
        config.passive_check.max_fails,
    )
    .await;

    if upstreams.is_empty() {
        ctx.events.emit(Event::Log(LogEvent {
            level: LogLevel::Error,
            message: "Reverse proxy: no healthy upstream backends available".to_string(),
            target: LOG_TARGET,
            trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
        }));
        // Collect active health check unhealthy metrics
        if let Some(counter) = active_unhealthy_counter {
            let guard = counter.lock();
            metrics.active_unhealthy_backends =
                guard.iter().map(|(k, v)| (k.clone(), *v)).collect();
        }
        return Ok((HttpResponse::BuiltinError(502, None), metrics));
    }

    // Extract affinity key and resolve affinity index
    let affinity_index = extract_affinity_index(&config.affinity, ctx, &upstreams, algorithm);

    // Backend selection loop — retries on connection failure when retry_connection is enabled
    loop {
        // Select upstream via load balancing (tracker already initialized inside)
        let Some(selected) = determine_proxy_to(
            &upstreams,
            &failed_backends,
            config.passive_check.enabled,
            config.passive_check.max_fails,
            algorithm,
            conn_state,
            health_check_state,
            &config.circuit_breaker,
            Some(&circuit_breaker_state),
            &metrics.selected_backends,
            affinity_index,
            &ctx.events,
        ) else {
            ctx.events.emit(Event::Log(LogEvent {
                level: LogLevel::Error,
                message: "Reverse proxy: all upstream backends are unhealthy".to_string(),
                target: LOG_TARGET,
                trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
            }));
            // Collect active health check unhealthy metrics
            if let Some(counter) = active_unhealthy_counter {
                let guard = counter.lock();
                metrics.active_unhealthy_backends =
                    guard.iter().map(|(k, v)| (k.clone(), *v)).collect();
            }
            return Ok((HttpResponse::BuiltinError(503, None), metrics));
        };

        metrics.selected_backends.push(selected.upstream.clone());

        let proxy_request_url: http::Uri =
            selected.upstream.proxy_to.parse().map_err(|e| {
                format!("Invalid upstream URL '{}': {e}", selected.upstream.proxy_to)
            })?;
        let is_https = proxy_request_url.scheme_str() == Some("https");
        let client_ip = config.proxy_header.map(|_| ctx.remote_address.ip());
        let local_limit = cm.get_local_limit(&selected.upstream);
        let idle_timeout = idle_timeout_for_upstream(config, &selected.upstream);

        match pool::try_send_with_pool(
            ctx,
            config,
            cm,
            &selected.upstream,
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
                    );
                }

                // Collect active health check unhealthy metrics
                if let Some(counter) = active_unhealthy_counter {
                    let guard = counter.lock();
                    metrics.active_unhealthy_backends =
                        guard.iter().map(|(k, v)| (k.clone(), *v)).collect();
                }

                // Set affinity cookie if needed
                let resp = maybe_set_affinity_cookie(
                    resp,
                    &config.affinity,
                    affinity_index,
                    &selected.upstream,
                    &upstreams,
                );

                return Ok((resp, metrics));
            }
            Err(e) => {
                record_backend_transport_failure(
                    Arc::clone(&failed_backends),
                    config.passive_check.enabled,
                    Some(&circuit_breaker_state),
                    &config.circuit_breaker,
                    &selected.upstream,
                    &mut metrics,
                    &ctx.events,
                );

                // Check if we should retry with another backend
                if config.retry_connection {
                    // Count how many healthy backends remain
                    let healthy_count = count_available_backends(
                        &upstreams,
                        &failed_backends,
                        config.passive_check.max_fails,
                        health_check_state,
                        Some(&circuit_breaker_state),
                        &config.circuit_breaker,
                        &metrics.selected_backends,
                    );

                    if healthy_count > 0 && metrics.selected_backends.len() < upstreams.len() {
                        ctx.events.emit(Event::Log(LogEvent {
                            level: LogLevel::Warn,
                            message: format!(
                                "Reverse proxy: backend failed, retrying with another — upstream: {url}: {err}",
                                url = selected.upstream.proxy_to,
                                err = e
                            ),
                            target: LOG_TARGET,
                            trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
                        }));
                        continue; // Loop back to select next backend
                    }
                }

                // No retry or no more backends — return error
                let (status, reason) = e.downcast_ref::<std::io::Error>().map_or(
                    (StatusCode::BAD_GATEWAY, "Bad gateway"),
                    |io_err| {
                        let (st, r) = io_error_status(io_err);
                        (st, r)
                    },
                );
                ctx.events.emit(Event::Log(LogEvent {
                    level: LogLevel::Error,
                    message: format!(
                        "Reverse proxy: {reason} — upstream: {url}: {err}",
                        url = selected.upstream.proxy_to,
                        err = e
                    ),
                    target: LOG_TARGET,
                    trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
                }));
                // Collect active health check unhealthy metrics
                if let Some(counter) = active_unhealthy_counter {
                    let guard = counter.lock();
                    metrics.active_unhealthy_backends =
                        guard.iter().map(|(k, v)| (k.clone(), *v)).collect();
                }
                return Ok((HttpResponse::BuiltinError(status.as_u16(), None), metrics));
            }
        }
    }
}
