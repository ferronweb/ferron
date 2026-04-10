//! Core forward proxy logic: CONNECT tunneling, HTTP forwarding, and ACL enforcement.

use std::net::IpAddr;
use std::str::FromStr;

use bytes::Bytes;
use ferron_http::HttpContext;
use ferron_observability::{Event, LogEvent, LogLevel};
use http::header;
use http::{Request, Response, StatusCode, Uri};
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, Empty};
use tokio::io::{AsyncRead, AsyncWrite};
use vibeio::net::TcpStream;
use vibeio_hyper::VibeioIo;

use crate::config::{domain_matches, ip_denied, port_allowed, ForwardProxyConfig};

const LOG_TARGET: &str = "ferron-fproxy";

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
) -> Result<ForwardProxyResult, Box<dyn std::error::Error + Send + Sync>> {
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
            emit_log(
                ctx,
                LogLevel::Warn,
                "CONNECT method is disabled for forward proxy",
            );
            ctx.res = Some(ferron_http::HttpResponse::BuiltinError(403, None));
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
) -> Result<ForwardProxyResult, Box<dyn std::error::Error + Send + Sync>> {
    let connect_address = match request.uri().authority() {
        Some(auth) => auth.to_string(),
        None => {
            emit_log(ctx, LogLevel::Warn, "CONNECT request missing authority");
            ctx.res = Some(ferron_http::HttpResponse::BuiltinError(400, None));
            return Ok(ForwardProxyResult::Handled);
        }
    };

    // Parse host and port
    let (host, port) = parse_host_port(&connect_address, 443)?;

    // ACL: check port
    if !port_allowed(&config.allow_ports, port) {
        emit_log(
            ctx,
            LogLevel::Warn,
            &format!("CONNECT to port {port} denied by ACL"),
        );
        ctx.res = Some(ferron_http::HttpResponse::BuiltinError(403, None));
        return Ok(ForwardProxyResult::Handled);
    }

    // ACL: check domain
    if !domain_matches(&config.allow_domains, &host) {
        emit_log(
            ctx,
            LogLevel::Warn,
            &format!("CONNECT to {host} denied by domain ACL"),
        );
        ctx.res = Some(ferron_http::HttpResponse::BuiltinError(403, None));
        return Ok(ForwardProxyResult::Handled);
    }

    // Resolve DNS and validate IP
    let resolved_ip = resolve_and_validate_ip(ctx, &host, &config.deny_ips).await?;
    if let Some(ip) = resolved_ip {
        if ip_denied(&config.deny_ips, ip) {
            emit_log(
                ctx,
                LogLevel::Warn,
                &format!("CONNECT to {host} resolved to denied IP {ip}"),
            );
            ctx.res = Some(ferron_http::HttpResponse::BuiltinError(403, None));
            return Ok(ForwardProxyResult::Handled);
        }
    }

    let error_logger = ctx.events.clone();
    let config = config.clone();
    let connect_address = connect_address.clone();

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
                    error_logger.emit(Event::Log(LogEvent {
                        level: LogLevel::Error,
                        message: format!(
                            "Forward proxy: HTTP CONNECT upgrade failed for {connect_address}"
                        ),
                        target: LOG_TARGET,
                    }));
                    return;
                }
            },
            None => {
                error_logger.emit(Event::Log(LogEvent {
                    level: LogLevel::Error,
                    message: format!(
                        "Forward proxy: no upgrade future for CONNECT {connect_address}"
                    ),
                    target: LOG_TARGET,
                }));
                return;
            }
        };

        // Connect to the remote server
        let backend_stream = match TcpStream::connect(&connect_address).await {
            Ok(stream) => stream,
            Err(err) => {
                error_logger.emit(Event::Log(LogEvent {
                    level: LogLevel::Error,
                    message: format!("Forward proxy: cannot connect to {connect_address}: {err}"),
                    target: LOG_TARGET,
                }));
                return;
            }
        };

        if let Err(err) = backend_stream.set_nodelay(true) {
            error_logger.emit(Event::Log(LogEvent {
                level: LogLevel::Warn,
                message: format!(
                    "Forward proxy: cannot set TCP_NODELAY for {connect_address}: {err}"
                ),
                target: LOG_TARGET,
            }));
        }

        let mut backend_stream = match backend_stream.into_poll() {
            Ok(stream) => stream,
            Err(err) => {
                error_logger.emit(Event::Log(LogEvent {
                    level: LogLevel::Error,
                    message: format!(
                        "Forward proxy: cannot convert TCP stream to poll I/O for {connect_address}: {err}"
                    ),
                    target: LOG_TARGET,
                }));
                return;
            }
        };

        let mut upgraded = upgraded;

        // Bidirectional copy between client and backend
        match tokio::io::copy_bidirectional(&mut upgraded, &mut backend_stream).await {
            Ok((client_to_backend, backend_to_client)) => {
                error_logger.emit(Event::Log(LogEvent {
                    level: LogLevel::Info,
                    message: format!(
                        "Forward proxy: CONNECT tunnel closed for {connect_address} \
                         (client→backend: {client_to_backend} bytes, \
                         backend→client: {backend_to_client} bytes)"
                    ),
                    target: LOG_TARGET,
                }));
            }
            Err(err) => {
                error_logger.emit(Event::Log(LogEvent {
                    level: LogLevel::Warn,
                    message: format!(
                        "Forward proxy: CONNECT tunnel error for {connect_address}: {err}"
                    ),
                    target: LOG_TARGET,
                }));
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
    Ok(ForwardProxyResult::Handled)
}

/// Handle an HTTP forwarding request (absolute URI in HTTP/1.x).
async fn handle_http_forward(
    ctx: &mut HttpContext,
    request: Request<HttpBody>,
    config: &ForwardProxyConfig,
) -> Result<ForwardProxyResult, Box<dyn std::error::Error + Send + Sync>> {
    let (mut parts, body) = request.into_parts();

    let scheme = parts.uri.scheme_str();
    match scheme {
        Some("http") | None => {} // none means relative URI with host, still valid
        Some("https") => {
            emit_log(
                ctx,
                LogLevel::Warn,
                "Forward proxy: HTTPS scheme in forward request is not supported",
            );
            ctx.res = Some(ferron_http::HttpResponse::BuiltinError(400, None));
            return Ok(ForwardProxyResult::Handled);
        }
        Some(other) => {
            emit_log(
                ctx,
                LogLevel::Warn,
                &format!("Forward proxy: unsupported scheme '{other}'"),
            );
            ctx.res = Some(ferron_http::HttpResponse::BuiltinError(400, None));
            return Ok(ForwardProxyResult::Handled);
        }
    }

    let host = match parts.uri.host() {
        Some(h) => h.to_string(),
        None => {
            emit_log(
                ctx,
                LogLevel::Warn,
                "Forward proxy: missing host in request URI",
            );
            ctx.res = Some(ferron_http::HttpResponse::BuiltinError(400, None));
            return Ok(ForwardProxyResult::Handled);
        }
    };

    let port = parts.uri.port_u16().unwrap_or(80);

    // ACL: check port
    if !port_allowed(&config.allow_ports, port) {
        emit_log(
            ctx,
            LogLevel::Warn,
            &format!("Forward proxy: port {port} denied by ACL"),
        );
        ctx.res = Some(ferron_http::HttpResponse::BuiltinError(403, None));
        return Ok(ForwardProxyResult::Handled);
    }

    // ACL: check domain
    if !domain_matches(&config.allow_domains, &host) {
        emit_log(
            ctx,
            LogLevel::Warn,
            &format!("Forward proxy: host '{host}' denied by domain ACL"),
        );
        ctx.res = Some(ferron_http::HttpResponse::BuiltinError(403, None));
        return Ok(ForwardProxyResult::Handled);
    }

    // Resolve DNS and validate IP
    let resolved_ip = resolve_and_validate_ip(ctx, &host, &config.deny_ips).await?;
    if let Some(ip) = resolved_ip {
        if ip_denied(&config.deny_ips, ip) {
            emit_log(
                ctx,
                LogLevel::Warn,
                &format!("Forward proxy: host '{host}' resolved to denied IP {ip}"),
            );
            ctx.res = Some(ferron_http::HttpResponse::BuiltinError(403, None));
            return Ok(ForwardProxyResult::Handled);
        }
    }

    let addr = format!("{host}:{port}");

    // Connect to the backend
    let stream = match TcpStream::connect(&addr).await {
        Ok(stream) => stream,
        Err(err) => {
            let status = match err.kind() {
                std::io::ErrorKind::ConnectionRefused
                | std::io::ErrorKind::NotFound
                | std::io::ErrorKind::HostUnreachable => StatusCode::SERVICE_UNAVAILABLE,
                std::io::ErrorKind::TimedOut => StatusCode::GATEWAY_TIMEOUT,
                _ => StatusCode::BAD_GATEWAY,
            };
            emit_log(
                ctx,
                LogLevel::Error,
                &format!("Forward proxy: cannot connect to {addr}: {err}"),
            );
            ctx.res = Some(ferron_http::HttpResponse::BuiltinError(
                status.as_u16(),
                None,
            ));
            return Ok(ForwardProxyResult::Handled);
        }
    };

    if let Err(err) = stream.set_nodelay(true) {
        emit_log(
            ctx,
            LogLevel::Warn,
            &format!("Forward proxy: cannot set TCP_NODELAY for {addr}: {err}"),
        );
    }

    let stream = match stream.into_poll() {
        Ok(stream) => stream,
        Err(err) => {
            emit_log(
                ctx,
                LogLevel::Error,
                &format!("Forward proxy: cannot convert TCP stream to poll I/O: {err}"),
            );
            ctx.res = Some(ferron_http::HttpResponse::BuiltinError(502, None));
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
            emit_log(
                ctx,
                LogLevel::Error,
                &format!("Forward proxy: HTTP/1 handshake failed: {err}"),
            );
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
            emit_log(
                ctx,
                LogLevel::Error,
                &format!("Forward proxy: request to backend failed: {err}"),
            );
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
fn parse_host_port(
    addr: &str,
    default_port: u16,
) -> Result<(String, u16), Box<dyn std::error::Error + Send + Sync>> {
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
) -> Result<Option<IpAddr>, Box<dyn std::error::Error + Send + Sync>> {
    // First check if the host is already an IP address
    if let Ok(ip) = IpAddr::from_str(host) {
        if ip_denied(deny_ips, ip) {
            return Err(format!("IP {ip} is in the denied IP list").into());
        }
        return Ok(Some(ip));
    }

    // Resolve via DNS on the secondary tokio runtime
    let handle = match crate::try_get_secondary_runtime_handle() {
        Some(h) => h,
        None => {
            emit_log(
                ctx,
                LogLevel::Warn,
                &format!(
                    "Forward proxy: secondary runtime not available for DNS resolution of {host}"
                ),
            );
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
                    Ok(mut addrs) => {
                        if let Some(ip) = addrs.next() {
                            let ip = ip.ip();
                            if ip_denied(&deny_ips, ip) {
                                Err(format!("Host '{host_str}' resolved to denied IP {ip}"))
                            } else {
                                Ok(Some(ip))
                            }
                        } else {
                            Ok(None)
                        }
                    }
                    Err(e) => Err(format!("DNS lookup failed: {e}")),
                }
            }
        })
        .await
        .map_err(|e| format!("DNS resolution task panicked: {e}"))??;

    if result.is_none() {
        emit_log(
            ctx,
            LogLevel::Warn,
            &format!("Forward proxy: DNS resolution returned no addresses for {host_str}"),
        );
    }

    Ok(result)
}

/// Emit a log event to the context's event sink.
fn emit_log(ctx: &HttpContext, level: LogLevel, message: &str) {
    ctx.events.emit(Event::Log(LogEvent {
        level,
        message: message.to_string(),
        target: LOG_TARGET,
    }));
}
