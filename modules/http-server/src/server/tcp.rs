//! TCP listener and connection handling

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use ferron_core::pipeline::Pipeline;
use ferron_core::runtime::Runtime;
use ferron_core::shutdown::RELOAD_TOKEN;
use ferron_core::{log_error, log_info};
use ferron_http::HttpContext;
use ferron_observability::{CompositeEventSink, Event, EventSink, LogEvent, LogLevel};
use http::Request;
use http_body_util::{combinators::UnsyncBoxBody, BodyExt};
use rustls::server::Acceptor;
use tokio_util::sync::CancellationToken;
use vibeio_http::{Http1, Http1Options, Http2, Http2Options, HttpProtocol};

use crate::config::ThreeStageResolver;
use crate::handler::request_handler;
use crate::server::tls_resolve::{RadixTree, TlsResolverRadixTree};

const LOG_TARGET: &str = "ferron-http-server";
type ResponseBody = UnsyncBoxBody<bytes::Bytes, io::Error>;
type RequestHandlerFuture =
    Pin<Box<dyn std::future::Future<Output = Result<http::Response<ResponseBody>, io::Error>>>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TcpListenerOptions {
    pub address: SocketAddr,
    pub send_buffer_size: Option<usize>,
    pub recv_buffer_size: Option<usize>,
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
        pipeline: Arc<Pipeline<HttpContext>>,
        runtime: &mut Runtime,
        config_resolver: Arc<ThreeStageResolver>,
        tls_resolver: Option<Arc<TlsResolverRadixTree>>,
        http_connection_options_resolver: Arc<RadixTree<HttpConnectionOptions>>,
        observability_resolver: Arc<RadixTree<Vec<Arc<dyn EventSink>>>>,
    ) -> Result<Self, std::io::Error> {
        let listener = build_tcp_listener(
            options.address,
            (options.send_buffer_size, options.recv_buffer_size),
        )?;

        log_info!("HTTP server listening on {}", options.address);

        let cancel_token = Arc::new(CancellationToken::new());

        let pipeline_clone = pipeline.clone();
        let cancel_token_clone = cancel_token.clone();

        runtime.spawn_primary_task(move || {
            let new_listener_result = listener.try_clone();
            let cancel_token = cancel_token_clone.clone();
            let config_resolver = config_resolver.clone();
            let tls_resolver = tls_resolver.clone();
            let http_connection_options_resolver = http_connection_options_resolver.clone();
            let observability_resolver = observability_resolver.clone();
            let global_observability = observability_resolver
                .root_data()
                .map(CompositeEventSink::new)
                .unwrap_or(CompositeEventSink::new(vec![]));
            let pipeline = pipeline_clone.clone();
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
                        emit_error(
                            &global_observability,
                            "Failed to convert socket to poll-based I/O",
                        );
                        continue;
                    };

                    let pipeline = pipeline.clone();
                    let config_resolver = config_resolver.clone();
                    let tls_resolver = tls_resolver.clone();
                    let http_connection_options_resolver =
                        http_connection_options_resolver.clone();
                    let observability_resolver = observability_resolver.clone();
                    let global_observability = global_observability.clone();
                    let connection_cancel_token = cancel_token.clone();
                    vibeio::spawn(async move {
                        let Ok(local_addr) = socket.local_addr() else {
                            emit_error(&global_observability, "Failed to get local address");
                            return;
                        };
                        let ip_observability = resolve_observability_sink(
                            &observability_resolver,
                            Some(local_addr.ip()),
                            None,
                            &global_observability,
                        );

                        if let Some(tls_resolver) = tls_resolver {
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
                                &http_connection_options_resolver,
                                local_addr.ip(),
                                hinted_hostname.as_deref(),
                            );
                            let resolver = if let Some(sni) = sni.as_deref() {
                                tls_resolver.lookup_ip_and_hostname(local_addr.ip(), sni)
                            } else {
                                tls_resolver.lookup_ip(local_addr.ip())
                            };
                            if let Some(resolver) = resolver {
                                let Ok(tls_stream_option) =
                                    resolver.handshake(start_handshake).await
                                else {
                                    let tls_observability = resolve_observability_sink(
                                        &observability_resolver,
                                        Some(local_addr.ip()),
                                        hinted_hostname.as_deref(),
                                        &ip_observability,
                                    );
                                    emit_error(&tls_observability, "Failed to start TLS handshake");
                                    return;
                                };
                                let tls_observability = resolve_observability_sink(
                                    &observability_resolver,
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
                                            pipeline,
                                            config_resolver,
                                            local_addr.ip(),
                                            hinted_hostname,
                                            connection_options,
                                            observability_resolver,
                                            tls_observability,
                                            connection_cancel_token.as_ref().clone(),
                                        )
                                        .await;
                                    } else if connection_options.protocols.supports_http1() {
                                        handle_http1_connection(
                                            tls_stream,
                                            pipeline,
                                            config_resolver,
                                            local_addr.ip(),
                                            hinted_hostname,
                                            true,
                                            connection_options,
                                            observability_resolver,
                                            tls_observability,
                                            connection_cancel_token.as_ref().clone(),
                                        )
                                        .await;
                                    } else {
                                        emit_error(
                                            &tls_observability,
                                            "TLS connection did not negotiate a supported HTTP protocol",
                                        );
                                    }
                                }
                            }
                        } else {
                            let connection_options = resolve_http_connection_options(
                                &http_connection_options_resolver,
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
                                pipeline,
                                config_resolver,
                                local_addr.ip(),
                                None,
                                false,
                                connection_options,
                                observability_resolver,
                                ip_observability,
                                connection_cancel_token.as_ref().clone(),
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
    listener_socket.listen(1024)?;

    Ok(listener_socket.into())
}

async fn handle_http1_connection<S>(
    socket: S,
    pipeline: Arc<Pipeline<HttpContext>>,
    config_resolver: Arc<ThreeStageResolver>,
    local_ip: IpAddr,
    hinted_hostname: Option<String>,
    is_tls: bool,
    connection_options: HttpConnectionOptions,
    observability_resolver: Arc<RadixTree<Vec<Arc<dyn EventSink>>>>,
    default_observability: CompositeEventSink,
    shutdown_token: CancellationToken,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + 'static,
{
    let graceful_shutdown = &**RELOAD_TOKEN.load();
    let mut connection_future = Box::pin(
        Http1::new(socket, build_http1_options(&connection_options))
            .graceful_shutdown_token(graceful_shutdown.clone())
            .handle(build_request_handler(
                pipeline,
                config_resolver,
                local_ip,
                hinted_hostname,
                is_tls,
                observability_resolver,
                default_observability.clone(),
            )),
    );
    let connection_result = tokio::select! {
        result = &mut connection_future => result,
        _ = shutdown_token.cancelled() => {
            graceful_shutdown.cancel();
            connection_future.await
        }
    };

    if let Err(error) = connection_result {
        emit_error(
            &default_observability,
            format!("HTTP/1 connection error: {error}"),
        );
    }
}

async fn handle_http2_connection<S>(
    socket: S,
    pipeline: Arc<Pipeline<HttpContext>>,
    config_resolver: Arc<ThreeStageResolver>,
    local_ip: IpAddr,
    hinted_hostname: Option<String>,
    connection_options: HttpConnectionOptions,
    observability_resolver: Arc<RadixTree<Vec<Arc<dyn EventSink>>>>,
    default_observability: CompositeEventSink,
    shutdown_token: CancellationToken,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + 'static,
{
    let graceful_shutdown = &**RELOAD_TOKEN.load();
    let mut connection_future = Box::pin(
        Http2::new(socket, build_http2_options(&connection_options))
            .graceful_shutdown_token(graceful_shutdown.clone())
            .handle(build_request_handler(
                pipeline,
                config_resolver,
                local_ip,
                hinted_hostname,
                true,
                observability_resolver,
                default_observability.clone(),
            )),
    );
    let connection_result = tokio::select! {
        result = &mut connection_future => result,
        _ = shutdown_token.cancelled() => {
            graceful_shutdown.cancel();
            connection_future.await
        }
    };

    if let Err(error) = connection_result {
        emit_error(
            &default_observability,
            format!("HTTP/2 connection error: {error}"),
        );
    }
}

fn build_http1_options(connection_options: &HttpConnectionOptions) -> Http1Options {
    Http1Options::default().enable_early_hints(connection_options.h1_enable_early_hints)
}

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

fn build_request_handler(
    pipeline: Arc<Pipeline<HttpContext>>,
    config_resolver: Arc<ThreeStageResolver>,
    local_ip: IpAddr,
    hinted_hostname: Option<String>,
    is_tls: bool,
    observability_resolver: Arc<RadixTree<Vec<Arc<dyn EventSink>>>>,
    default_observability: CompositeEventSink,
) -> impl Fn(Request<vibeio_http::Incoming>) -> RequestHandlerFuture {
    move |request: Request<vibeio_http::Incoming>| {
        let pipeline = pipeline.clone();
        let config_resolver = config_resolver.clone();
        let hinted_hostname = hinted_hostname.clone();
        let observability_resolver = observability_resolver.clone();
        let default_observability = default_observability.clone();
        Box::pin(async move {
            let hostname = request_hostname_for_lookup(&request, hinted_hostname.as_deref());
            let request_observability = resolve_observability_sink(
                &observability_resolver,
                Some(local_ip),
                hostname.as_deref(),
                &default_observability,
            );
            let (parts, body) = request.into_parts();
            let request = Request::from_parts(parts, body.boxed_unsync());
            request_handler(
                request,
                pipeline,
                config_resolver,
                local_ip,
                hostname,
                is_tls,
                request_observability,
            )
            .await
        })
    }
}

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

fn resolve_observability_sink(
    observability_resolver: &RadixTree<Vec<Arc<dyn EventSink>>>,
    ip: Option<IpAddr>,
    hostname: Option<&str>,
    fallback: &CompositeEventSink,
) -> CompositeEventSink {
    let normalized_hostname = hostname.and_then(normalize_host_for_lookup);
    let sinks = match (ip, normalized_hostname.as_deref()) {
        (Some(ip), Some(hostname)) => observability_resolver.lookup_ip_and_hostname(ip, hostname),
        (Some(ip), None) => observability_resolver.lookup_ip(ip),
        (None, Some(hostname)) => observability_resolver.lookup_hostname(hostname),
        (None, None) => observability_resolver.root_data(),
    };

    sinks
        .map(CompositeEventSink::new)
        .unwrap_or_else(|| fallback.clone())
}

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

fn emit_error(observability: &CompositeEventSink, message: impl Into<String>) {
    observability.emit(Event::Log(LogEvent {
        level: LogLevel::Error,
        message: message.into(),
        target: LOG_TARGET,
    }));
}

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
