//! TCP listener and connection handling

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use ferron_core::admin::ADMIN_METRICS;
use ferron_core::pipeline::Pipeline;
use ferron_core::providers::Provider;
use ferron_core::runtime::Runtime;
use ferron_core::{log_error, log_info};
use ferron_http::{HttpContext, HttpErrorContext, HttpFileContext};
use ferron_observability::{
    CompositeEventSink, Event, EventSink, LogEvent, LogLevel, ObservabilityContext,
};
use http::Request;
use http_body_util::{combinators::UnsyncBoxBody, BodyExt};
use rustls::server::Acceptor;
use tokio_util::sync::CancellationToken;
use vibeio_http::{Http1, Http1Options, Http2, Http2Options, HttpProtocol};

use crate::config::ThreeStageResolver;
use crate::handler::request_handler;
use crate::server::tls_resolve::RadixTree;
use crate::server::HttpServerConfig;
use crate::util::proxy_protocol::read_proxy_header;

/// Bundled shared state for the per-request handler closure.
/// Cloning a single `Arc<RequestHandlerState>` replaces cloning 6 individual
/// `Arc`s plus a `String` and a `CompositeEventSink`, reducing atomic
/// refcount contention at high RPS.
///
/// # Performance optimization
/// Connection-scoped radix tree lookups (observability, HTTP connection options)
/// are resolved once at connection setup and cached here, avoiding redundant
/// tree traversals on every HTTP request within the same connection.
struct RequestHandlerState {
    pipeline: Arc<Pipeline<HttpContext>>,
    file_pipeline: Arc<Pipeline<HttpFileContext>>,
    error_pipeline: Arc<Pipeline<HttpErrorContext>>,
    config_resolver: Arc<ThreeStageResolver>,
    /// Pre-resolved observability sinks for this connection's IP + SNI hostname.
    connection_observability: CompositeEventSink,
    /// Kept for virtual-hosting lookups when request Host header differs from SNI.
    observability_resolver: Arc<RadixTree<Vec<ObservabilityProviderEntry>>>,
    local_address: SocketAddr,
    remote_address: SocketAddr,
    /// Pre-normalized hostname from TLS SNI (available for per-request Host header overrides).
    hinted_hostname: Option<String>,
    encrypted: bool,
    https_port: Option<u16>,
}

// Type alias for the config ArcSwap
type ConfigArcSwap = Arc<ArcSwap<HttpServerConfig>>;

type ObservabilityProviderEntry = (
    Arc<dyn Provider<ObservabilityContext>>,
    Arc<ferron_core::config::ServerConfigurationBlock>,
);

const LOG_TARGET: &str = "ferron-http-server";
type ResponseBody = UnsyncBoxBody<bytes::Bytes, io::Error>;
type RequestHandlerFuture =
    Pin<Box<dyn std::future::Future<Output = Result<http::Response<ResponseBody>, io::Error>>>>;

/// RAII guard that decrements the active connection counter on drop.
struct ConnectionCountGuard;

impl ConnectionCountGuard {
    fn new() -> Self {
        ADMIN_METRICS
            .connections_active
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Self
    }
}

impl Drop for ConnectionCountGuard {
    fn drop(&mut self) {
        ADMIN_METRICS
            .connections_active
            .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TcpListenerOptions {
    pub address: SocketAddr,
    pub send_buffer_size: Option<usize>,
    pub recv_buffer_size: Option<usize>,
    pub backlog: Option<i32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HttpProtocols {
    pub http1: bool,
    pub http2: bool,
}

impl HttpProtocols {
    pub const fn empty() -> Self {
        Self {
            http1: false,
            http2: false,
        }
    }

    pub const fn supports_http1(self) -> bool {
        self.http1
    }

    #[allow(dead_code)]
    pub const fn supports_http2(self) -> bool {
        self.http2
    }

    pub fn alpn_protocols(self) -> Vec<Vec<u8>> {
        let mut protocols = Vec::new();
        if self.http2 {
            protocols.push(b"h2".to_vec());
        }
        if self.http1 {
            protocols.push(b"http/1.1".to_vec());
            protocols.push(b"http/1.0".to_vec());
        }
        protocols
    }
}

impl Default for HttpProtocols {
    fn default() -> Self {
        Self {
            http1: true,
            http2: true,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Http2Settings {
    pub initial_window_size: Option<u32>,
    pub max_frame_size: Option<u32>,
    pub max_concurrent_streams: Option<u32>,
    pub max_header_list_size: Option<u32>,
    pub enable_connect_protocol: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct HttpConnectionOptions {
    pub protocols: HttpProtocols,
    pub h1_enable_early_hints: bool,
    pub h2: Http2Settings,
    pub proxy_protocol_enabled: bool,
}

impl HttpConnectionOptions {
    pub fn alpn_protocols(&self) -> Vec<Vec<u8>> {
        self.protocols.alpn_protocols()
    }
}

pub struct TcpListenerHandle {
    cancel_token: Arc<CancellationToken>,
}

impl TcpListenerHandle {
    pub fn new(
        options: TcpListenerOptions,
        config: ConfigArcSwap,
        runtime: &mut Runtime,
    ) -> Result<Self, std::io::Error> {
        let listener = build_tcp_listener(
            options.address,
            (options.send_buffer_size, options.recv_buffer_size),
            options.backlog,
        )?;

        if config.load().tls_resolver.is_some() {
            log_info!("HTTPS server listening on {}", options.address);
        } else {
            log_info!("HTTP server listening on {}", options.address);
        }

        let cancel_token = Arc::new(CancellationToken::new());

        let config_clone = config.clone();
        let cancel_token_clone = cancel_token.clone();

        runtime.spawn_primary_task(move || {
            let new_listener_result = listener.try_clone();
            let cancel_token = cancel_token_clone.clone();
            let config = config_clone.clone();
            Box::pin(async move {
                let Ok(new_listener) = new_listener_result else {
                    log_error!("Failed to clone listener");
                    return;
                };
                let Ok(listener) = vibeio::net::TcpListener::from_std(new_listener) else {
                    log_error!("Failed to convert listener to vibeio");
                    return;
                };
                #[cfg(unix)]
                let mut handle_exhaustion_backoff = Duration::from_millis(10);
                loop {
                    let accept_result = tokio::select! {
                        res = listener.accept() => res,
                        _ = cancel_token.cancelled() => {
                            return;
                        }
                    };
                    let (socket, _) = match accept_result {
                        Ok(socket) => {
                            #[cfg(unix)]
                            {
                                handle_exhaustion_backoff = Duration::from_millis(10);
                            }
                            socket
                        }
                        Err(err) => {
                            let global_observability =
                                resolve_root_observability_sink(&config.load().observability_resolver);
                            emit_error(
                                &global_observability,
                                format!("Failed to accept connection: {err}"),
                            );
                            #[cfg(unix)]
                            if err.raw_os_error() == Some(24) {
                                vibeio::time::sleep(handle_exhaustion_backoff).await;
                                handle_exhaustion_backoff =
                                    handle_exhaustion_backoff.saturating_mul(2);
                                if handle_exhaustion_backoff > Duration::from_secs(1) {
                                    handle_exhaustion_backoff = Duration::from_secs(1);
                                }
                            }
                            continue;
                        }
                    };
                    let _ = socket.set_nodelay(true);

                    let Ok(socket) = socket.into_poll() else {
                        let global_observability =
                            resolve_root_observability_sink(&config.load().observability_resolver);
                        emit_error(
                            &global_observability,
                            "Failed to convert socket to poll-based I/O",
                        );
                        continue;
                    };

                    // Load the current config for this connection
                    let server_config = config.load_full();
                    let connection_cancel_token = cancel_token.clone();
                    vibeio::spawn(async move {
                        let _conn_guard = ConnectionCountGuard::new();

                        // Read PROXY protocol header
                        // Use root HttpConnectionOptions to determine if PROXY protocol is enabled
                        let proxy_protocol_enabled = server_config
                            .http_connection_options_resolver
                            .root_data()
                            .map(|opts| opts.proxy_protocol_enabled)
                            .unwrap_or(false);
                        let (socket, proxy_client_addr, proxy_server_addr) = if proxy_protocol_enabled {
                            // Use tokio's TcpStream to read PROXY header asynchronously
                            match read_proxy_header(socket).await {
                                Ok((stream, client_addr, server_addr)) => {
                                    // Convert back to std TcpStream for vibeio
                                    (stream, client_addr, server_addr)
                                }
                                Err(e) => {
                                    let global_observability =
                                        resolve_root_observability_sink(&server_config.observability_resolver);
                                    emit_error(
                                        &global_observability,
                                        format!("Failed to read PROXY protocol header: {e}"),
                                    );
                                    return;
                                }
                            }
                        } else {
                            (socket, None, None)
                        };

                        // Use PROXY protocol addresses if available, otherwise get from socket
                        let (remote_addr, local_addr) = if let (Some(client), Some(server)) =
                            (proxy_client_addr, proxy_server_addr)
                        {
                            (client, server)
                        } else {
                            let Ok(remote_addr) = socket.peer_addr() else {
                                let global_observability =
                                    resolve_root_observability_sink(&server_config.observability_resolver);
                                emit_error(&global_observability, "Failed to get remote address");
                                return;
                            };
                            let Ok(local_addr) = socket.local_addr() else {
                                let global_observability =
                                    resolve_root_observability_sink(&server_config.observability_resolver);
                                emit_error(&global_observability, "Failed to get local address");
                                return;
                            };
                            (remote_addr, local_addr)
                        };
                        let ip_observability = resolve_observability_sink(
                            &server_config.observability_resolver,
                            Some(local_addr.ip()),
                            None,
                            &CompositeEventSink::new(vec![]),
                        );

                        if let Some(tls_resolver) = &server_config.tls_resolver {
                            let Ok(start_handshake) =
                                tokio_rustls::LazyConfigAcceptor::new(Acceptor::default(), socket)
                                    .await
                            else {
                                emit_error(&ip_observability, "Failed to start TLS handshake");
                                return;
                            };
                            let sni = start_handshake
                                .client_hello()
                                .server_name()
                                .map(std::borrow::ToOwned::to_owned);
                            let hinted_hostname =
                                sni.as_deref().and_then(normalize_host_for_lookup);
                            let connection_options = resolve_http_connection_options(
                                &server_config.http_connection_options_resolver,
                                local_addr.ip(),
                                hinted_hostname.as_deref(),
                            );
                            let resolver = if let Some(sni) = sni.as_deref() {
                                tls_resolver.lookup_ip_and_hostname(local_addr.ip(), sni)
                            } else {
                                tls_resolver.lookup_ip(local_addr.ip())
                            };
                            if let Some(resolver) = resolver {
                                let tls_stream_option = match
                                    resolver.handshake(start_handshake).await
                                {
                                    Ok(s) => s,
                                    Err(e) => {
                                    let tls_observability = resolve_observability_sink(
                                        &server_config.observability_resolver,
                                        Some(local_addr.ip()),
                                        hinted_hostname.as_deref(),
                                        &ip_observability,
                                    );
                                    emit_error(&tls_observability, format!("Failed to start TLS handshake: {e}"));
                                    return;
                                    }
                                };
                                let tls_observability = resolve_observability_sink(
                                    &server_config.observability_resolver,
                                    Some(local_addr.ip()),
                                    hinted_hostname.as_deref(),
                                    &ip_observability,
                                );
                                if let Some(tls_stream) = tls_stream_option {
                                    let negotiated_protocol = tls_stream
                                        .get_ref()
                                        .1
                                        .alpn_protocol()
                                        .map(|protocol| protocol.to_vec());
                                    if negotiated_protocol.as_deref() == Some(b"h2".as_slice()) {
                                        handle_http2_connection(
                                            tls_stream,
                                            remote_addr,
                                            server_config.pipeline.clone(),
                                            server_config.file_pipeline.clone(),
                                            server_config.error_pipeline.clone(),
                                            server_config.config_resolver.clone(),
                                            local_addr,
                                            hinted_hostname,
                                            true,
                                            server_config.https_port,
                                            connection_options,
                                            server_config.observability_resolver.clone(),
                                            tls_observability,
                                            (*connection_cancel_token).clone(),
                                            server_config.reload_token.clone(),
                                        )
                                        .await;
                                    } else if connection_options.protocols.supports_http1() {
                                        handle_http1_connection(
                                            tls_stream,
                                            remote_addr,
                                            server_config.pipeline.clone(),
                                            server_config.file_pipeline.clone(),
                                            server_config.error_pipeline.clone(),
                                            server_config.config_resolver.clone(),
                                            local_addr,
                                            hinted_hostname,
                                            true,
                                            server_config.https_port,
                                            connection_options,
                                            server_config.observability_resolver.clone(),
                                            tls_observability,
                                            (*connection_cancel_token).clone(),
                                            server_config.reload_token.clone(),
                                        )
                                        .await;
                                    } else {
                                        emit_error(
                                            &tls_observability,
                                            "TLS connection did not negotiate a supported HTTP protocol",
                                        );
                                    }
                                }
                            } else {
                                // Construct empty rustls `ServerConfig`
                                if let Ok(b) = rustls::ServerConfig::builder_with_provider(
                                      Arc::new(rustls::crypto::aws_lc_rs::default_provider())
                                    )
                                    .with_safe_default_protocol_versions() {
                                        let tls_config = b.with_no_client_auth().with_cert_resolver(Arc::new(NoCertResolver));
                                        if let Err(e) = start_handshake.into_stream(Arc::new(tls_config)).await {
                                            let tls_observability = resolve_observability_sink(
                                                &server_config.observability_resolver,
                                                Some(local_addr.ip()),
                                                hinted_hostname.as_deref(),
                                                &ip_observability,
                                            );
                                            emit_error(&tls_observability, format!("Failed to start TLS handshake: {e}"));
                                        }
                                    }
                            }
                        } else {
                            let connection_options = resolve_http_connection_options(
                                &server_config.http_connection_options_resolver,
                                local_addr.ip(),
                                None,
                            );
                            if !connection_options.protocols.supports_http1() {
                                emit_error(
                                    &ip_observability,
                                    "Plain TCP listener requires HTTP/1.x support",
                                );
                                return;
                            }
                            handle_http1_connection(
                                socket,
                                remote_addr,
                                server_config.pipeline.clone(),
                                server_config.file_pipeline.clone(),
                                server_config.error_pipeline.clone(),
                                server_config.config_resolver.clone(),
                                local_addr,
                                None,
                                false,
                                server_config.https_port,
                                connection_options,
                                server_config.observability_resolver.clone(),
                                ip_observability,
                                (*connection_cancel_token).clone(),
                                server_config.reload_token.clone(),
                            )
                            .await;
                        }
                    });
                }
            })
        });

        Ok(Self { cancel_token })
    }

    pub fn cancel(&self) {
        self.cancel_token.cancel();
    }
}

fn build_tcp_listener(
    address: SocketAddr,
    tcp_buffer_sizes: (Option<usize>, Option<usize>),
    backlog: Option<i32>,
) -> Result<std::net::TcpListener, io::Error> {
    let listener_socket = socket2::Socket::new(
        if address.is_ipv6() {
            socket2::Domain::IPV6
        } else {
            socket2::Domain::IPV4
        },
        socket2::Type::STREAM,
        Some(socket2::Protocol::TCP),
    )?;

    listener_socket
        .set_reuse_address(!cfg!(windows))
        .unwrap_or_default();
    if let Some(send_buffer_size) = tcp_buffer_sizes.0 {
        listener_socket
            .set_send_buffer_size(send_buffer_size)
            .unwrap_or_default();
    }
    if let Some(recv_buffer_size) = tcp_buffer_sizes.1 {
        listener_socket
            .set_recv_buffer_size(recv_buffer_size)
            .unwrap_or_default();
    }
    if address.is_ipv6() {
        listener_socket.set_only_v6(false).unwrap_or_default();
    }

    listener_socket.bind(&address.into())?;
    listener_socket.listen(backlog.unwrap_or(1024))?;

    Ok(listener_socket.into())
}

#[allow(clippy::too_many_arguments)]
async fn handle_http1_connection<S>(
    socket: S,
    remote_address: SocketAddr,
    pipeline: Arc<Pipeline<HttpContext>>,
    file_pipeline: Arc<Pipeline<HttpFileContext>>,
    error_pipeline: Arc<Pipeline<HttpErrorContext>>,
    config_resolver: Arc<ThreeStageResolver>,
    local_address: SocketAddr,
    hinted_hostname: Option<String>,
    encrypted: bool,
    https_port: Option<u16>,
    connection_options: HttpConnectionOptions,
    observability_resolver: Arc<RadixTree<Vec<ObservabilityProviderEntry>>>,
    connection_observability: CompositeEventSink,
    shutdown_token: CancellationToken,
    reload_token: CancellationToken,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + 'static,
{
    let graceful_shutdown = CancellationToken::new();
    let handler_state = Arc::new(RequestHandlerState {
        pipeline,
        file_pipeline,
        error_pipeline,
        config_resolver,
        connection_observability,
        observability_resolver,
        local_address,
        remote_address,
        hinted_hostname,
        encrypted,
        https_port,
    });
    let mut connection_future = Box::pin(
        Http1::new(socket, build_http1_options(&connection_options))
            .graceful_shutdown_token(graceful_shutdown.clone())
            .handle(build_request_handler(handler_state.clone())),
    );
    let connection_result = tokio::select! {
        result = &mut connection_future => result,
        _ = shutdown_token.cancelled() => {
            graceful_shutdown.cancel();
            connection_future.await
        }
        _ = reload_token.cancelled() => {
            graceful_shutdown.cancel();
            connection_future.await
        }
    };

    if let Err(error) = connection_result {
        emit_error(
            &handler_state.connection_observability,
            format!("HTTP/1 connection error: {error}"),
        );
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_http2_connection<S>(
    socket: S,
    remote_address: SocketAddr,
    pipeline: Arc<Pipeline<HttpContext>>,
    file_pipeline: Arc<Pipeline<HttpFileContext>>,
    error_pipeline: Arc<Pipeline<HttpErrorContext>>,
    config_resolver: Arc<ThreeStageResolver>,
    local_address: SocketAddr,
    hinted_hostname: Option<String>,
    encrypted: bool,
    https_port: Option<u16>,
    connection_options: HttpConnectionOptions,
    observability_resolver: Arc<RadixTree<Vec<ObservabilityProviderEntry>>>,
    connection_observability: CompositeEventSink,
    shutdown_token: CancellationToken,
    reload_token: CancellationToken,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + 'static,
{
    let graceful_shutdown = CancellationToken::new();
    let handler_state = Arc::new(RequestHandlerState {
        pipeline,
        file_pipeline,
        error_pipeline,
        config_resolver,
        connection_observability,
        observability_resolver,
        local_address,
        remote_address,
        hinted_hostname,
        encrypted,
        https_port,
    });
    let mut connection_future = Box::pin(
        Http2::new(socket, build_http2_options(&connection_options))
            .graceful_shutdown_token(graceful_shutdown.clone())
            .handle(build_request_handler(handler_state.clone())),
    );
    let connection_result = tokio::select! {
        result = &mut connection_future => result,
        _ = shutdown_token.cancelled() => {
            graceful_shutdown.cancel();
            connection_future.await
        }
        _ = reload_token.cancelled() => {
            graceful_shutdown.cancel();
            connection_future.await
        }
    };

    if let Err(error) = connection_result {
        emit_error(
            &handler_state.connection_observability,
            format!("HTTP/2 connection error: {error}"),
        );
    }
}

#[inline]
fn build_http1_options(connection_options: &HttpConnectionOptions) -> Http1Options {
    Http1Options::default().enable_early_hints(connection_options.h1_enable_early_hints)
}

#[inline]
fn build_http2_options(connection_options: &HttpConnectionOptions) -> Http2Options {
    let mut options = Http2Options::default();
    let builder = options.h2_builder();
    if let Some(initial_window_size) = connection_options.h2.initial_window_size {
        builder.initial_window_size(initial_window_size);
    }
    if let Some(max_frame_size) = connection_options.h2.max_frame_size {
        builder.max_frame_size(max_frame_size);
    }
    if let Some(max_concurrent_streams) = connection_options.h2.max_concurrent_streams {
        builder.max_concurrent_streams(max_concurrent_streams);
    }
    if let Some(max_header_list_size) = connection_options.h2.max_header_list_size {
        builder.max_header_list_size(max_header_list_size);
    }
    if connection_options.h2.enable_connect_protocol {
        builder.enable_connect_protocol();
    }
    options
}

#[inline]
fn build_request_handler(
    state: Arc<RequestHandlerState>,
) -> impl Fn(Request<vibeio_http::Incoming>) -> RequestHandlerFuture {
    move |request: Request<vibeio_http::Incoming>| {
        let state = Arc::clone(&state);
        Box::pin(async move {
            let hostname = request_hostname_for_lookup(&request, state.hinted_hostname.as_deref());
            // Reuse the connection-level observability sink; only re-resolve if the request
            // Host header differs from the SNI hostname (virtual hosting on the same connection).
            let request_observability = match hostname.as_deref() {
                Some(host) if state.hinted_hostname.as_deref() != Some(host) => {
                    resolve_observability_sink_with_normalized(
                        &state.observability_resolver,
                        &state.connection_observability,
                        state.local_address.ip(),
                        Some(host),
                    )
                }
                _ => state.connection_observability.clone(),
            };
            let (parts, body) = request.into_parts();
            let request = Request::from_parts(parts, body.boxed_unsync());
            request_handler(
                request,
                state.pipeline.clone(),
                state.file_pipeline.clone(),
                state.error_pipeline.clone(),
                state.config_resolver.clone(),
                state.local_address,
                state.remote_address,
                hostname,
                state.encrypted,
                state.https_port,
                request_observability,
            )
            .await
        })
    }
}

#[inline]
fn resolve_http_connection_options(
    resolver: &RadixTree<HttpConnectionOptions>,
    ip: IpAddr,
    hostname: Option<&str>,
) -> HttpConnectionOptions {
    let normalized_hostname = hostname.and_then(normalize_host_for_lookup);
    match normalized_hostname.as_deref() {
        Some(hostname) => resolver
            .lookup_ip_and_hostname(ip, hostname)
            .or_else(|| resolver.lookup_ip(ip)),
        None => resolver.lookup_ip(ip),
    }
    .or_else(|| resolver.root_data())
    .unwrap_or_default()
}

/// Initialize event sinks from provider entries (called at request/connection time).
#[inline]
fn initialize_sinks_from_providers(
    entries: &[ObservabilityProviderEntry],
) -> Vec<Arc<dyn EventSink>> {
    let mut sinks = Vec::with_capacity(entries.len());
    for (provider, log_config) in entries {
        let mut ctx = ObservabilityContext {
            log_config: log_config.clone(),
            sink: None,
        };
        if let Ok(()) = provider.execute(&mut ctx) {
            if let Some(sink) = ctx.sink {
                sinks.push(sink);
            }
        }
    }
    sinks
}

/// Helper to resolve root-level observability sinks (for pre-connection errors).
#[inline]
fn resolve_root_observability_sink(
    observability_resolver: &RadixTree<Vec<ObservabilityProviderEntry>>,
) -> CompositeEventSink {
    let sinks = observability_resolver
        .root_data()
        .map(|e| initialize_sinks_from_providers(&e))
        .unwrap_or_default();
    CompositeEventSink::new(sinks)
}

#[inline]
fn resolve_observability_sink(
    observability_resolver: &RadixTree<Vec<ObservabilityProviderEntry>>,
    ip: Option<IpAddr>,
    hostname: Option<&str>,
    fallback: &CompositeEventSink,
) -> CompositeEventSink {
    // Normalize hostname once to avoid redundant string operations
    let normalized_hostname = hostname.and_then(normalize_host_for_lookup);
    resolve_observability_sink_with_normalized_and_resolver(
        observability_resolver,
        ip,
        normalized_hostname.as_deref(),
        fallback,
    )
}

/// Optimized variant that accepts a pre-normalized hostname.
/// When the hostname matches the connection's SNI, returns the connection-level
/// sink directly without any radix tree lookup. Otherwise performs a fresh lookup.
#[inline]
fn resolve_observability_sink_with_normalized(
    observability_resolver: &RadixTree<Vec<ObservabilityProviderEntry>>,
    connection_observability: &CompositeEventSink,
    ip: IpAddr,
    hostname: Option<&str>,
) -> CompositeEventSink {
    // Fast path: if hostname matches SNI, return connection-level sink
    if hostname.is_none() {
        return connection_observability.clone();
    }

    // Slow path: hostname differs from SNI, need to lookup
    let entries = observability_resolver.lookup_ip_and_hostname(ip, hostname.unwrap());
    let sinks = entries
        .map(|e| initialize_sinks_from_providers(&e))
        .unwrap_or_default();
    if sinks.is_empty() {
        connection_observability.clone()
    } else {
        CompositeEventSink::new(sinks)
    }
}

/// Core implementation that performs the radix tree lookup with pre-normalized values.
#[inline]
fn resolve_observability_sink_with_normalized_and_resolver(
    observability_resolver: &RadixTree<Vec<ObservabilityProviderEntry>>,
    ip: Option<IpAddr>,
    hostname: Option<&str>,
    fallback: &CompositeEventSink,
) -> CompositeEventSink {
    let entries = match (ip, hostname) {
        (Some(ip), Some(hostname)) => observability_resolver.lookup_ip_and_hostname(ip, hostname),
        (Some(ip), None) => observability_resolver.lookup_ip(ip),
        (None, Some(hostname)) => observability_resolver.lookup_hostname(hostname),
        (None, None) => observability_resolver.root_data(),
    };

    let sinks = entries
        .map(|e| initialize_sinks_from_providers(&e))
        .unwrap_or_default();
    if sinks.is_empty() {
        fallback.clone()
    } else {
        CompositeEventSink::new(sinks)
    }
}

#[inline]
fn request_hostname_for_lookup<B>(
    request: &Request<B>,
    hinted_hostname: Option<&str>,
) -> Option<String> {
    request
        .headers()
        .get(http::header::HOST)
        .and_then(|value| value.to_str().ok())
        .and_then(normalize_host_for_lookup)
        .or_else(|| {
            request
                .uri()
                .authority()
                .map(|authority| authority.as_str())
                .and_then(normalize_host_for_lookup)
        })
        .or_else(|| hinted_hostname.map(std::borrow::ToOwned::to_owned))
}

#[inline]
fn emit_error(observability: &CompositeEventSink, message: impl Into<String>) {
    observability.emit(Event::Log(LogEvent {
        level: LogLevel::Error,
        message: message.into(),
        target: LOG_TARGET,
    }));
}

#[inline]
fn normalize_host_for_lookup(host: &str) -> Option<String> {
    let host = host.trim();
    if host.is_empty() {
        return None;
    }

    if let Some(rest) = host.strip_prefix('[') {
        let end = rest.find(']')?;
        return Some(rest[..end].to_ascii_lowercase());
    }

    let host_without_port = match host.rsplit_once(':') {
        Some((candidate, port))
            if !candidate.contains(':') && port.chars().all(|c| c.is_ascii_digit()) =>
        {
            candidate
        }
        _ => host,
    };
    let normalized = host_without_port.trim().trim_end_matches('.');
    if normalized.is_empty() {
        return None;
    }

    Some(normalized.to_ascii_lowercase())
}

#[derive(Debug)]
pub struct NoCertResolver;

impl rustls::server::ResolvesServerCert for NoCertResolver {
    #[inline]
    fn resolve(
        &self,
        _client_hello: rustls::server::ClientHello<'_>,
    ) -> Option<Arc<rustls::sign::CertifiedKey>> {
        None
    }
}
