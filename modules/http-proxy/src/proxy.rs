//! Core proxy logic: request transformation, TLS, connection establishment, and forwarding.

use std::cell::UnsafeCell;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use ferron_http::{HttpContext, HttpResponse};
use ferron_observability::{Event, LogEvent, LogLevel};
use http::header::{HeaderName, HeaderValue};
use http::{Request, Response, StatusCode};
use http_body_util::{BodyExt, Empty};
use hyper::body::Incoming;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};
use rustls_platform_verifier::BuilderVerifierExt;
use tokio_rustls::TlsConnector;
use vibeio_hyper::VibeioIo;

use crate::config::ProxyConfig;
use crate::connections::{ConnectionManager, PoolKey};
use crate::send_net_io::{SendTcpStreamPoll, SendUnixStreamPoll};
use crate::send_request::{
    http1_handshake, http1_handshake_unix, http2_handshake, http2_handshake_unix, ProxyBody,
    SendRequestWrapper, TrackedBody,
};
use crate::upstream::{
    determine_proxy_to, mark_backend_failure, resolve_upstreams, ConnectionsTrackState,
    LoadBalancerAlgorithmInner, UpstreamInner,
};
use crate::util::TtlCache;
use crate::ProxyMetrics;

const LOG_TARGET: &str = "ferron-proxy";

/// Check whether `client_ip_from_header` is configured.
fn client_ip_from_header_enabled(ctx: &HttpContext) -> bool {
    ctx.configuration
        .get_value("client_ip_from_header", false)
        .and_then(|v| v.as_str())
        .is_some()
}

/// Construct proxy request with header transformations.
fn construct_proxy_request(
    ctx: &mut HttpContext,
    config: &ProxyConfig,
    proxy_request_url: &http::Uri,
) -> Result<Request<ProxyBody>, Box<dyn std::error::Error + Send + Sync>> {
    let req = ctx.req.take().ok_or("no request in context")?;
    let (mut parts, body) = req.into_parts();

    let mut uri_parts = proxy_request_url.clone().into_parts();
    if let Some(pq) = parts.uri.path_and_query() {
        uri_parts.path_and_query = Some(pq.clone());
    }
    parts.uri = http::Uri::from_parts(uri_parts)?;

    for name in &config.headers_to_remove {
        parts.headers.remove(name);
    }
    for (name, value) in &config.headers_to_replace {
        parts.headers.remove(name);
        parts.headers.insert(
            name.clone(),
            HeaderValue::from_str(value).map_err(|e| format!("Invalid header value: {e}"))?,
        );
    }
    for (name, value) in &config.headers_to_add {
        parts.headers.append(
            name.clone(),
            HeaderValue::from_str(value).map_err(|e| format!("Invalid header value: {e}"))?,
        );
    }

    let client_ip = ctx.remote_address.ip();
    let proto = if ctx.encrypted { "https" } else { "http" };

    if client_ip_from_header_enabled(ctx) {
        append_x_forwarded_for(&mut parts.headers, client_ip);
        append_forwarded(&mut parts.headers, client_ip, proto, ctx.local_address.ip());
    } else {
        set_x_forwarded_for(&mut parts.headers, client_ip);
        set_forwarded(&mut parts.headers, client_ip, proto, ctx.local_address.ip());
    }

    parts.headers.insert(
        HeaderName::from_static("x-forwarded-proto"),
        HeaderValue::from_static(proto),
    );
    parts.headers.insert(
        HeaderName::from_static("x-real-ip"),
        HeaderValue::from_str(&client_ip.to_string())?,
    );

    Ok(Request::from_parts(parts, body))
}

fn set_x_forwarded_for(headers: &mut http::HeaderMap, client_ip: std::net::IpAddr) {
    if let Ok(hv) = HeaderValue::from_str(&client_ip.to_string()) {
        headers.insert("x-forwarded-for", hv);
    }
}

fn append_x_forwarded_for(headers: &mut http::HeaderMap, client_ip: std::net::IpAddr) {
    let client_ip_str = client_ip.to_string();
    if let Some(existing) = headers.get("x-forwarded-for") {
        if let Ok(existing_str) = existing.to_str() {
            let new_value = format!("{}, {}", existing_str, client_ip_str);
            if let Ok(hv) = HeaderValue::from_str(&new_value) {
                headers.insert("x-forwarded-for", hv);
                return;
            }
        }
    }
    if let Ok(hv) = HeaderValue::from_str(&client_ip_str) {
        headers.insert("x-forwarded-for", hv);
    }
}

fn set_forwarded(
    headers: &mut http::HeaderMap,
    client_ip: std::net::IpAddr,
    proto: &'static str,
    local_ip: std::net::IpAddr,
) {
    let element = build_forwarded_element(client_ip, proto, local_ip);
    if let Ok(hv) = HeaderValue::from_str(&element) {
        headers.insert("forwarded", hv);
    }
}

fn append_forwarded(
    headers: &mut http::HeaderMap,
    client_ip: std::net::IpAddr,
    proto: &'static str,
    local_ip: std::net::IpAddr,
) {
    let element = build_forwarded_element(client_ip, proto, local_ip);
    if let Some(existing) = headers.get("forwarded") {
        if let Ok(existing_str) = existing.to_str() {
            let new_value = format!("{}, {}", existing_str, element);
            if let Ok(hv) = HeaderValue::from_str(&new_value) {
                headers.insert("forwarded", hv);
                return;
            }
        }
    }
    if let Ok(hv) = HeaderValue::from_str(&element) {
        headers.insert("forwarded", hv);
    }
}

fn build_forwarded_element(
    client_ip: std::net::IpAddr,
    proto: &str,
    local_ip: std::net::IpAddr,
) -> String {
    let for_value = if client_ip.is_ipv6() {
        format!("\"[{}]\"", client_ip)
    } else {
        client_ip.to_string()
    };
    let by_value = if local_ip.is_ipv6() {
        format!("\"[{}]\"", local_ip)
    } else {
        local_ip.to_string()
    };
    format!("for={};proto={};by={}", for_value, proto, by_value)
}

fn build_tls_config(http2: bool, http2_only: bool, no_verification: bool) -> ClientConfig {
    let builder = rustls::ClientConfig::builder();
    let mut tls_client_config = if no_verification {
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoServerVerifier))
    } else {
        BuilderVerifierExt::with_platform_verifier(builder)
    }
    .with_no_client_auth();

    if http2_only {
        tls_client_config.alpn_protocols = vec![b"h2".to_vec()];
    } else if http2 {
        tls_client_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    } else {
        tls_client_config.alpn_protocols = vec![b"http/1.1".to_vec()];
    }

    tls_client_config
}

#[derive(Debug)]
struct NoServerVerifier;

impl ServerCertVerifier for NoServerVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::aws_lc_rs::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

pub fn io_error_status(err: &std::io::Error) -> (StatusCode, &'static str) {
    match err.kind() {
        std::io::ErrorKind::ConnectionRefused
        | std::io::ErrorKind::NotFound
        | std::io::ErrorKind::HostUnreachable => {
            (StatusCode::SERVICE_UNAVAILABLE, "Service unavailable")
        }
        std::io::ErrorKind::TimedOut => (StatusCode::GATEWAY_TIMEOUT, "Gateway timeout"),
        _ => (StatusCode::BAD_GATEWAY, "Bad gateway"),
    }
}

fn select_pool<'a>(
    cm: &'a ConnectionManager,
    upstream: &UpstreamInner,
) -> &'a connpool::Pool<PoolKey, SendRequestWrapper> {
    #[cfg(unix)]
    if upstream.proxy_unix.is_some() {
        return cm.unix_connections();
    }
    cm.connections()
}

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
pub async fn execute_proxy(
    ctx: &mut HttpContext,
    config: &ProxyConfig,
    cm: &ConnectionManager,
    failed_backends: Arc<tokio::sync::RwLock<TtlCache<UpstreamInner, u64>>>,
    algorithm: &LoadBalancerAlgorithmInner,
    conn_state: Option<&ConnectionsTrackState>,
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
        }));
        return Ok((HttpResponse::BuiltinError(502, None), metrics));
    }

    // Select upstream via load balancing (tracker already initialized inside)
    let Some(selected) = determine_proxy_to(
        &upstreams,
        &failed_backends,
        config.lb_health_check,
        config.lb_health_check_max_fails,
        algorithm,
        conn_state,
    )
    .await
    else {
        ctx.events.emit(Event::Log(LogEvent {
            level: LogLevel::Error,
            message: "Reverse proxy: all upstream backends are unhealthy".to_string(),
            target: LOG_TARGET,
        }));
        return Ok((HttpResponse::BuiltinError(503, None), metrics));
    };

    metrics.selected_backends.push(selected.upstream.clone());

    let proxy_request_url: http::Uri = selected
        .upstream
        .proxy_to
        .parse()
        .map_err(|e| format!("Invalid upstream URL '{}': {e}", selected.upstream.proxy_to))?;
    let is_https = proxy_request_url.scheme_str() == Some("https");
    let client_ip = config.proxy_header.map(|_| ctx.remote_address.ip());
    let local_limit_idx = cm.get_local_limit(&selected.upstream).await;
    let idle_timeout = idle_timeout_for_upstream(config, &selected.upstream);

    match try_send_with_pool(
        ctx,
        config,
        cm,
        &selected.upstream,
        &proxy_request_url,
        client_ip,
        local_limit_idx,
        idle_timeout,
        is_https,
        conn_state,
        selected.tracker,
        &mut metrics,
    )
    .await
    {
        Ok(resp) => Ok((resp, metrics)),
        Err(e) => {
            mark_backend_failure(
                Arc::clone(&failed_backends),
                config.lb_health_check,
                &selected.upstream,
                &mut metrics,
            )
            .await;
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
            }));
            Ok((HttpResponse::BuiltinError(status.as_u16(), None), metrics))
        }
    }
}

/// Try to send a request using the connection pool with racing of
/// non-ready pooled connections against a newly established connection.
///
/// When pooled connections are not ready (but alive), they are collected
/// and raced against establishing a brand-new connection, avoiding the
/// cost of unnecessary duplicate connection establishments.
async fn try_send_with_pool(
    ctx: &mut HttpContext,
    config: &ProxyConfig,
    cm: &ConnectionManager,
    upstream: &UpstreamInner,
    proxy_url: &http::Uri,
    client_ip: Option<IpAddr>,
    local_limit_idx: Option<usize>,
    idle_timeout: Duration,
    is_https: bool,
    _conn_state: Option<&ConnectionsTrackState>,
    tracked_connection: Option<Arc<()>>,
    metrics: &mut ProxyMetrics,
) -> Result<HttpResponse, Box<dyn std::error::Error + Send + Sync>> {
    let pool_key = (upstream.clone(), client_ip);
    let pool = select_pool(cm, upstream);

    // Collect non-ready-but-alive connections for racing
    let mut pending_items: Vec<connpool::Item<PoolKey, SendRequestWrapper>> = Vec::new();

    // Pull one connection from the pool and check readiness
    let mut item = if let Some(idx) = local_limit_idx {
        pool.pull_with_wait_local_limit(pool_key.clone(), Some(idx))
            .await
    } else {
        pool.pull(pool_key.clone()).await
    };

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
        )
        .await;
    }

    if should_keep {
        // Connection is alive but not ready — collect for racing
        if item.inner().is_some() {
            pending_items.push(item);
        }
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
        local_limit_idx,
        is_https,
        _conn_state,
        tracked_connection,
    )
    .await
}

/// Wait for any pending connection to become ready.
///
/// Returns the item if one becomes ready, or `None` if all fail.
async fn wait_for_any_ready(
    pending_items: &mut Vec<connpool::Item<PoolKey, SendRequestWrapper>>,
    idle_timeout: Duration,
) -> Option<connpool::Item<PoolKey, SendRequestWrapper>> {
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

/// Establish a new connection and send the request.
async fn establish_and_send(
    ctx: &mut HttpContext,
    config: &ProxyConfig,
    cm: &ConnectionManager,
    upstream: &UpstreamInner,
    proxy_url: &http::Uri,
    client_ip: Option<IpAddr>,
    local_limit_idx: Option<usize>,
    is_https: bool,
    _conn_state: Option<&ConnectionsTrackState>,
    tracked_connection: Option<Arc<()>>,
) -> Result<HttpResponse, Box<dyn std::error::Error + Send + Sync>> {
    let pool_key = (upstream.clone(), client_ip);
    let pool = select_pool(cm, upstream);

    let mut item = if let Some(idx) = local_limit_idx {
        pool.pull_with_wait_local_limit(pool_key.clone(), Some(idx))
            .await
    } else {
        pool.pull(pool_key.clone()).await
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

            if config.http2_only || config.http2 {
                http2_handshake_unix(stream, drop_guard).await?
            } else {
                http1_handshake_unix(stream, drop_guard).await?
            }
        }
        #[cfg(not(unix))]
        unreachable!()
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
            }));
            std::io::Error::other(format!("Connect failed: {e}"))
        })?;
        let mut stream = SendTcpStreamPoll::new_comp_io(tcp)
            .map_err(|e| std::io::Error::other(format!("Wrap failed: {e}")))?;

        let drop_guard = unsafe { stream.get_drop_guard() };

        if is_https {
            let tls_config =
                build_tls_config(config.http2, config.http2_only, config.no_verification);
            let connector = TlsConnector::from(Arc::new(tls_config));
            let domain = ServerName::try_from(host.to_string())
                .map_err(|e| format!("Invalid server name: {e}"))?;
            let tls_stream = connector.connect(domain, stream).await.map_err(|e| {
                ctx.events.emit(Event::Log(LogEvent {
                    level: LogLevel::Warn,
                    message: format!("Reverse proxy: TLS handshake with {addr} failed: {e}"),
                    target: LOG_TARGET,
                }));
                std::io::Error::other(format!("TLS handshake failed: {e}"))
            })?;

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

    send_via_wrapper(
        ctx,
        config,
        wrapper,
        item,
        proxy_url,
        tracked_connection,
        config.keepalive,
    )
    .await
}

/// Send request via a SendRequestWrapper and handle the response.
async fn send_via_wrapper(
    ctx: &mut HttpContext,
    config: &ProxyConfig,
    mut wrapper: SendRequestWrapper,
    item: connpool::Item<PoolKey, SendRequestWrapper>,
    proxy_url: &http::Uri,
    tracked_connection: Option<Arc<()>>,
    enable_keepalive: bool,
) -> Result<HttpResponse, Box<dyn std::error::Error + Send + Sync>> {
    let request = construct_proxy_request(ctx, config, proxy_url)?;

    let response = match wrapper.send_request(request).await {
        Ok(resp) => resp,
        Err(e) => {
            return Err(format!("Bad gateway: {e}").into());
        }
    };

    let status = response.status();

    // Handle HTTP 101 Switching Protocols (upgrades)
    if status == StatusCode::SWITCHING_PROTOCOLS {
        handle_upgrade(response, ctx, item).await?;
        return Ok(HttpResponse::BuiltinError(101, None));
    }

    let (parts, body) = response.into_parts();

    #[allow(clippy::arc_with_non_send_sync)]
    let pool_item_arc = if enable_keepalive && !wrapper.is_closed() {
        let pool_item_unsafe = Arc::new(UnsafeCell::new(item));
        let pool_item_clone = Arc::clone(&pool_item_unsafe);
        let pool_item_mut = unsafe { &mut *pool_item_unsafe.get() };
        pool_item_mut.inner_mut().replace(wrapper);
        Some(pool_item_clone)
    } else {
        None
    };

    let tracked_body = TrackedBody::new(
        body.map_err(std::io::Error::other),
        tracked_connection,
        pool_item_arc,
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

/// Handle HTTP 101 Switching Protocols (WebSocket upgrades).
async fn handle_upgrade(
    response: Response<Incoming>,
    ctx: &mut HttpContext,
    item: connpool::Item<PoolKey, SendRequestWrapper>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (resp_parts, _) = response.into_parts();
    let resp_for_upgrade = Response::from_parts(resp_parts.clone(), ());

    let upgrade_request = Request::builder()
        .method(
            ctx.req
                .as_ref()
                .map(|r| r.method().clone())
                .unwrap_or_default(),
        )
        .uri(
            ctx.req
                .as_ref()
                .map(|r| r.uri().clone())
                .unwrap_or_default(),
        )
        .version(ctx.req.as_ref().map(|r| r.version()).unwrap_or_default())
        .body(Empty::<Bytes>::new().map_err(|e| match e {}).boxed_unsync())
        .map_err(|e| format!("Failed to build upgrade request: {e}"))?;

    let events = ctx.events.clone();

    #[allow(clippy::arc_with_non_send_sync)]
    let pool_item = Arc::new(UnsafeCell::new(item));
    let pool_item_for_task = Arc::clone(&pool_item);

    vibeio::spawn(async move {
        match hyper::upgrade::on(resp_for_upgrade).await {
            Ok(upgraded_backend) => {
                let mut upgrade_request = upgrade_request;
                let upgrade_future = vibeio_http::prepare_upgrade(&mut upgrade_request);

                if let Some(upgraded_future) = upgrade_future {
                    match upgraded_future.await {
                        Some(upgraded_client) => {
                            let mut backend = VibeioIo::new(upgraded_backend);
                            let mut client = upgraded_client;

                            let _ = tokio::io::copy_bidirectional(&mut backend, &mut client).await;
                            drop(pool_item_for_task);
                        }
                        None => {
                            events.emit(Event::Log(LogEvent {
                                level: LogLevel::Warn,
                                message: "Reverse proxy: frontend HTTP upgrade failed".to_string(),
                                target: LOG_TARGET,
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
                }));
            }
        }
    });

    Ok(())
}

#[allow(dead_code)]
pub fn error_response(status: StatusCode) -> HttpResponse {
    use http_body_util::Full;
    HttpResponse::Custom(
        Response::builder()
            .status(status)
            .body(
                Full::new(Bytes::from(format!(
                    "<html><body><h1>{status}</h1></body></html>"
                )))
                .map_err(|e| match e {})
                .boxed_unsync(),
            )
            .expect("Failed to build error response"),
    )
}
