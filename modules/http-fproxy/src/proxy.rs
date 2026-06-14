//! Core forward proxy logic: CONNECT tunneling, HTTP forwarding, and ACL enforcement.

use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;

use bytes::Bytes;
use ferron_http::HttpContext;
use ferron_observability::{
    CompositeEventSink, Event, LogAttributeValue, LogEvent, LogLevel, MetricAttributeValue,
    MetricEvent, MetricType, MetricValue,
};
use http::{header, Request, Response, StatusCode, Uri};
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, Empty};
use tokio::io::{AsyncRead, AsyncWrite};
use vibeio::net::TcpStream;
use vibeio_hyper::VibeioIo;

use crate::config::{domain_matches, ip_denied, port_allowed, ForwardProxyConfig};
use crate::error::{ConnectErrorKind, ForwardErrorKind, ForwardProxyError};

const LOG_TARGET: &str = "ferron-http-fproxy";

/// Type alias for the HTTP request body used by Ferron's HTTP pipeline.
type HttpBody = UnsyncBoxBody<Bytes, std::io::Error>;

/// Result of a forward proxy operation.
pub enum ForwardProxyResult {
    /// Request was handled (response is set, pipeline should stop).
    Handled,
    /// Request is not a forward proxy request (pipeline should continue).
    PassThrough,
}

/// Execute forward proxy logic for an incoming request.
///
/// This function:
/// 1. Determines if the request is a forward proxy request (absolute URI or CONNECT)
/// 2. Evaluates ACLs (domain, port, IP)
/// 3. Executes the proxy (CONNECT tunneling or HTTP forwarding)
pub async fn execute_forward_proxy(
    ctx: &mut HttpContext,
    config: &ForwardProxyConfig,
) -> Result<ForwardProxyResult, ForwardProxyError> {
    let req = match ctx.req.take() {
        Some(req) => req,
        None => return Ok(ForwardProxyResult::PassThrough),
    };

    let is_connect = req.method() == hyper::Method::CONNECT;
    let is_proxy_request = is_connect || uri_has_host(req.uri());

    if !is_proxy_request {
        ctx.req = Some(req);
        return Ok(ForwardProxyResult::PassThrough);
    }

    // CONNECT handling
    if is_connect {
        if !config.connect_method {
            emit_error_log(ctx, &ForwardProxyError::ConnectDisabled);
            ctx.res = Some(ferron_http::HttpResponse::BuiltinError(403, None));
            emit_forward_proxy_metric(ctx, "connect", "connect_disabled", 403, None);
            return Ok(ForwardProxyResult::Handled);
        }
        return handle_connect(ctx, req, config).await;
    }

    // HTTP forwarding (absolute URI)
    handle_http_forward(ctx, req, config).await
}

/// Check if a URI has a host component (i.e., is an absolute URI for forward proxy).
fn uri_has_host(uri: &Uri) -> bool {
    uri.host().is_some()
}

/// Handle an HTTP CONNECT request by establishing a TCP tunnel.
async fn handle_connect(
    ctx: &mut HttpContext,
    request: Request<HttpBody>,
    config: &ForwardProxyConfig,
) -> Result<ForwardProxyResult, ForwardProxyError> {
    let connect_address = match request.uri().authority() {
        Some(auth) => auth.to_string(),
        None => {
            emit_error_log(ctx, &ForwardProxyError::BadConnectRequest);
            ctx.res = Some(ferron_http::HttpResponse::BuiltinError(400, None));
            emit_forward_proxy_metric(ctx, "connect", "bad_request", 400, None);
            return Ok(ForwardProxyResult::Handled);
        }
    };

    // Parse host and port
    let (host, port) = parse_host_port(&connect_address, 443)?;

    // ACL: check port
    if !port_allowed(&config.allow_ports, port) {
        let err = ForwardProxyError::PortDenied { port };
        emit_error_log(ctx, &err);
        ctx.res = Some(ferron_http::HttpResponse::BuiltinError(403, None));
        emit_forward_proxy_metric(ctx, "connect", "acl_denied", 403, None);
        return Ok(ForwardProxyResult::Handled);
    }

    // ACL: check domain
    if !domain_matches(&config.allow_domains, &host) {
        let err = ForwardProxyError::DomainDenied {
            domain: host.clone(),
        };
        emit_error_log(ctx, &err);
        ctx.res = Some(ferron_http::HttpResponse::BuiltinError(403, None));
        emit_forward_proxy_metric(ctx, "connect", "acl_denied", 403, None);
        return Ok(ForwardProxyResult::Handled);
    }

    // Resolve DNS and validate IP (fail if IP is denied)
    let Some(resolved_ips) = resolve_and_validate_ip(ctx, &host, &config.deny_ips).await? else {
        let err = ForwardProxyError::DnsUnresolved(host.clone());
        emit_error_log(ctx, &err);
        ctx.res = Some(ferron_http::HttpResponse::BuiltinError(403, None));
        emit_forward_proxy_metric(ctx, "connect", "dns_unresolved", 403, None);
        return Ok(ForwardProxyResult::Handled);
    };
    let socket_addrs = resolved_ips
        .into_iter()
        .map(|ip| SocketAddr::new(ip, port))
        .collect::<Vec<_>>();

    let error_logger = ctx.events.clone();
    let config = config.clone();
    let connect_address = connect_address.clone();
    let trace_context = ferron_http::trace_context::current_event_trace_context(ctx);

    // Prepare HTTP upgrade for the request
    let (request, upgrade_future) = {
        let mut req = request;
        let upgrade = vibeio_http::prepare_upgrade(&mut req);
        (req, upgrade)
    };

    // Spawn the tunnel
    vibeio::spawn(async move {
        // Wait for the upgrade
        let upgraded = match upgrade_future {
            Some(future) => match future.await {
                Some(upgraded) => upgraded,
                None => {
                    let err = ForwardProxyError::ConnectError {
                        target: connect_address.clone(),
                        kind: ConnectErrorKind::UpgradeFailed { detail: None },
                    };
                    emit_error_log_to_events(&error_logger, &err, trace_context.clone());
                    emit_forward_proxy_metric_to_events(
                        &error_logger,
                        "connect",
                        "upgrade_failed",
                        502,
                        Some("upgrade_failed".to_string()),
                        trace_context.clone(),
                    );
                    return;
                }
            },
            None => {
                let err = ForwardProxyError::ConnectError {
                    target: connect_address.clone(),
                    kind: ConnectErrorKind::UpgradeFailed { detail: None },
                };
                emit_error_log_to_events(&error_logger, &err, trace_context.clone());
                emit_forward_proxy_metric_to_events(
                    &error_logger,
                    "connect",
                    "upgrade_failed",
                    502,
                    Some("upgrade_failed".to_string()),
                    trace_context.clone(),
                );
                return;
            }
        };

        // Connect to the remote server
        let backend_stream = match TcpStream::connect(&*socket_addrs).await {
            Ok(stream) => stream,
            Err(err) => {
                let err = ForwardProxyError::ConnectError {
                    target: connect_address.clone(),
                    kind: ConnectErrorKind::ConnectFailed {
                        error: err.to_string(),
                    },
                };
                emit_error_log_to_events(&error_logger, &err, trace_context.clone());
                emit_forward_proxy_metric_to_events(
                    &error_logger,
                    "connect",
                    "backend_connect_error",
                    502,
                    Some("backend_connect_error".to_string()),
                    trace_context.clone(),
                );
                return;
            }
        };

        if let Err(err) = backend_stream.set_nodelay(true) {
            emit_log_to_events(
                &error_logger,
                LogLevel::Warn,
                "Forward proxy: cannot set TCP_NODELAY",
                "Forward proxy: cannot set TCP_NODELAY",
                vec![
                    (
                        "forward_proxy.target",
                        LogAttributeValue::String(connect_address.clone()),
                    ),
                    ("error.message", LogAttributeValue::String(err.to_string())),
                ],
                trace_context.clone(),
            );
        }

        let mut backend_stream = match backend_stream.into_poll() {
            Ok(stream) => stream,
            Err(err) => {
                let err = ForwardProxyError::ConnectError {
                    target: connect_address.clone(),
                    kind: ConnectErrorKind::ConnectFailed {
                        error: err.to_string(),
                    },
                };
                emit_error_log_to_events(&error_logger, &err, trace_context.clone());
                emit_forward_proxy_metric_to_events(
                    &error_logger,
                    "connect",
                    "backend_connect_error",
                    502,
                    Some("backend_connect_error".to_string()),
                    trace_context.clone(),
                );
                return;
            }
        };

        let mut upgraded = upgraded;

        // Bidirectional copy between client and backend
        match tokio::io::copy_bidirectional(&mut upgraded, &mut backend_stream).await {
            Ok((client_to_backend, backend_to_client)) => {
                emit_log_to_events(
                    &error_logger,
                    LogLevel::Info,
                    "Forward proxy: CONNECT tunnel closed",
                    &format!(
                        "Forward proxy: CONNECT tunnel closed for {connect_address} \
                         (client→backend: {client_to_backend} bytes, \
                         backend→client: {backend_to_client} bytes)"
                    ),
                    vec![
                        (
                            "forward_proxy.target",
                            LogAttributeValue::String(connect_address.clone()),
                        ),
                        (
                            "forward_proxy.bytes.client_to_backend",
                            LogAttributeValue::I64(client_to_backend as i64),
                        ),
                        (
                            "forward_proxy.bytes.backend_to_client",
                            LogAttributeValue::I64(backend_to_client as i64),
                        ),
                    ],
                    trace_context.clone(),
                );
                emit_forward_proxy_metric_to_events(
                    &error_logger,
                    "connect",
                    "tunnel_closed",
                    200,
                    None,
                    trace_context.clone(),
                );
            }
            Err(err) => {
                let err = ForwardProxyError::ConnectError {
                    target: connect_address.clone(),
                    kind: ConnectErrorKind::CopyFailed {
                        error: err.to_string(),
                    },
                };
                emit_error_log_to_events(&error_logger, &err, trace_context.clone());
                emit_forward_proxy_metric_to_events(
                    &error_logger,
                    "connect",
                    "tunnel_error",
                    502,
                    Some("tunnel_error".to_string()),
                    trace_context.clone(),
                );
            }
        }

        let _ = request;
        let _ = config;
    });

    // Respond with 200 Connection Established
    let response = Response::builder()
        .status(StatusCode::OK)
        .body(Empty::new().map_err(|e| match e {}).boxed_unsync())
        .unwrap_or_default();

    ctx.res = Some(ferron_http::HttpResponse::Custom(response));
    emit_forward_proxy_metric(ctx, "connect", "tunnel_established", 200, None);
    Ok(ForwardProxyResult::Handled)
}

/// Handle an HTTP forwarding request (absolute URI in HTTP/1.x).
async fn handle_http_forward(
    ctx: &mut HttpContext,
    request: Request<HttpBody>,
    config: &ForwardProxyConfig,
) -> Result<ForwardProxyResult, ForwardProxyError> {
    let (mut parts, body) = request.into_parts();

    let scheme = parts.uri.scheme_str();
    match scheme {
        Some("http") | None => {} // none means relative URI with host, still valid
        Some("https") => {
            let err = ForwardProxyError::UnsupportedScheme("https".to_string());
            emit_error_log(ctx, &err);
            ctx.res = Some(ferron_http::HttpResponse::BuiltinError(400, None));
            emit_forward_proxy_metric(ctx, "request", "unsupported_scheme", 400, None);
            return Ok(ForwardProxyResult::Handled);
        }
        Some(other) => {
            let err = ForwardProxyError::UnsupportedScheme(other.to_string());
            emit_error_log(ctx, &err);
            ctx.res = Some(ferron_http::HttpResponse::BuiltinError(400, None));
            emit_forward_proxy_metric(ctx, "request", "unsupported_scheme", 400, None);
            return Ok(ForwardProxyResult::Handled);
        }
    }

    let host = match parts.uri.host() {
        Some(h) => h.to_string(),
        None => {
            emit_error_log(ctx, &ForwardProxyError::MissingHost);
            ctx.res = Some(ferron_http::HttpResponse::BuiltinError(400, None));
            emit_forward_proxy_metric(ctx, "request", "bad_request", 400, None);
            return Ok(ForwardProxyResult::Handled);
        }
    };

    let port = parts.uri.port_u16().unwrap_or(80);

    // ACL: check port
    if !port_allowed(&config.allow_ports, port) {
        let err = ForwardProxyError::PortDenied { port };
        emit_error_log(ctx, &err);
        ctx.res = Some(ferron_http::HttpResponse::BuiltinError(403, None));
        emit_forward_proxy_metric(ctx, "request", "acl_denied", 403, None);
        return Ok(ForwardProxyResult::Handled);
    }

    // ACL: check domain
    if !domain_matches(&config.allow_domains, &host) {
        let err = ForwardProxyError::DomainDenied {
            domain: host.clone(),
        };
        emit_error_log(ctx, &err);
        ctx.res = Some(ferron_http::HttpResponse::BuiltinError(403, None));
        emit_forward_proxy_metric(ctx, "request", "acl_denied", 403, None);
        return Ok(ForwardProxyResult::Handled);
    }

    // Resolve DNS and validate IP (fail if IP is denied)
    let Some(resolved_ips) = resolve_and_validate_ip(ctx, &host, &config.deny_ips).await? else {
        let err = ForwardProxyError::DnsUnresolved(host.clone());
        emit_error_log(ctx, &err);
        ctx.res = Some(ferron_http::HttpResponse::BuiltinError(403, None));
        emit_forward_proxy_metric(ctx, "request", "dns_unresolved", 403, None);
        return Ok(ForwardProxyResult::Handled);
    };
    let addr = format!("{host}:{port}");
    let socket_addrs = resolved_ips
        .into_iter()
        .map(|ip| SocketAddr::new(ip, port))
        .collect::<Vec<_>>();

    // Connect to the backend
    let stream = match TcpStream::connect(&*socket_addrs).await {
        Ok(stream) => stream,
        Err(err) => {
            let status = match err.kind() {
                std::io::ErrorKind::ConnectionRefused
                | std::io::ErrorKind::NotFound
                | std::io::ErrorKind::HostUnreachable => StatusCode::SERVICE_UNAVAILABLE,
                std::io::ErrorKind::TimedOut => StatusCode::GATEWAY_TIMEOUT,
                _ => StatusCode::BAD_GATEWAY,
            };
            let err = ForwardProxyError::ForwardError {
                address: addr.clone(),
                kind: ForwardErrorKind::ConnectFailed {
                    error: err.to_string(),
                },
            };
            emit_error_log(ctx, &err);
            ctx.res = Some(ferron_http::HttpResponse::BuiltinError(
                status.as_u16(),
                None,
            ));
            emit_forward_proxy_metric(
                ctx,
                "request",
                "backend_connect_error",
                status.as_u16(),
                Some("backend_connect_error".to_string()),
            );
            return Ok(ForwardProxyResult::Handled);
        }
    };

    if let Err(err) = stream.set_nodelay(true) {
        emit_log_with_attrs(
            ctx,
            LogLevel::Warn,
            "Forward proxy: cannot set TCP_NODELAY",
            &format!("Forward proxy: cannot set TCP_NODELAY for {addr}: {err}"),
            vec![
                ("upstream.address", LogAttributeValue::String(addr.clone())),
                ("error.message", LogAttributeValue::String(err.to_string())),
            ],
        );
    }

    let stream = match stream.into_poll() {
        Ok(stream) => stream,
        Err(err) => {
            let err = ForwardProxyError::ForwardError {
                address: addr.clone(),
                kind: ForwardErrorKind::ConnectFailed {
                    error: err.to_string(),
                },
            };
            emit_error_log(ctx, &err);
            ctx.res = Some(ferron_http::HttpResponse::BuiltinError(502, None));
            emit_forward_proxy_metric(
                ctx,
                "request",
                "backend_connect_error",
                502,
                Some("backend_connect_error".to_string()),
            );
            return Ok(ForwardProxyResult::Handled);
        }
    };

    // Build the request with path-only URI
    let request_path = parts.uri.path();
    let query = parts
        .uri
        .query()
        .map(|q| format!("?{q}"))
        .unwrap_or_default();

    parts.uri = Uri::from_str(&format!("{request_path}{query}"))?;

    // Connection: close for HTTP/1.1
    parts.headers.insert(header::CONNECTION, "close".parse()?);

    let proxy_request = Request::from_parts(parts, body);

    // Forward the request
    let result = http_proxy_forward(stream, proxy_request, ctx).await;
    let status_code = result.status().as_u16();
    emit_forward_proxy_metric(
        ctx,
        "request",
        if status_code >= 400 {
            "backend_error"
        } else {
            "proxied"
        },
        status_code,
        (status_code >= 400).then(|| "backend_error".to_string()),
    );
    ctx.res = Some(ferron_http::HttpResponse::Custom(result));
    Ok(ForwardProxyResult::Handled)
}

/// Forward an HTTP request to a backend over an established TCP stream.
async fn http_proxy_forward(
    stream: impl AsyncRead + AsyncWrite + Unpin + 'static,
    proxy_request: Request<HttpBody>,
    ctx: &mut HttpContext,
) -> Response<HttpBody> {
    let io = VibeioIo::new(stream);

    let (mut sender, conn) = match hyper::client::conn::http1::handshake(io).await {
        Ok(data) => data,
        Err(err) => {
            let err = ForwardProxyError::ForwardError {
                address: String::new(),
                kind: ForwardErrorKind::HandshakeFailed {
                    error: err.to_string(),
                },
            };
            emit_error_log(ctx, &err);
            return error_response(StatusCode::BAD_GATEWAY);
        }
    };

    vibeio::spawn(async move {
        let _ = conn.await;
    });

    match sender.send_request(proxy_request).await {
        Ok(response) => response.map(|b| {
            b.map_err(|e| std::io::Error::other(e.to_string()))
                .boxed_unsync()
        }),
        Err(err) => {
            let err = ForwardProxyError::ForwardError {
                address: String::new(),
                kind: ForwardErrorKind::SendRequestFailed {
                    error: err.to_string(),
                },
            };
            emit_error_log(ctx, &err);
            error_response(StatusCode::BAD_GATEWAY)
        }
    }
}

/// Build an error response.
fn error_response(status: StatusCode) -> Response<HttpBody> {
    Response::builder()
        .status(status)
        .body(Empty::new().map_err(|e| match e {}).boxed_unsync())
        .unwrap_or_default()
}

/// Parse a host:port string, returning (host, port).
/// Uses default_port if no port is specified.
fn parse_host_port(addr: &str, default_port: u16) -> Result<(String, u16), ForwardProxyError> {
    // Handle IPv6: [::1]:8080 or [::1]
    if addr.starts_with('[') {
        if let Some(close_bracket) = addr.find(']') {
            let host = &addr[1..close_bracket];
            let rest = &addr[close_bracket + 1..];
            let port = if let Some(rest) = rest.strip_prefix(':') {
                rest.parse::<u16>().unwrap_or(default_port)
            } else {
                default_port
            };
            return Ok((host.to_string(), port));
        }
    }

    // IPv4 or hostname: host:port or host
    if let Some(colon_pos) = addr.rfind(':') {
        let host = &addr[..colon_pos];
        let port = addr[colon_pos + 1..].parse::<u16>().unwrap_or(default_port);
        Ok((host.to_string(), port))
    } else {
        Ok((addr.to_string(), default_port))
    }
}

/// Resolve a hostname to an IP and check against the deny list.
/// Returns `Ok(Some(ip))` if resolved, `Ok(None)` if resolution failed,
/// or `Err` if the resolved IP is denied.
async fn resolve_and_validate_ip(
    ctx: &mut HttpContext,
    host: &str,
    deny_ips: &[ipnet::IpNet],
) -> Result<Option<Vec<IpAddr>>, ForwardProxyError> {
    // First check if the host is already an IP address
    if let Ok(ip) = IpAddr::from_str(host) {
        if ip_denied(deny_ips, ip) {
            return Err(ForwardProxyError::DnsDeniedIp {
                host: host.to_string(),
                ip,
            });
        }
        return Ok(Some(vec![ip]));
    }

    // Resolve via DNS on the secondary tokio runtime
    let handle = match crate::try_get_secondary_runtime_handle() {
        Some(h) => h,
        None => {
            let err = ForwardProxyError::DnsUnavailable(host.to_string());
            emit_error_log(ctx, &err);
            return Ok(None);
        }
    };

    let host_str = host.to_string();
    let deny_ips = deny_ips.to_vec();

    // Spawn on secondary runtime to use tokio::net::lookup_host
    let result = handle
        .spawn({
            let host_str = host_str.clone();
            async move {
                match tokio::net::lookup_host(format!("{host_str}:0")).await {
                    Ok(addrs) => {
                        let ips = addrs.map(|a| a.ip()).collect::<Vec<_>>();
                        if !ips.is_empty() {
                            for ip in &ips {
                                if ip_denied(&deny_ips, *ip) {
                                    return Err(ForwardProxyError::DnsDeniedIp {
                                        host: host_str,
                                        ip: *ip,
                                    });
                                }
                            }
                            Ok(Some(ips))
                        } else {
                            Ok(None)
                        }
                    }
                    Err(e) => Err(ForwardProxyError::DnsUnresolved(format!(
                        "DNS lookup failed: {e}"
                    ))),
                }
            }
        })
        .await
        .map_err(|_| {
            ForwardProxyError::DnsUnresolved(format!("DNS resolution task panicked for {host_str}"))
        })??;

    if result.is_none() {
        let err = ForwardProxyError::DnsUnresolved(host_str.clone());
        emit_error_log(ctx, &err);
    }

    Ok(result)
}

/// Emit a structured log event for a `ForwardProxyError`, using its `summary()`
/// and including the error type as an attribute.
fn emit_error_log(ctx: &HttpContext, err: &ForwardProxyError) {
    let mut attributes = vec![
        ("error.type", LogAttributeValue::StaticStr(err.error_type())),
        ("error.message", LogAttributeValue::String(err.to_string())),
    ];
    match err {
        ForwardProxyError::PortDenied { port } => {
            attributes.push((
                "network.destination.port",
                LogAttributeValue::I64(*port as i64),
            ));
        }
        ForwardProxyError::DomainDenied { domain } => {
            attributes.push((
                "network.destination.name",
                LogAttributeValue::String(domain.clone()),
            ));
        }
        ForwardProxyError::UnsupportedScheme(scheme) => {
            attributes.push(("url.scheme", LogAttributeValue::String(scheme.clone())));
        }
        ForwardProxyError::DnsUnresolved(host) | ForwardProxyError::DnsUnavailable(host) => {
            attributes.push(("dns.name", LogAttributeValue::String(host.clone())));
        }
        ForwardProxyError::DnsDeniedIp { host, .. } => {
            attributes.push(("dns.name", LogAttributeValue::String(host.clone())));
        }
        ForwardProxyError::ConnectError { target, .. } => {
            attributes.push((
                "forward_proxy.target",
                LogAttributeValue::String(target.clone()),
            ));
        }
        ForwardProxyError::ForwardError { address, .. } => {
            attributes.push((
                "upstream.address",
                LogAttributeValue::String(address.clone()),
            ));
        }
        _ => {}
    }

    let level = match err {
        ForwardProxyError::ConnectDisabled
        | ForwardProxyError::BadConnectRequest
        | ForwardProxyError::PortDenied { .. }
        | ForwardProxyError::DomainDenied { .. }
        | ForwardProxyError::UnsupportedScheme(_)
        | ForwardProxyError::MissingHost
        | ForwardProxyError::DnsUnresolved(_)
        | ForwardProxyError::DnsUnavailable(_)
        | ForwardProxyError::DnsDeniedIp { .. } => LogLevel::Warn,
        ForwardProxyError::ConnectError { kind, .. } => match kind {
            ConnectErrorKind::UpgradeFailed { .. } | ConnectErrorKind::ConnectFailed { .. } => {
                LogLevel::Error
            }
            ConnectErrorKind::CopyFailed { .. } => LogLevel::Warn,
        },
        ForwardProxyError::ForwardError { .. } => LogLevel::Error,
    };

    ctx.events.emit(Event::Log(LogEvent {
        level,
        message: err.to_string(),
        summary: err.summary().into(),
        target: LOG_TARGET,
        attributes,
        trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
    }));
}

/// Emit a structured log event for a `ForwardProxyError` to a standalone event sink
/// (used inside spawned tasks where `HttpContext` is not available).
fn emit_error_log_to_events(
    events: &CompositeEventSink,
    err: &ForwardProxyError,
    trace_context: Option<ferron_observability::EventTraceContext>,
) {
    let mut attributes = vec![
        ("error.type", LogAttributeValue::StaticStr(err.error_type())),
        ("error.message", LogAttributeValue::String(err.to_string())),
    ];
    match err {
        ForwardProxyError::ConnectError { target, .. } => {
            attributes.push((
                "forward_proxy.target",
                LogAttributeValue::String(target.clone()),
            ));
        }
        ForwardProxyError::ForwardError { address, .. } => {
            attributes.push((
                "upstream.address",
                LogAttributeValue::String(address.clone()),
            ));
        }
        ForwardProxyError::DnsUnresolved(host)
        | ForwardProxyError::DnsUnavailable(host)
        | ForwardProxyError::DnsDeniedIp { host, .. } => {
            attributes.push(("dns.name", LogAttributeValue::String(host.clone())));
        }
        _ => {}
    }

    let level = match err {
        ForwardProxyError::ConnectError { kind, .. } => match kind {
            ConnectErrorKind::UpgradeFailed { .. } | ConnectErrorKind::ConnectFailed { .. } => {
                LogLevel::Error
            }
            ConnectErrorKind::CopyFailed { .. } => LogLevel::Warn,
        },
        ForwardProxyError::ForwardError { .. } => LogLevel::Error,
        _ => LogLevel::Warn,
    };

    events.emit(Event::Log(LogEvent {
        level,
        message: err.to_string(),
        summary: err.summary().into(),
        target: LOG_TARGET,
        attributes,
        trace_context,
    }));
}

/// Emit a log event with explicit summary and attributes.
fn emit_log_with_attrs(
    ctx: &HttpContext,
    level: LogLevel,
    summary: &'static str,
    message: &str,
    attributes: Vec<(&'static str, LogAttributeValue)>,
) {
    ctx.events.emit(Event::Log(LogEvent {
        level,
        message: message.to_string(),
        summary: summary.into(),
        target: LOG_TARGET,
        attributes,
        trace_context: ferron_http::trace_context::current_event_trace_context(ctx),
    }));
}

/// Emit a log event with explicit summary and attributes to a standalone event sink.
fn emit_log_to_events(
    events: &CompositeEventSink,
    level: LogLevel,
    summary: &'static str,
    message: &str,
    attributes: Vec<(&'static str, LogAttributeValue)>,
    trace_context: Option<ferron_observability::EventTraceContext>,
) {
    events.emit(Event::Log(LogEvent {
        level,
        message: message.to_string(),
        summary: summary.into(),
        target: LOG_TARGET,
        attributes,
        trace_context,
    }));
}

fn emit_forward_proxy_metric(
    ctx: &HttpContext,
    mode: &'static str,
    result: &'static str,
    status_code: u16,
    error_type: Option<String>,
) {
    emit_forward_proxy_metric_to_events(
        &ctx.events,
        mode,
        result,
        status_code,
        error_type,
        ferron_http::trace_context::current_event_trace_context(ctx),
    );
}

fn emit_forward_proxy_metric_to_events(
    events: &CompositeEventSink,
    mode: &'static str,
    result: &'static str,
    status_code: u16,
    error_type: Option<String>,
    trace_context: Option<ferron_observability::EventTraceContext>,
) {
    let mut attributes = vec![
        (
            "ferron.forward_proxy.mode",
            MetricAttributeValue::StaticStr(mode),
        ),
        (
            "ferron.forward_proxy.result",
            MetricAttributeValue::StaticStr(result),
        ),
        (
            "http.response.status_code",
            MetricAttributeValue::I64(status_code as i64),
        ),
    ];
    if let Some(error_type) = error_type {
        attributes.push(("error.type", MetricAttributeValue::String(error_type)));
    }

    events.emit(Event::Metric(MetricEvent {
        name: "ferron.forward_proxy.requests",
        attributes,
        ty: MetricType::Counter,
        value: MetricValue::U64(1),
        unit: Some("{request}"),
        description: Some("Number of forward proxy requests by mode and outcome."),
        trace_context,
    }));
}
