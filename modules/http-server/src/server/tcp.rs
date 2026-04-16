//! TCP listener and connection handling

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use ferron_core::pipeline::Pipeline;
use ferron_core::runtime::Runtime;
use ferron_core::{log_error, log_info};
use ferron_http::{HttpContext, HttpErrorContext, HttpFileContext};
use ferron_observability::CompositeEventSink;
use rustls::server::Acceptor;
use tokio_util::sync::CancellationToken;
use vibeio_http::{Http1, Http1Options, Http2, Http2Options, HttpProtocol};

use crate::config::ThreeStageResolver;
use crate::handler::bad_request_handler;
use crate::server::tls_resolve::RadixTree;
use crate::util::proxy_protocol::read_proxy_header;

use super::common::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TcpListenerOptions {
    pub address: SocketAddr,
    pub send_buffer_size: Option<usize>,
    pub recv_buffer_size: Option<usize>,
    pub backlog: Option<i32>,
}

pub struct TcpListenerHandle {
    cancel_token: Arc<CancellationToken>,
}

impl TcpListenerHandle {
    pub fn new(
        options: TcpListenerOptions,
        http3_alt_svc: bool,
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
                                            http3_alt_svc
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
                                            http3_alt_svc
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
                            handle_http1_connection_zerocopy(
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
                                http3_alt_svc
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
    listener_socket.listen(backlog.unwrap_or(-1))?;

    Ok(listener_socket.into())
}

#[cfg(not(target_os = "linux"))]
#[allow(clippy::too_many_arguments)]
#[inline]
async fn handle_http1_connection_zerocopy<S>(
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
    http3_alt_svc: bool,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + 'static,
{
    handle_http1_connection(
        socket,
        remote_address,
        pipeline,
        file_pipeline,
        error_pipeline,
        config_resolver,
        local_address,
        hinted_hostname,
        encrypted,
        https_port,
        connection_options,
        observability_resolver,
        connection_observability,
        shutdown_token,
        reload_token,
        http3_alt_svc,
    )
    .await
}

#[cfg(target_os = "linux")]
#[allow(clippy::too_many_arguments)]
async fn handle_http1_connection_zerocopy<S>(
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
    http3_alt_svc: bool,
) where
    for<'a> S: tokio::io::AsyncRead
        + tokio::io::AsyncWrite
        + vibeio::io::AsInnerRawHandle<'a>
        + Unpin
        + 'static,
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
        http3_alt_svc,
    });
    let mut connection_future = Box::pin(
        Http1::new(socket, build_http1_options(&connection_options))
            .graceful_shutdown_token(graceful_shutdown.clone())
            .zerocopy()
            .handle_with_error_fn(
                build_request_handler(handler_state.clone()),
                build_bad_request_handler(handler_state.clone()),
            ),
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
    http3_alt_svc: bool,
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
        http3_alt_svc,
    });
    let mut connection_future = Box::pin(
        Http1::new(socket, build_http1_options(&connection_options))
            .graceful_shutdown_token(graceful_shutdown.clone())
            .handle_with_error_fn(
                build_request_handler(handler_state.clone()),
                build_bad_request_handler(handler_state.clone()),
            ),
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
    http3_alt_svc: bool,
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
        http3_alt_svc,
    });
    let mut connection_future = Box::pin(
        Http2::new(socket, build_http2_options(&connection_options))
            .graceful_shutdown_token(graceful_shutdown.clone())
            .handle_with_error_fn(
                build_request_handler(handler_state.clone()),
                build_bad_request_handler(handler_state.clone()),
            ),
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
fn build_bad_request_handler(
    state: Arc<RequestHandlerState>,
) -> impl Fn(bool) -> RequestHandlerFuture {
    move |is_timeout: bool| {
        let state = Arc::clone(&state);
        Box::pin(async move {
            let request_observability = state.connection_observability.clone();
            bad_request_handler(
                is_timeout,
                state.error_pipeline.clone(),
                request_observability,
            )
            .await
        })
    }
}
