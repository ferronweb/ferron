//! Connection pool management logic.

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use futures_util::future::select_ok;
use http_body_util::BodyExt;
use rustls::pki_types::ServerName;
use tokio_rustls::TlsConnector;
use vibeio_hyper::VibeioIo;

use crate::config::ProxyConfig;
use crate::connections::{ConnectionManager, PooledConnection};
use crate::proxy::cached_tls_config;
use crate::proxy::connect::build_proxy_protocol_header;
use crate::send_net_io::SendTcpStreamPoll;
#[cfg(unix)]
use crate::send_net_io::SendUnixStreamPoll;
use crate::send_request::{http1_handshake, http2_handshake, SendRequestWrapper, TrackedBody};
#[cfg(unix)]
use crate::send_request::{http1_handshake_unix, http2_handshake_unix};
use crate::types::upstream::UpstreamInner;
use crate::types::ConnectionsTrackState;
use crate::ProxyMetrics;
use ferron_http::HttpContext;

/// Try to send a request using the connection pool with racing of
/// non-ready pooled connections against a newly established connection.
///
/// When pooled connections are not ready (but alive), they are collected
/// and raced against establishing a brand-new connection, avoiding the
/// cost of unnecessary duplicate connection establishments.
#[allow(clippy::too_many_arguments)]
pub async fn try_send_with_pool(
    ctx: &mut HttpContext,
    config: &ProxyConfig,
    cm: &ConnectionManager,
    upstream: Arc<UpstreamInner>,
    proxy_url: &http::Uri,
    client_ip: Option<IpAddr>,
    local_limit: Option<usize>,
    idle_timeout: Duration,
    is_https: bool,
    _conn_state: Option<&ConnectionsTrackState>,
    tracked_connection: Option<Arc<()>>,
    metrics: &mut ProxyMetrics,
) -> Result<ferron_http::HttpResponse, Box<dyn std::error::Error + Send + Sync>> {
    // Collect non-ready-but-alive connections for racing
    let mut pending_items: Vec<PooledConnection> = Vec::new();
    // Track a non-ready-but-kept item slot for reuse in establish_and_send
    // (avoids double-pull when the connection is dead and can't be raced).
    let mut reusable_item: Option<PooledConnection> = None;

    // Pull one connection from the pool and check readiness
    let pull_start = std::time::Instant::now();
    let item = if let Some(limit) = local_limit {
        cm.pull_with_local_limit(upstream.clone(), client_ip, Some(limit))
    } else {
        cm.pull(upstream.clone(), client_ip)
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
    pending_items: &mut Vec<PooledConnection>,
    idle_timeout: Duration,
) -> Option<PooledConnection> {
    if pending_items.is_empty() {
        return None;
    }

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

/// Establish a new connection and send the request.
///
/// If `existing_item` is provided, it is reused instead of pulling a new one
/// from the pool, avoiding a double semaphore acquisition.
#[allow(clippy::too_many_arguments)]
pub async fn establish_and_send(
    ctx: &mut HttpContext,
    config: &ProxyConfig,
    cm: &ConnectionManager,
    upstream: Arc<UpstreamInner>,
    proxy_url: &http::Uri,
    client_ip: Option<IpAddr>,
    local_limit: Option<usize>,
    is_https: bool,
    _conn_state: Option<&ConnectionsTrackState>,
    tracked_connection: Option<Arc<()>>,
    existing_item: Option<PooledConnection>,
    metrics: &mut ProxyMetrics,
) -> Result<ferron_http::HttpResponse, Box<dyn std::error::Error + Send + Sync>> {
    let item: Option<PooledConnection> = if let Some(it) = existing_item {
        Some(it)
    } else if let Some(limit) = local_limit {
        cm.pull_with_local_limit(upstream.clone(), client_ip, Some(limit))
    } else {
        cm.pull(upstream.clone(), client_ip)
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
            let unix = vibeio::net::PollUnixStream::connect(unix_path)
                .await
                .map_err(|e| std::io::Error::other(format!("Unix connect failed: {e}")))?;
            let mut stream = SendUnixStreamPoll::new(unix);

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
                        ctx.events.emit(ferron_observability::Event::Log(
                            ferron_observability::LogEvent {
                                level: ferron_observability::LogLevel::Warn,
                                message: format!(
                                    "Reverse proxy: TLS handshake with {unix_path} failed: {e}"
                                ),
                                target: "ferron-http-proxy",
                                trace_context:
                                    ferron_http::trace_context::current_event_trace_context(ctx),
                            },
                        ));
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
            } else if config.http2_only || config.http2 {
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

        let tcp = vibeio::net::PollTcpStream::connect(&addr)
            .await
            .map_err(|e| {
                ctx.events.emit(ferron_observability::Event::Log(
                    ferron_observability::LogEvent {
                        level: ferron_observability::LogLevel::Warn,
                        message: format!("Reverse proxy: TCP connect to {addr} failed: {e}"),
                        target: "ferron-http-proxy",
                        trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
                    },
                ));
                std::io::Error::other(format!("Connect failed: {e}"))
            })?;
        let mut stream = SendTcpStreamPoll::new(tcp);

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
                    ctx.events.emit(ferron_observability::Event::Log(
                        ferron_observability::LogEvent {
                            level: ferron_observability::LogLevel::Warn,
                            message: format!(
                                "Reverse proxy: TLS handshake with {addr} failed: {e}"
                            ),
                            target: "ferron-http-proxy",
                            trace_context: ferron_http::trace_context::current_event_trace_context(
                                ctx,
                            ),
                        },
                    ));
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
        } else if config.http2_only || config.http2 {
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

/// Establish a connection without pool tracking.
///
/// This is used when the pool is at capacity and we need to establish
/// a connection without waiting for a pool slot.
#[allow(clippy::too_many_arguments)]
pub async fn establish_connection_without_pool(
    ctx: &mut HttpContext,
    config: &ProxyConfig,
    upstream: Arc<UpstreamInner>,
    proxy_url: &http::Uri,
    client_ip: Option<IpAddr>,
    is_https: bool,
    _conn_state: Option<&ConnectionsTrackState>,
    tracked_connection: Option<Arc<()>>,
    metrics: &mut ProxyMetrics,
) -> Result<ferron_http::HttpResponse, Box<dyn std::error::Error + Send + Sync>> {
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
            let unix = vibeio::net::PollUnixStream::connect(unix_path)
                .await
                .map_err(|e| std::io::Error::other(format!("Unix connect failed: {e}")))?;
            let mut stream = SendUnixStreamPoll::new(unix);

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

        let tcp = vibeio::net::PollTcpStream::connect(&addr)
            .await
            .map_err(|e| {
                ctx.events.emit(ferron_observability::Event::Log(
                    ferron_observability::LogEvent {
                        level: ferron_observability::LogLevel::Warn,
                        message: format!("Reverse proxy: TCP connect to {addr} failed: {e}"),
                        target: "ferron-http-proxy",
                        trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
                    },
                ));
                std::io::Error::other(format!("Connect failed: {e}"))
            })?;
        let mut stream = SendTcpStreamPoll::new(tcp);

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
                    ctx.events.emit(ferron_observability::Event::Log(
                        ferron_observability::LogEvent {
                            level: ferron_observability::LogLevel::Warn,
                            message: format!(
                                "Reverse proxy: TLS handshake with {addr} failed: {e}"
                            ),
                            target: "ferron-http-proxy",
                            trace_context: ferron_http::trace_context::current_event_trace_context(
                                ctx,
                            ),
                        },
                    ));
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
pub async fn send_request_without_pool_item(
    ctx: &mut HttpContext,
    config: &ProxyConfig,
    mut wrapper: SendRequestWrapper,
    proxy_url: &http::Uri,
    _tracked_connection: Option<Arc<()>>,
    _enable_keepalive: bool,
    metrics: &mut ProxyMetrics,
) -> Result<ferron_http::HttpResponse, Box<dyn std::error::Error + Send + Sync>> {
    let request = crate::proxy::request::construct_proxy_request(ctx, config, proxy_url)?;

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

    let mut builder = http::Response::builder().status(parts.status);
    for (name, value) in parts.headers {
        if let Some(n) = name {
            builder = builder.header(n, value);
        }
    }
    let response = builder
        .body(tracked_body.boxed_unsync())
        .expect("Failed to build response");

    Ok(ferron_http::HttpResponse::Custom(response))
}

/// Send request via a SendRequestWrapper and handle the response.
#[allow(clippy::too_many_arguments)]
pub async fn send_via_wrapper(
    ctx: &mut HttpContext,
    config: &ProxyConfig,
    mut wrapper: SendRequestWrapper,
    item: PooledConnection,
    proxy_url: &http::Uri,
    tracked_connection: Option<Arc<()>>,
    enable_keepalive: bool,
    is_unix: bool,
    _local_limit: Option<usize>,
    metrics: &mut ProxyMetrics,
) -> Result<ferron_http::HttpResponse, Box<dyn std::error::Error + Send + Sync>> {
    let request = crate::proxy::request::construct_proxy_request(ctx, config, proxy_url)?;
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
    if status == http::StatusCode::SWITCHING_PROTOCOLS {
        let response_upgrade = http::Response::from_parts(parts.clone(), ());
        handle_upgrade(response_upgrade, extensions, ctx, item).await?;

        // Remove some response headers as indicated by "Connection" header (RFC 7230)
        crate::proxy::response::remove_headers_rfc7230(&mut parts);

        Ok(ferron_http::HttpResponse::Custom(
            http::Response::from_parts(parts, body.map_err(std::io::Error::other).boxed_unsync()),
        ))
    } else if config.intercept_errors && status.as_u16() >= 400 {
        // Intercept upstream error responses if configured.
        // When intercept_errors is true, upstream 4xx/5xx responses
        // are replaced with Ferron's built-in error response.
        // When intercept_errors is false (default), the full upstream response is passed through.
        Ok(ferron_http::HttpResponse::BuiltinError(
            status.as_u16(),
            None,
        ))
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
        crate::proxy::response::remove_headers_rfc7230(&mut parts);

        let mut response = http::Response::from_parts(parts, tracked_body.boxed_unsync());
        *response.version_mut() = http::Version::default();

        Ok(ferron_http::HttpResponse::Custom(response))
    }
}

/// Handle HTTP 101 Switching Protocols (WebSocket upgrades).
pub async fn handle_upgrade(
    resp_for_upgrade: http::Response<()>,
    req_extensions: http::Extensions,
    ctx: &mut HttpContext,
    mut item: PooledConnection,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut upgrade_request = http::Request::new(http_body_util::Empty::<bytes::Bytes>::new());
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
                            events.emit(ferron_observability::Event::Log(
                                ferron_observability::LogEvent {
                                    level: ferron_observability::LogLevel::Warn,
                                    message: "Reverse proxy: backend HTTP upgrade failed"
                                        .to_string(),
                                    target: "ferron-http-proxy",
                                    trace_context: None,
                                },
                            ));
                        }
                    }
                }
            }
            Err(_) => {
                events.emit(ferron_observability::Event::Log(
                    ferron_observability::LogEvent {
                        level: ferron_observability::LogLevel::Warn,
                        message: "Reverse proxy: frontend HTTP upgrade failed".to_string(),
                        target: "ferron-http-proxy",
                        trace_context: None,
                    },
                ));
            }
        }
    });

    Ok(())
}
