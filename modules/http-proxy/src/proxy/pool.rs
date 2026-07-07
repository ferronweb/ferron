//! Connection pool management logic.

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use ferron_http::trace_context::current_event_trace_context;
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
use crate::send_request::{
    http1_handshake, http2_handshake, BodyTrackingState, ContentLengthTrackingBody,
    SendRequestWrapper, TrackedBody, TruncatedTracker,
};
#[cfg(unix)]
use crate::send_request::{http1_handshake_unix, http2_handshake_unix};
use crate::types::error::ProxyError;
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
) -> Result<ferron_http::HttpResponse, ProxyError> {
    // Collect non-ready-but-alive connection for racing
    let mut pending_items: Vec<PooledConnection> = Vec::new();
    // Track a non-ready-but-kept item slot for reuse in establish_and_send
    // (avoids double-pull when the connection is dead and can't be raced).
    let mut reusable_item: Option<PooledConnection> = None;

    // Pull one connection from the pool and check readiness
    let mut pull_start = None;
    let item_fut = async {
        if let Some(limit) = local_limit {
            cm.pull_with_local_limit(upstream.clone(), client_ip, Some(limit), idle_timeout)
                .await
        } else {
            cm.pull(upstream.clone(), client_ip, idle_timeout).await
        }
    };
    let pull_start_set_fut = async {
        if pull_start.is_none() {
            pull_start = Some(std::time::Instant::now());
        }
        std::future::pending().await
    };
    let mut item = tokio::select! {
        biased;
        item = item_fut => item,
        item_mock = pull_start_set_fut => item_mock,
    };

    // Track pool wait metrics
    if let Some(pull_duration) = pull_start.map(|d| d.elapsed().as_secs_f64()) {
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
        metrics.pool_hit = true;
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
                metrics.pool_hit = true;
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
        idle_timeout,
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
    idle_timeout: Duration,
) -> Result<ferron_http::HttpResponse, ProxyError> {
    metrics.pool_miss = true;
    let mut item: PooledConnection = if let Some(it) = existing_item {
        it
    } else if let Some(limit) = local_limit {
        cm.pull_with_local_limit(upstream.clone(), client_ip, Some(limit), idle_timeout)
            .await
    } else {
        cm.pull(upstream.clone(), client_ip, idle_timeout).await
    };

    *item.inner_mut() = None;

    let connect_start = std::time::Instant::now();

    #[cfg(unix)]
    let is_unix = upstream.proxy_unix.is_some();
    #[cfg(not(unix))]
    let is_unix = false;

    let wrapper_fut = async {
        if is_unix {
            #[cfg(unix)]
            {
                let unix_path = upstream
                    .proxy_unix
                    .as_ref()
                    .ok_or("Unix socket path not set")?;
                let unix = match vibeio::net::PollUnixStream::connect(unix_path).await {
                    Ok(s) => s,
                    Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
                        return Err(ProxyError::Timeout(format!("Unix connect failed: {e}")));
                    }

                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::ConnectionAborted
                                | std::io::ErrorKind::NotFound
                                | std::io::ErrorKind::HostUnreachable
                        ) =>
                    {
                        return Err(ProxyError::ConnectFailedUnavailable(format!(
                            "Unix connect failed: {e}"
                        )));
                    }
                    Err(e) => {
                        return Err(ProxyError::ConnectFailed(format!(
                            "Unix connect failed: {e}"
                        )));
                    }
                };
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
                            ProxyError::ProxyProtocolWriteFailed(format!(
                                "PROXY header write failed: {e}"
                            ))
                        })?;
                    }
                }

                if is_https {
                    let connector = TlsConnector::from(cached_tls_config(
                        config.http2,
                        config.http2_only,
                        config.no_verification,
                        upstream.mtls.clone(),
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
                                summary: "Reverse proxy: TLS handshake with Unix socket failed"
                                    .into(),
                                target: "ferron-http-proxy",
                                attributes: vec![
                                    (
                                        "upstream.address",
                                        ferron_observability::LogAttributeValue::String(
                                            unix_path.to_string(),
                                        ),
                                    ),
                                    (
                                        "error.message",
                                        ferron_observability::LogAttributeValue::String(
                                            e.to_string(),
                                        ),
                                    ),
                                ],
                                trace_context:
                                    ferron_http::trace_context::current_event_trace_context(ctx),
                                control_plane_metadata: None,
                            },
                        ));
                            return Err(ProxyError::TlsHandshakeFailed(format!(
                                "TLS handshake failed: {e}"
                            )));
                        }
                    };

                    let negotiated_h2 = tls_stream.get_ref().1.alpn_protocol() == Some(b"h2");
                    let use_http2 = (config.http2 && negotiated_h2) || config.http2_only;

                    if use_http2 {
                        http2_handshake_unix(tls_stream, drop_guard).await
                    } else {
                        http1_handshake_unix(tls_stream, drop_guard).await
                    }
                } else if config.http2_only || config.http2 {
                    http2_handshake_unix(stream, drop_guard).await
                } else {
                    http1_handshake_unix(stream, drop_guard).await
                }
            }
            #[cfg(not(unix))]
            unreachable!();
        } else {
            // Use pre-resolved IP from connect_to if available, otherwise parse from proxy_url
            let addr = if let Some(ct) = &upstream.connect_to {
                ct.to_string()
            } else {
                let host = proxy_url.host().ok_or("upstream URL has no host")?;
                let port = proxy_url
                    .port_u16()
                    .unwrap_or(if is_https { 443 } else { 80 });
                format!("{host}:{port}")
            };

            // Extract hostname for TLS SNI (always use original hostname, not resolved IP)
            let sni_host = proxy_url
                .host()
                .ok_or("upstream URL has no host")?
                .to_string();

            let tcp = match vibeio::net::PollTcpStream::connect(&addr)
                .await
                .map_err(|e| {
                    ctx.events.emit(ferron_observability::Event::Log(
                        ferron_observability::LogEvent {
                            level: ferron_observability::LogLevel::Warn,
                            message: format!("Reverse proxy: TCP connect to {addr} failed: {e}"),
                            summary: "Reverse proxy: TCP connect to backend failed".into(),
                            target: "ferron-http-proxy",
                            attributes: vec![(
                                "upstream.address",
                                ferron_observability::LogAttributeValue::String(addr.clone()),
                            )],
                            trace_context: ferron_http::trace_context::current_event_trace_context(
                                ctx,
                            ),
                            control_plane_metadata: None,
                        },
                    ));
                    e
                }) {
                Ok(s) => s,
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
                    return Err(ProxyError::Timeout(format!("TCP connect failed: {e}")));
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::ConnectionAborted
                            | std::io::ErrorKind::NotFound
                            | std::io::ErrorKind::HostUnreachable
                    ) =>
                {
                    return Err(ProxyError::ConnectFailedUnavailable(format!(
                        "TCP connect failed: {e}"
                    )));
                }
                Err(e) => {
                    return Err(ProxyError::ConnectFailed(format!(
                        "TCP connect failed: {e}"
                    )));
                }
            };
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
                        ProxyError::ProxyProtocolWriteFailed(format!(
                            "PROXY header write failed: {e}"
                        ))
                    })?;
                }
            }

            if is_https {
                let connector = TlsConnector::from(cached_tls_config(
                    config.http2,
                    config.http2_only,
                    config.no_verification,
                    upstream.mtls.clone(),
                ));
                let domain = ServerName::try_from(sni_host)
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
                                summary: "Reverse proxy: TLS handshake with backend failed".into(),
                                target: "ferron-http-proxy",
                                attributes: vec![(
                                    "upstream.address",
                                    ferron_observability::LogAttributeValue::String(addr.clone()),
                                )],
                                trace_context:
                                    ferron_http::trace_context::current_event_trace_context(ctx),
                                control_plane_metadata: None,
                            },
                        ));
                        return Err(ProxyError::TlsHandshakeFailed(format!(
                            "TLS handshake failed: {e}"
                        )));
                    }
                };

                let negotiated_h2 = tls_stream.get_ref().1.alpn_protocol() == Some(b"h2");
                let use_http2 = (config.http2 && negotiated_h2) || config.http2_only;

                if use_http2 {
                    http2_handshake(tls_stream, drop_guard).await
                } else {
                    http1_handshake(tls_stream, drop_guard).await
                }
            } else if config.http2_only || config.http2 {
                http2_handshake(stream, drop_guard).await
            } else {
                http1_handshake(stream, drop_guard).await
            }
        }
    };
    let wrapper_result = if let Some(t) = upstream.connection_timeout {
        vibeio::time::timeout(t, wrapper_fut).await
    } else {
        Ok(wrapper_fut.await)
    };

    metrics.connect_time_secs += connect_start.elapsed().as_secs_f64();

    let wrapper = match wrapper_result {
        Ok(Ok(w)) => w,
        Ok(Err(e)) => return Err(e),
        Err(_) => {
            return Err(ProxyError::Timeout(
                "configured connection timeout elapsed".into(),
            ))
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
) -> Result<ferron_http::HttpResponse, ProxyError> {
    let request = crate::proxy::request::construct_proxy_request(ctx, config, proxy_url)?;
    let extensions = request.extensions().clone();

    let start = std::time::Instant::now();
    let response = match wrapper.send_request(request).await {
        Ok(resp) => {
            let elapsed = start.elapsed().as_secs_f64();
            metrics.upstream_time_secs = elapsed;
            metrics.ttfb_secs = elapsed;
            resp
        }
        Err(e) => {
            return Err(ProxyError::SendRequestError(format!("Bad gateway: {e}")));
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

        // Extract backend URL before consuming item
        let backend_url = item.key().map(|k| k.0.proxy_to.clone()).unwrap_or_default();

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

        // Extract Content-Length for truncation detection
        let expected_length = parts
            .headers
            .get(http::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok());

        let tracking_state = BodyTrackingState::new(expected_length);

        // Wrap body in ContentLengthTrackingBody for byte-level tracking
        let tracking_body = ContentLengthTrackingBody::new(body, tracking_state.clone());

        let truncated_tracker = TruncatedTracker::new(
            tracking_state,
            backend_url,
            ctx.events.clone(),
            ferron_http::trace_context::current_event_trace_context(ctx),
        );

        let tracked_body = TrackedBody::new(
            tracking_body.map_err(std::io::Error::other),
            tracked_connection,
            Some(truncated_tracker),
        );

        // Remove some response headers as indicated by "Connection" header (RFC 7230)
        crate::proxy::response::remove_headers_rfc7230(&mut parts);

        let mut response = http::Response::from_parts(parts, tracked_body.boxed_unsync());
        *response.version_mut() = http::Version::default();

        drop(pool_return_info);

        Ok(ferron_http::HttpResponse::Custom(response))
    }
}

/// Handle HTTP 101 Switching Protocols (WebSocket upgrades).
pub async fn handle_upgrade(
    resp_for_upgrade: http::Response<()>,
    req_extensions: http::Extensions,
    ctx: &mut HttpContext,
    mut item: PooledConnection,
) -> Result<(), ProxyError> {
    let mut upgrade_request = http::Request::new(http_body_util::Empty::<bytes::Bytes>::new());
    *upgrade_request.extensions_mut() = req_extensions;

    let events = ctx.events.clone();
    let upstream = item.key().map(|k| &*k.0);
    let trace_context = current_event_trace_context(ctx);
    let mut upstream_attrs = Vec::with_capacity(2);
    if let Some(backend) = upstream {
        upstream_attrs.push((
            "ferron.proxy.backend_url",
            ferron_observability::LogAttributeValue::String(backend.proxy_to.clone()),
        ));
        if let Some(ref unix_path) = backend.proxy_unix {
            upstream_attrs.push((
                "ferron.proxy.backend_unix_path",
                ferron_observability::LogAttributeValue::String(unix_path.clone()),
            ));
        }
    }

    // Take the inner value to prevent Drop from returning the connection to pool.
    // For upgrade connections, we don't return them to the pool
    // (upgrade connections are long-lived, not pooled).
    // Letting item drop naturally decrements the outstanding counter.
    let _wrapper = item.inner_mut().take();

    let upgrade_future = vibeio_http::prepare_upgrade(&mut upgrade_request);
    vibeio::spawn(async move {
        match hyper::upgrade::on(resp_for_upgrade).await {
            Ok(upgraded_backend) => {
                if let Some(upgraded_future) = upgrade_future {
                    match upgraded_future.await {
                        Some(upgraded_client) => {
                            let mut backend = VibeioIo::new(upgraded_backend);
                            let mut client = upgraded_client;

                            if let Err(e) =
                                tokio::io::copy_bidirectional(&mut backend, &mut client).await
                            {
                                let mut upstream_attrs = upstream_attrs;
                                upstream_attrs.push((
                                    "error.message",
                                    ferron_observability::LogAttributeValue::String(e.to_string()),
                                ));
                                events.emit(ferron_observability::Event::Log(
                                    ferron_observability::LogEvent {
                                        level: ferron_observability::LogLevel::Warn,
                                        message: format!(
                                            "Reverse proxy: HTTP upgrade tunneling failed: {}",
                                            e
                                        ),
                                        summary: "Reverse proxy: HTTP upgrade tunneling failed"
                                            .into(),
                                        target: "ferron-http-proxy",
                                        attributes: upstream_attrs,
                                        trace_context,
                                        control_plane_metadata: None,
                                    },
                                ));
                            }
                            // Connection not returned to pool for upgrades
                            // (upgrade connections are long-lived, not pooled)
                        }
                        None => {
                            events.emit(ferron_observability::Event::Log(
                                ferron_observability::LogEvent {
                                    level: ferron_observability::LogLevel::Warn,
                                    message: "Reverse proxy: backend HTTP upgrade failed"
                                        .to_string(),
                                    summary: "Reverse proxy: backend HTTP upgrade failed".into(),
                                    target: "ferron-http-proxy",
                                    attributes: upstream_attrs,
                                    trace_context,
                                    control_plane_metadata: None,
                                },
                            ));
                        }
                    }
                }
            }
            Err(e) => {
                let mut upstream_attrs = upstream_attrs;
                upstream_attrs.push((
                    "error.message",
                    ferron_observability::LogAttributeValue::String(e.to_string()),
                ));
                events.emit(ferron_observability::Event::Log(
                    ferron_observability::LogEvent {
                        level: ferron_observability::LogLevel::Warn,
                        message: format!("Reverse proxy: frontend HTTP upgrade failed: {e}"),
                        summary: "Reverse proxy: frontend HTTP upgrade failed".into(),
                        target: "ferron-http-proxy",
                        attributes: upstream_attrs,
                        trace_context,
                        control_plane_metadata: None,
                    },
                ));
            }
        }
    });

    Ok(())
}
