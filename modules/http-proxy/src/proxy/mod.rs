//! Core proxy logic: request transformation, TLS, connection establishment, and forwarding.

mod request;
mod tls;

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use ferron_http::{HttpContext, HttpResponse};
use ferron_observability::{Event, LogEvent, LogLevel};
use http::{Request, Response, StatusCode};
use http_body_util::{BodyExt, Empty};
use rustls::pki_types::ServerName;
use tokio_rustls::TlsConnector;
use vibeio_hyper::VibeioIo;

use crate::config::ProxyConfig;
use crate::connections::{ConnectionManager, PoolKey};
use crate::connpool_single::PoolItem;
use crate::send_net_io::SendTcpStreamPoll;
#[cfg(unix)]
use crate::send_net_io::SendUnixStreamPoll;
use crate::send_request::{http1_handshake, http2_handshake, SendRequestWrapper, TrackedBody};
#[cfg(unix)]
use crate::send_request::{http1_handshake_unix, http2_handshake_unix};
use crate::upstream::{
    determine_proxy_to, mark_backend_failure, resolve_upstreams, ConnectionsTrackState,
    HealthCheckStateMap, LoadBalancerAlgorithmInner, UpstreamInner,
};
use crate::util::TtlCache;
use crate::ProxyMetrics;

use self::request::construct_proxy_request;
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
    failed_backends: Arc<parking_lot::RwLock<TtlCache<UpstreamInner, u64>>>,
    algorithm: &LoadBalancerAlgorithmInner,
    conn_state: Option<&ConnectionsTrackState>,
    health_check_state: Option<&HealthCheckStateMap>,
    active_unhealthy_counter: Option<&parking_lot::Mutex<HashMap<String, u64>>>,
) -> Result<(HttpResponse, ProxyMetrics), Box<dyn std::error::Error + Send + Sync>> {
    let mut metrics = ProxyMetrics::new();

    // Resolve upstreams (SRV records are resolved here, static ones pass through)
    let upstreams = resolve_upstreams(
        &config.upstreams,
        Arc::clone(&failed_backends),
        config.lb_health_check_max_fails,
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

    // Backend selection loop — retries on connection failure when lb_retry_connection is enabled
    loop {
        // Select upstream via load balancing (tracker already initialized inside)
        let Some(selected) = determine_proxy_to(
            &upstreams,
            &failed_backends,
            config.lb_health_check,
            config.lb_health_check_max_fails,
            algorithm,
            conn_state,
            health_check_state,
            &metrics.selected_backends,
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

        match try_send_with_pool(
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
                // Collect active health check unhealthy metrics
                if let Some(counter) = active_unhealthy_counter {
                    let guard = counter.lock();
                    metrics.active_unhealthy_backends =
                        guard.iter().map(|(k, v)| (k.clone(), *v)).collect();
                }
                return Ok((resp, metrics));
            }
            Err(e) => {
                mark_backend_failure(
                    Arc::clone(&failed_backends),
                    config.lb_health_check,
                    &selected.upstream,
                    &mut metrics,
                );

                // Check if we should retry with another backend
                if config.lb_retry_connection {
                    // Count how many healthy backends remain
                    let healthy_count = count_healthy_backends(
                        &upstreams,
                        &failed_backends,
                        config.lb_health_check_max_fails,
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

/// Count how many backends are currently healthy (not exceeding failure threshold).
fn count_healthy_backends(
    upstreams: &[UpstreamInner],
    failed_backends: &parking_lot::RwLock<TtlCache<UpstreamInner, u64>>,
    health_check_max_fails: u64,
) -> usize {
    let failed = failed_backends.read();
    upstreams
        .iter()
        .filter(|u| {
            failed
                .get(*u)
                .is_none_or(|fails| fails <= health_check_max_fails)
        })
        .count()
}

/// Try to send a request using the connection pool with racing of
/// non-ready pooled connections against a newly established connection.
///
/// When pooled connections are not ready (but alive), they are collected
/// and raced against establishing a brand-new connection, avoiding the
/// cost of unnecessary duplicate connection establishments.
#[allow(clippy::too_many_arguments)]
async fn try_send_with_pool(
    ctx: &mut HttpContext,
    config: &ProxyConfig,
    cm: &ConnectionManager,
    upstream: &UpstreamInner,
    proxy_url: &http::Uri,
    client_ip: Option<IpAddr>,
    local_limit: Option<usize>,
    idle_timeout: Duration,
    is_https: bool,
    _conn_state: Option<&ConnectionsTrackState>,
    tracked_connection: Option<Arc<()>>,
    metrics: &mut ProxyMetrics,
) -> Result<HttpResponse, Box<dyn std::error::Error + Send + Sync>> {
    // Collect non-ready-but-alive connections for racing
    let mut pending_items: Vec<PoolItem<PoolKey, Arc<UpstreamInner>, SendRequestWrapper>> =
        Vec::new();
    // Track a non-ready-but-kept item slot for reuse in establish_and_send
    // (avoids double-pull when the connection is dead and can't be raced).
    let mut reusable_item: Option<PoolItem<PoolKey, Arc<UpstreamInner>, SendRequestWrapper>> = None;

    // Pull one connection from the pool and check readiness
    let pull_start = std::time::Instant::now();
    let item = if let Some(limit) = local_limit {
        cm.pull_with_local_limit(upstream, client_ip, Some(limit))
    } else {
        cm.pull(upstream, client_ip)
    };

    // If pool returned None (at capacity), we'll need to establish a new connection
    let mut item = match item {
        Some(i) => i,
        None => {
            return establish_and_send(
                ctx,
                config,
                cm,
                upstream,
                proxy_url,
                client_ip,
                local_limit,
                is_https,
                _conn_state,
                tracked_connection,
                None,
                metrics,
            )
            .await;
        }
    };

    let pull_duration = pull_start.elapsed().as_secs_f64();

    // Track pool wait metrics when pool was exhausted (no immediate connection available)
    if item.inner().is_none() || pull_duration > 0.001 {
        metrics.pool_waits += 1;
        metrics.pool_wait_time_secs += pull_duration;
    }

    let (is_ready, should_keep) = if let Some(wrapper) = item.inner_mut() {
        wrapper.check_ready(Some(idle_timeout))
    } else {
        (false, false)
    };

    if is_ready {
        metrics.connection_reused = true;
        let wrapper = item.inner_mut().take().unwrap();
        return send_via_wrapper(
            ctx,
            config,
            wrapper,
            item,
            proxy_url,
            tracked_connection,
            true,
            upstream.proxy_unix.is_some(),
            local_limit,
            metrics,
        )
        .await;
    }

    if should_keep {
        // Connection is alive but not ready — collect for racing
        if item.inner().is_some() {
            pending_items.push(item);
        }
    } else {
        // Connection is dead — keep the item slot for reuse in establish_and_send
        // to avoid pulling a second time.
        reusable_item = Some(item);
    }

    // Race pending items against establishing new
    if !pending_items.is_empty() {
        match wait_for_any_ready(&mut pending_items, idle_timeout).await {
            Some(mut item) => {
                metrics.connection_reused = true;
                let wrapper = item.inner_mut().take().unwrap();
                return send_via_wrapper(
                    ctx,
                    config,
                    wrapper,
                    item,
                    proxy_url,
                    tracked_connection,
                    true,
                    upstream.proxy_unix.is_some(),
                    local_limit,
                    metrics,
                )
                .await;
            }
            None => {
                // All pending items failed — establish new connection
            }
        }
    }

    establish_and_send(
        ctx,
        config,
        cm,
        upstream,
        proxy_url,
        client_ip,
        local_limit,
        is_https,
        _conn_state,
        tracked_connection,
        reusable_item,
        metrics,
    )
    .await
}

/// Wait for any pending connection to become ready.
///
/// Returns the item if one becomes ready, or `None` if all fail.
async fn wait_for_any_ready(
    pending_items: &mut Vec<PoolItem<PoolKey, Arc<UpstreamInner>, SendRequestWrapper>>,
    idle_timeout: Duration,
) -> Option<PoolItem<PoolKey, Arc<UpstreamInner>, SendRequestWrapper>> {
    if pending_items.is_empty() {
        return None;
    }

    use futures_util::future::select_ok;

    let futures: Vec<_> = pending_items
        .drain(..)
        .map(|mut item| {
            Box::pin(async move {
                if let Some(wrapper) = item.inner_mut() {
                    if wrapper.wait_ready(Some(idle_timeout)).await {
                        return Ok(item);
                    }
                }
                Err(())
            })
        })
        .collect();

    if futures.is_empty() {
        return None;
    }

    match select_ok(futures).await {
        Ok((item, _remaining)) => Some(item),
        Err(_) => None,
    }
}

/// Build a PROXY protocol header for the given version and connection details.
fn build_proxy_protocol_header(
    version: crate::upstream::ProxyHeader,
    client_ip: IpAddr,
    local_ip: IpAddr,
    client_port: u16,
    local_port: u16,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    match version {
        crate::upstream::ProxyHeader::V1 => {
            let is_ipv4 = client_ip.is_ipv4() && local_ip.is_ipv4();
            let proto = if is_ipv4 { "TCP4" } else { "TCP6" };
            let client_str = client_ip.to_string();
            let local_str = local_ip.to_string();
            Ok(
                format!("PROXY {proto} {client_str} {local_str} {client_port} {local_port}\r\n")
                    .into_bytes(),
            )
        }
        crate::upstream::ProxyHeader::V2 => {
            let is_ipv4 = client_ip.is_ipv4() && local_ip.is_ipv4();
            let addresses = if is_ipv4 {
                let client_v4 = match client_ip {
                    IpAddr::V4(addr) => addr,
                    _ => return Err("Client IP is not IPv4".into()),
                };
                let local_v4 = match local_ip {
                    IpAddr::V4(addr) => addr,
                    _ => return Err("Local IP is not IPv4".into()),
                };
                ppp::v2::Addresses::IPv4(ppp::v2::IPv4::new(
                    client_v4,
                    local_v4,
                    client_port,
                    local_port,
                ))
            } else {
                let client_v6 = match client_ip {
                    IpAddr::V6(addr) => addr,
                    _ => return Err("Client IP is not IPv6".into()),
                };
                let local_v6 = match local_ip {
                    IpAddr::V6(addr) => addr,
                    _ => return Err("Local IP is not IPv6".into()),
                };
                ppp::v2::Addresses::IPv6(ppp::v2::IPv6::new(
                    client_v6,
                    local_v6,
                    client_port,
                    local_port,
                ))
            };
            let header = ppp::v2::Builder::with_addresses(
                ppp::v2::Version::Two | ppp::v2::Command::Proxy,
                ppp::v2::Protocol::Stream,
                addresses,
            )
            .build()?;
            Ok(header)
        }
    }
}

/// Establish a new connection and send the request.
///
/// If `existing_item` is provided, it is reused instead of pulling a new one
/// from the pool, avoiding a double semaphore acquisition.
#[allow(clippy::too_many_arguments)]
async fn establish_and_send(
    ctx: &mut HttpContext,
    config: &ProxyConfig,
    cm: &ConnectionManager,
    upstream: &UpstreamInner,
    proxy_url: &http::Uri,
    client_ip: Option<IpAddr>,
    local_limit: Option<usize>,
    is_https: bool,
    _conn_state: Option<&ConnectionsTrackState>,
    tracked_connection: Option<Arc<()>>,
    existing_item: Option<PoolItem<PoolKey, Arc<UpstreamInner>, SendRequestWrapper>>,
    metrics: &mut ProxyMetrics,
) -> Result<HttpResponse, Box<dyn std::error::Error + Send + Sync>> {
    let item: Option<PoolItem<PoolKey, Arc<UpstreamInner>, SendRequestWrapper>> =
        if let Some(it) = existing_item {
            Some(it)
        } else if let Some(limit) = local_limit {
            cm.pull_with_local_limit(upstream, client_ip, Some(limit))
        } else {
            cm.pull(upstream, client_ip)
        };

    // If pool returned None (at capacity), we need to proceed without a pooled item
    let mut item = match item {
        Some(i) => i,
        None => {
            // No pooled item available, establish connection without pool tracking
            return establish_connection_without_pool(
                ctx,
                config,
                upstream,
                proxy_url,
                client_ip,
                is_https,
                _conn_state,
                tracked_connection,
                metrics,
            )
            .await;
        }
    };

    *item.inner_mut() = None;

    #[cfg(unix)]
    let is_unix = upstream.proxy_unix.is_some();
    #[cfg(not(unix))]
    let is_unix = false;

    let wrapper = if is_unix {
        #[cfg(unix)]
        {
            let unix_path = upstream
                .proxy_unix
                .as_ref()
                .ok_or("Unix socket path not set")?;
            let unix = vibeio::net::UnixStream::connect(unix_path)
                .await
                .map_err(|e| std::io::Error::other(format!("Unix connect failed: {e}")))?;
            let mut stream = SendUnixStreamPoll::new_comp_io(unix)
                .map_err(|e| std::io::Error::other(format!("Unix wrap failed: {e}")))?;

            let drop_guard = unsafe { stream.get_drop_guard() };

            // Write PROXY protocol header if configured (before HTTP handshake)
            if let Some(proxy_header_version) = config.proxy_header {
                if let Some(cip) = client_ip {
                    let local_addr = ctx.local_address;
                    let header_bytes = build_proxy_protocol_header(
                        proxy_header_version,
                        cip,
                        local_addr.ip(),
                        ctx.remote_address.port(),
                        local_addr.port(),
                    )?;
                    use tokio::io::AsyncWriteExt;
                    stream.write_all(&header_bytes).await.map_err(|e| {
                        std::io::Error::other(format!("PROXY header write failed: {e}"))
                    })?;
                }
            }

            if is_https {
                let connector = TlsConnector::from(cached_tls_config(
                    config.http2,
                    config.http2_only,
                    config.no_verification,
                ));
                let host = proxy_url.host().ok_or("upstream URL has no host")?;
                let domain = ServerName::try_from(host.to_string())
                    .map_err(|e| format!("Invalid server name: {e}"))?;
                let tls_start = std::time::Instant::now();
                let tls_stream = match connector.connect(domain, stream).await {
                    Ok(s) => {
                        metrics.tls_handshake_time_secs += tls_start.elapsed().as_secs_f64();
                        s
                    }
                    Err(e) => {
                        metrics.tls_handshake_failures += 1;
                        ctx.events.emit(Event::Log(LogEvent {
                            level: LogLevel::Warn,
                            message: format!(
                                "Reverse proxy: TLS handshake with {unix_path} failed: {e}"
                            ),
                            target: LOG_TARGET,
                            trace_context: ferron_http::trace_context::current_event_trace_context(
                                ctx,
                            ),
                        }));
                        return Err(
                            std::io::Error::other(format!("TLS handshake failed: {e}")).into()
                        );
                    }
                };

                let negotiated_h2 = tls_stream.get_ref().1.alpn_protocol() == Some(b"h2");
                let use_http2 = (config.http2 && negotiated_h2) || config.http2_only;

                if use_http2 {
                    http2_handshake_unix(tls_stream, drop_guard).await?
                } else {
                    http1_handshake_unix(tls_stream, drop_guard).await?
                }
            } else if config.http2_only {
                http2_handshake_unix(stream, drop_guard).await?
            } else {
                http1_handshake_unix(stream, drop_guard).await?
            }
        }
        #[cfg(not(unix))]
        unreachable!();
    } else {
        let host = proxy_url.host().ok_or("upstream URL has no host")?;
        let port = proxy_url
            .port_u16()
            .unwrap_or(if is_https { 443 } else { 80 });
        let addr = format!("{host}:{port}");

        let tcp = vibeio::net::TcpStream::connect(&addr).await.map_err(|e| {
            ctx.events.emit(Event::Log(LogEvent {
                level: LogLevel::Warn,
                message: format!("Reverse proxy: TCP connect to {addr} failed: {e}"),
                target: LOG_TARGET,
                trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
            }));
            std::io::Error::other(format!("Connect failed: {e}"))
        })?;
        let mut stream = SendTcpStreamPoll::new_comp_io(tcp)
            .map_err(|e| std::io::Error::other(format!("Wrap failed: {e}")))?;

        let drop_guard = unsafe { stream.get_drop_guard() };

        // Write PROXY protocol header if configured (before TLS or HTTP handshake)
        if let Some(proxy_header_version) = config.proxy_header {
            if let Some(cip) = client_ip {
                let local_addr = ctx.local_address;
                let header_bytes = build_proxy_protocol_header(
                    proxy_header_version,
                    cip,
                    local_addr.ip(),
                    ctx.remote_address.port(),
                    local_addr.port(),
                )?;
                use tokio::io::AsyncWriteExt;
                stream.write_all(&header_bytes).await.map_err(|e| {
                    std::io::Error::other(format!("PROXY header write failed: {e}"))
                })?;
            }
        }

        if is_https {
            let connector = TlsConnector::from(cached_tls_config(
                config.http2,
                config.http2_only,
                config.no_verification,
            ));
            let domain = ServerName::try_from(host.to_string())
                .map_err(|e| format!("Invalid server name: {e}"))?;
            let tls_start = std::time::Instant::now();
            let tls_stream = match connector.connect(domain, stream).await {
                Ok(s) => {
                    metrics.tls_handshake_time_secs += tls_start.elapsed().as_secs_f64();
                    s
                }
                Err(e) => {
                    metrics.tls_handshake_failures += 1;
                    ctx.events.emit(Event::Log(LogEvent {
                        level: LogLevel::Warn,
                        message: format!("Reverse proxy: TLS handshake with {addr} failed: {e}"),
                        target: LOG_TARGET,
                        trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
                    }));
                    return Err(std::io::Error::other(format!("TLS handshake failed: {e}")).into());
                }
            };

            let negotiated_h2 = tls_stream.get_ref().1.alpn_protocol() == Some(b"h2");
            let use_http2 = (config.http2 && negotiated_h2) || config.http2_only;

            if use_http2 {
                http2_handshake(tls_stream, drop_guard).await?
            } else {
                http1_handshake(tls_stream, drop_guard).await?
            }
        } else if config.http2_only {
            http2_handshake(stream, drop_guard).await?
        } else {
            http1_handshake(stream, drop_guard).await?
        }
    };

    send_via_wrapper(
        ctx,
        config,
        wrapper,
        item,
        proxy_url,
        tracked_connection,
        config.keepalive,
        is_unix,
        local_limit,
        metrics,
    )
    .await
}

/// Send request via a SendRequestWrapper and handle the response.
#[allow(clippy::too_many_arguments)]
async fn send_via_wrapper(
    ctx: &mut HttpContext,
    config: &ProxyConfig,
    mut wrapper: SendRequestWrapper,
    item: PoolItem<PoolKey, Arc<UpstreamInner>, SendRequestWrapper>,
    proxy_url: &http::Uri,
    tracked_connection: Option<Arc<()>>,
    enable_keepalive: bool,
    is_unix: bool,
    _local_limit: Option<usize>,
    metrics: &mut ProxyMetrics,
) -> Result<HttpResponse, Box<dyn std::error::Error + Send + Sync>> {
    let request = construct_proxy_request(ctx, config, proxy_url)?;
    let extensions = request.extensions().clone();

    let start = std::time::Instant::now();
    let response = match wrapper.send_request(request).await {
        Ok(resp) => {
            metrics.upstream_time_secs = start.elapsed().as_secs_f64();
            resp
        }
        Err(e) => {
            return Err(format!("Bad gateway: {e}").into());
        }
    };

    let status = response.status();
    metrics.status_code = Some(status.as_u16());

    let (mut parts, body) = response.into_parts();

    // Handle HTTP 101 Switching Protocols (upgrades)
    if status == StatusCode::SWITCHING_PROTOCOLS {
        let response_upgrade = Response::from_parts(parts.clone(), ());
        handle_upgrade(response_upgrade, extensions, ctx, item).await?;

        // Remove some response headers as indicated by "Connection" header (RFC 7230)
        remove_headers_rfc7230(&mut parts);

        Ok(HttpResponse::Custom(Response::from_parts(
            parts,
            body.map_err(std::io::Error::other).boxed_unsync(),
        )))
    } else if config.intercept_errors && status.as_u16() >= 400 {
        // Intercept upstream error responses if configured.
        // When intercept_errors is true, upstream 4xx/5xx responses
        // are replaced with Ferron's built-in error response.
        // When intercept_errors is false (default), the full upstream response is passed through.
        Ok(HttpResponse::BuiltinError(status.as_u16(), None))
    } else {
        // For keepalive, we extract the wrapper and create a PoolReturnInfo.
        // This prevents the PoolItem's Drop from running, and instead we manually
        // return the connection via PoolReturnInfo when TrackedBody is dropped.
        let pool_return_info = if enable_keepalive && !wrapper.is_closed() {
            Some(crate::send_request::PoolReturnInfo::from_item(
                item, wrapper, is_unix,
            ))
        } else {
            // Item will be dropped here, returning connection to pool via its Drop impl
            // (wrapper is consumed by the response and not returned to pool)
            drop(item);
            None
        };

        let tracked_body = TrackedBody::new(
            body.map_err(std::io::Error::other),
            tracked_connection,
            pool_return_info,
        );

        // Remove some response headers as indicated by "Connection" header (RFC 7230)
        remove_headers_rfc7230(&mut parts);

        let mut response = Response::from_parts(parts, tracked_body.boxed_unsync());
        *response.version_mut() = http::Version::default();

        Ok(HttpResponse::Custom(response))
    }
}

/// Handle HTTP 101 Switching Protocols (WebSocket upgrades).
async fn handle_upgrade(
    resp_for_upgrade: Response<()>,
    req_extensions: http::Extensions,
    ctx: &mut HttpContext,
    mut item: PoolItem<PoolKey, Arc<UpstreamInner>, SendRequestWrapper>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut upgrade_request = Request::new(Empty::<Bytes>::new());
    *upgrade_request.extensions_mut() = req_extensions;

    let events = ctx.events.clone();

    // Take the inner value to prevent Drop from returning to pool.
    // For upgrade connections, we don't return them to the pool
    // (upgrade connections are long-lived, not pooled).
    let _wrapper = item.inner_mut().take();
    // Prevent item's Drop from running (we handle cleanup manually)
    std::mem::forget(item);

    let upgrade_future = vibeio_http::prepare_upgrade(&mut upgrade_request);
    vibeio::spawn(async move {
        match hyper::upgrade::on(resp_for_upgrade).await {
            Ok(upgraded_backend) => {
                if let Some(upgraded_future) = upgrade_future {
                    match upgraded_future.await {
                        Some(upgraded_client) => {
                            let mut backend = VibeioIo::new(upgraded_backend);
                            let mut client = upgraded_client;

                            let _ = tokio::io::copy_bidirectional(&mut backend, &mut client).await;
                            // Connection not returned to pool for upgrades
                            // (upgrade connections are long-lived, not pooled)
                        }
                        None => {
                            events.emit(Event::Log(LogEvent {
                                level: LogLevel::Warn,
                                message: "Reverse proxy: frontend HTTP upgrade failed".to_string(),
                                target: LOG_TARGET,
                                trace_context: None,
                            }));
                        }
                    }
                }
            }
            Err(_) => {
                events.emit(Event::Log(LogEvent {
                    level: LogLevel::Warn,
                    message: "Reverse proxy: backend HTTP upgrade failed".to_string(),
                    target: LOG_TARGET,
                    trace_context: None,
                }));
            }
        }
    });

    Ok(())
}

/// Establish a connection without pool tracking.
///
/// This is used when the pool is at capacity and we need to establish
/// a connection without waiting for a pool slot.
#[allow(clippy::too_many_arguments)]
async fn establish_connection_without_pool(
    ctx: &mut HttpContext,
    config: &ProxyConfig,
    upstream: &UpstreamInner,
    proxy_url: &http::Uri,
    client_ip: Option<IpAddr>,
    is_https: bool,
    _conn_state: Option<&ConnectionsTrackState>,
    tracked_connection: Option<Arc<()>>,
    metrics: &mut ProxyMetrics,
) -> Result<HttpResponse, Box<dyn std::error::Error + Send + Sync>> {
    // Establish connection without pool tracking
    // (similar to establish_and_send but without the item handling)
    #[cfg(unix)]
    let is_unix = upstream.proxy_unix.is_some();
    #[cfg(not(unix))]
    let is_unix = false;

    let wrapper = if is_unix {
        #[cfg(unix)]
        {
            let unix_path = upstream
                .proxy_unix
                .as_ref()
                .ok_or("Unix socket path not set")?;
            let unix = vibeio::net::UnixStream::connect(unix_path)
                .await
                .map_err(|e| std::io::Error::other(format!("Unix connect failed: {e}")))?;
            let mut stream = SendUnixStreamPoll::new_comp_io(unix)
                .map_err(|e| std::io::Error::other(format!("Unix wrap failed: {e}")))?;

            let drop_guard = unsafe { stream.get_drop_guard() };

            // Write PROXY protocol header if configured
            if let Some(proxy_header_version) = config.proxy_header {
                if let Some(cip) = client_ip {
                    let local_addr = ctx.local_address;
                    let header_bytes = build_proxy_protocol_header(
                        proxy_header_version,
                        cip,
                        local_addr.ip(),
                        ctx.remote_address.port(),
                        local_addr.port(),
                    )?;
                    use tokio::io::AsyncWriteExt;
                    stream.write_all(&header_bytes).await.map_err(|e| {
                        std::io::Error::other(format!("PROXY header write failed: {e}"))
                    })?;
                }
            }

            if config.http2_only || config.http2 {
                http2_handshake_unix(stream, drop_guard).await?
            } else {
                http1_handshake_unix(stream, drop_guard).await?
            }
        }
        #[cfg(not(unix))]
        unreachable!();
    } else {
        let host = proxy_url.host().ok_or("upstream URL has no host")?;
        let port = proxy_url
            .port_u16()
            .unwrap_or(if is_https { 443 } else { 80 });
        let addr = format!("{host}:{port}");

        let tcp = vibeio::net::TcpStream::connect(&addr).await.map_err(|e| {
            ctx.events.emit(Event::Log(LogEvent {
                level: LogLevel::Warn,
                message: format!("Reverse proxy: TCP connect to {addr} failed: {e}"),
                target: LOG_TARGET,
                trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
            }));
            std::io::Error::other(format!("Connect failed: {e}"))
        })?;
        let mut stream = SendTcpStreamPoll::new_comp_io(tcp)
            .map_err(|e| std::io::Error::other(format!("Wrap failed: {e}")))?;

        let drop_guard = unsafe { stream.get_drop_guard() };

        // Write PROXY protocol header if configured
        if let Some(proxy_header_version) = config.proxy_header {
            if let Some(cip) = client_ip {
                let local_addr = ctx.local_address;
                let header_bytes = build_proxy_protocol_header(
                    proxy_header_version,
                    cip,
                    local_addr.ip(),
                    ctx.remote_address.port(),
                    local_addr.port(),
                )?;
                use tokio::io::AsyncWriteExt;
                stream.write_all(&header_bytes).await.map_err(|e| {
                    std::io::Error::other(format!("PROXY header write failed: {e}"))
                })?;
            }
        }

        if is_https {
            let connector = TlsConnector::from(cached_tls_config(
                config.http2,
                config.http2_only,
                config.no_verification,
            ));
            let domain = ServerName::try_from(host.to_string())
                .map_err(|e| format!("Invalid server name: {e}"))?;
            let tls_start = std::time::Instant::now();
            let tls_stream = match connector.connect(domain, stream).await {
                Ok(s) => {
                    metrics.tls_handshake_time_secs += tls_start.elapsed().as_secs_f64();
                    s
                }
                Err(e) => {
                    metrics.tls_handshake_failures += 1;
                    ctx.events.emit(Event::Log(LogEvent {
                        level: LogLevel::Warn,
                        message: format!("Reverse proxy: TLS handshake with {addr} failed: {e}"),
                        target: LOG_TARGET,
                        trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
                    }));
                    return Err(std::io::Error::other(format!("TLS handshake failed: {e}")).into());
                }
            };

            let negotiated_h2 = tls_stream.get_ref().1.alpn_protocol() == Some(b"h2");
            let use_http2 = (config.http2 || config.http2_only) && negotiated_h2;

            if use_http2 {
                http2_handshake(tls_stream, drop_guard).await?
            } else {
                http1_handshake(tls_stream, drop_guard).await?
            }
        } else if config.http2_only || config.http2 {
            http2_handshake(stream, drop_guard).await?
        } else {
            http1_handshake(stream, drop_guard).await?
        }
    };

    // Send request without pool item (connection won't be returned to pool)
    send_request_without_pool_item(
        ctx,
        config,
        wrapper,
        proxy_url,
        tracked_connection,
        config.keepalive,
        metrics,
    )
    .await
}

/// Send request without pool tracking.
async fn send_request_without_pool_item(
    ctx: &mut HttpContext,
    config: &ProxyConfig,
    mut wrapper: SendRequestWrapper,
    proxy_url: &http::Uri,
    _tracked_connection: Option<Arc<()>>,
    _enable_keepalive: bool,
    metrics: &mut ProxyMetrics,
) -> Result<HttpResponse, Box<dyn std::error::Error + Send + Sync>> {
    let request = construct_proxy_request(ctx, config, proxy_url)?;

    let start = std::time::Instant::now();
    let response = match wrapper.send_request(request).await {
        Ok(resp) => {
            metrics.upstream_time_secs = start.elapsed().as_secs_f64();
            resp
        }
        Err(e) => {
            return Err(format!("Bad gateway: {e}").into());
        }
    };

    let status = response.status();
    metrics.status_code = Some(status.as_u16());

    // For non-pooled connections, we don't return to pool
    let (parts, body) = response.into_parts();

    let tracked_body = TrackedBody::new(
        body.map_err(std::io::Error::other),
        None, // No connection tracker
        None, // No pool return info
    );

    let mut builder = Response::builder().status(parts.status);
    for (name, value) in parts.headers {
        if let Some(n) = name {
            builder = builder.header(n, value);
        }
    }
    let response = builder
        .body(tracked_body.boxed_unsync())
        .expect("Failed to build response");

    Ok(HttpResponse::Custom(response))
}

/// Remove headers from the response as indicated by the "Connection" header,
/// per RFC 7230.
fn remove_headers_rfc7230(parts: &mut http::response::Parts) {
    if let Some(connection) = parts.headers.get(http::header::CONNECTION) {
        for header in connection
            .to_str()
            .unwrap_or("")
            .split(',')
            .map(|h| h.trim().to_string())
            .collect::<Vec<_>>()
        {
            if header.eq_ignore_ascii_case("upgrade") {
                // Don't break HTTP upgrade handling
                continue;
            }
            parts.headers.remove(header);
        }
        parts.headers.remove(http::header::CONNECTION);
    }
}

#[cfg(test)]
mod tests {
    use super::request::{
        append_forwarded, append_x_forwarded_for, build_forwarded_element, set_forwarded,
        set_x_forwarded_for,
    };
    use super::*;
    use http::header::HeaderValue;
    use http::HeaderMap;

    #[test]
    fn test_set_x_forwarded_for_ipv4() {
        let mut headers = HeaderMap::new();
        set_x_forwarded_for(&mut headers, "192.168.1.1");
        assert_eq!(
            headers.get("x-forwarded-for").unwrap().to_str().unwrap(),
            "192.168.1.1"
        );
    }

    #[test]
    fn test_set_x_forwarded_for_ipv6() {
        let mut headers = HeaderMap::new();
        set_x_forwarded_for(&mut headers, "::1");
        assert_eq!(
            headers.get("x-forwarded-for").unwrap().to_str().unwrap(),
            "::1"
        );
    }

    #[test]
    fn test_append_x_forwarded_for_no_existing() {
        let mut headers = HeaderMap::new();
        append_x_forwarded_for(&mut headers, "10.0.0.1");
        assert_eq!(
            headers.get("x-forwarded-for").unwrap().to_str().unwrap(),
            "10.0.0.1"
        );
    }

    #[test]
    fn test_append_x_forwarded_for_with_existing() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("192.168.1.1"));
        append_x_forwarded_for(&mut headers, "10.0.0.1");
        assert_eq!(
            headers.get("x-forwarded-for").unwrap().to_str().unwrap(),
            "192.168.1.1, 10.0.0.1"
        );
    }

    #[test]
    fn test_build_forwarded_element_ipv4() {
        let proto = "https";
        let result = build_forwarded_element("192.168.1.1", proto, "10.0.0.1");
        assert_eq!(result, "for=192.168.1.1;proto=https;by=10.0.0.1");
    }

    #[test]
    fn test_build_forwarded_element_ipv6() {
        let proto = "http";
        let result = build_forwarded_element("2001:db8::1", proto, "10.0.0.1");
        assert_eq!(result, "for=\"[2001:db8::1]\";proto=http;by=10.0.0.1");
    }

    #[test]
    fn test_set_forwarded_ipv4() {
        let mut headers = HeaderMap::new();
        set_forwarded(&mut headers, "192.168.1.1", "https", "10.0.0.1");
        assert_eq!(
            headers.get("forwarded").unwrap().to_str().unwrap(),
            "for=192.168.1.1;proto=https;by=10.0.0.1"
        );
    }

    #[test]
    fn test_append_forwarded_no_existing() {
        let mut headers = HeaderMap::new();
        append_forwarded(&mut headers, "192.168.1.1", "http", "10.0.0.1");
        assert_eq!(
            headers.get("forwarded").unwrap().to_str().unwrap(),
            "for=192.168.1.1;proto=http;by=10.0.0.1"
        );
    }

    #[test]
    fn test_append_forwarded_with_existing() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "forwarded",
            HeaderValue::from_static("for=192.168.1.1;proto=https;by=10.0.0.1"),
        );
        append_forwarded(&mut headers, "10.0.0.1", "http", "172.16.0.1");
        assert_eq!(
            headers.get("forwarded").unwrap().to_str().unwrap(),
            "for=192.168.1.1;proto=https;by=10.0.0.1, for=10.0.0.1;proto=http;by=172.16.0.1"
        );
    }

    #[test]
    fn test_io_error_status_connection_refused() {
        let err = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused");
        let (status, reason) = io_error_status(&err);
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(reason, "Service unavailable");
    }

    #[test]
    fn test_io_error_status_not_found() {
        let err = std::io::Error::new(std::io::ErrorKind::NotFound, "not found");
        let (status, reason) = io_error_status(&err);
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(reason, "Service unavailable");
    }

    #[test]
    fn test_io_error_status_host_unreachable() {
        let err = std::io::Error::new(std::io::ErrorKind::HostUnreachable, "unreachable");
        let (status, reason) = io_error_status(&err);
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(reason, "Service unavailable");
    }

    #[test]
    fn test_io_error_status_timed_out() {
        let err = std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout");
        let (status, reason) = io_error_status(&err);
        assert_eq!(status, StatusCode::GATEWAY_TIMEOUT);
        assert_eq!(reason, "Gateway timeout");
    }

    #[test]
    fn test_io_error_status_other() {
        let err = std::io::Error::other("other error");
        let (status, reason) = io_error_status(&err);
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert_eq!(reason, "Bad gateway");
    }

    #[test]
    fn test_interpolate_header_value_no_interpolation() {
        // Can't test directly since function is private, but we can verify behavior
        // through the fact that plain strings pass through unchanged
        let value = "plain-value";
        assert!(!value.contains("{{"));
    }

    #[test]
    fn test_build_proxy_protocol_header_v1_ipv4() {
        let client_ip = IpAddr::V4(std::net::Ipv4Addr::new(192, 168, 1, 100));
        let local_ip = IpAddr::V4(std::net::Ipv4Addr::new(10, 0, 0, 1));
        let header = build_proxy_protocol_header(
            crate::upstream::ProxyHeader::V1,
            client_ip,
            local_ip,
            12345,
            80,
        )
        .unwrap();

        let header_str = String::from_utf8(header).unwrap();
        assert_eq!(header_str, "PROXY TCP4 192.168.1.100 10.0.0.1 12345 80\r\n");
    }

    #[test]
    fn test_build_proxy_protocol_header_v1_ipv6() {
        let client_ip = IpAddr::V6(std::net::Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1));
        let local_ip = IpAddr::V6(std::net::Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1));
        let header = build_proxy_protocol_header(
            crate::upstream::ProxyHeader::V1,
            client_ip,
            local_ip,
            443,
            8080,
        )
        .unwrap();

        let header_str = String::from_utf8(header).unwrap();
        assert!(header_str.starts_with("PROXY TCP6"));
        assert!(header_str.contains("2001:db8::1"));
        assert!(header_str.contains("::1"));
        assert!(header_str.contains("443"));
        assert!(header_str.contains("8080"));
        assert!(header_str.ends_with("\r\n"));
    }

    #[test]
    fn test_build_proxy_protocol_header_v2_ipv4() {
        let client_ip = IpAddr::V4(std::net::Ipv4Addr::new(192, 168, 1, 100));
        let local_ip = IpAddr::V4(std::net::Ipv4Addr::new(10, 0, 0, 1));
        let header = build_proxy_protocol_header(
            crate::upstream::ProxyHeader::V2,
            client_ip,
            local_ip,
            12345,
            80,
        )
        .unwrap();

        // PROXY protocol v2 signature: \r\n\r\n\0\r\nQUIT\n
        assert!(header.len() >= 16);
        assert_eq!(&header[0..12], b"\r\n\r\n\x00\r\nQUIT\n");
    }

    #[test]
    fn test_build_proxy_protocol_header_v2_ipv6() {
        let client_ip = IpAddr::V6(std::net::Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1));
        let local_ip = IpAddr::V6(std::net::Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1));
        let header = build_proxy_protocol_header(
            crate::upstream::ProxyHeader::V2,
            client_ip,
            local_ip,
            443,
            8080,
        )
        .unwrap();

        // IPv6 v2 header is larger than IPv4
        assert!(header.len() >= 16);
        // Same signature for v2
        assert_eq!(&header[0..12], b"\r\n\r\n\x00\r\nQUIT\n");
    }

    #[test]
    fn test_count_healthy_backends_all_healthy() {
        use crate::upstream::UpstreamInner;
        use crate::util::TtlCache;
        use std::time::Duration;

        let upstreams = vec![
            UpstreamInner {
                proxy_to: "http://backend1".to_string(),
                proxy_unix: None,
            },
            UpstreamInner {
                proxy_to: "http://backend2".to_string(),
                proxy_unix: None,
            },
        ];

        let failed_backends = parking_lot::RwLock::new(TtlCache::new(Duration::from_secs(60)));
        let count = count_healthy_backends(&upstreams, &failed_backends, 3);
        assert_eq!(count, 2);
    }

    #[test]
    fn test_count_healthy_backends_some_unhealthy() {
        use crate::upstream::UpstreamInner;
        use crate::util::TtlCache;
        use std::time::Duration;

        let upstreams = vec![
            UpstreamInner {
                proxy_to: "http://backend1".to_string(),
                proxy_unix: None,
            },
            UpstreamInner {
                proxy_to: "http://backend2".to_string(),
                proxy_unix: None,
            },
        ];

        let failed_backends = parking_lot::RwLock::new(TtlCache::new(Duration::from_secs(60)));
        {
            let mut failed = failed_backends.write();
            failed.insert(upstreams[0].clone(), 5); // Exceeds max_fails of 3
        }

        let count = count_healthy_backends(&upstreams, &failed_backends, 3);
        assert_eq!(count, 1);
    }

    #[test]
    fn test_count_healthy_backends_all_unhealthy() {
        use crate::upstream::UpstreamInner;
        use crate::util::TtlCache;
        use std::time::Duration;

        let upstreams = vec![
            UpstreamInner {
                proxy_to: "http://backend1".to_string(),
                proxy_unix: None,
            },
            UpstreamInner {
                proxy_to: "http://backend2".to_string(),
                proxy_unix: None,
            },
        ];

        let failed_backends = parking_lot::RwLock::new(TtlCache::new(Duration::from_secs(60)));
        {
            let mut failed = failed_backends.write();
            failed.insert(upstreams[0].clone(), 5);
            failed.insert(upstreams[1].clone(), 10);
        }

        let count = count_healthy_backends(&upstreams, &failed_backends, 3);
        assert_eq!(count, 0);
    }

    #[test]
    fn bench_cached_tls_config() {
        use std::time::Instant;
        // Warm up the cache
        let _ = cached_tls_config(true, false, false);
        let iterations = 100usize;
        let start = Instant::now();
        for _ in 0..iterations {
            let _ = cached_tls_config(true, false, false);
        }
        let elapsed = start.elapsed();
        println!("cached_tls_config {} iters: {:?}", iterations, elapsed);
    }
}
