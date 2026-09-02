use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use ferron_core::pipeline::Pipeline;
use ferron_http::{HttpContext, HttpErrorContext, HttpFileContext};
use ferron_observability::{
    CompositeEventSink, Event, LogAttributeValue, MetricAttributeValue, MetricEvent, MetricType,
    MetricValue,
};
use ferron_tls::TlsConnectionParams;
use tokio_util::sync::CancellationToken;
use zincio_http::{Http1, Http1Options, Http2, Http2Options, HttpProtocol};

use crate::config::ThreeStageResolver;
use crate::server::tls_resolve::RadixTree;

use super::common::*;

// Backlog size of -1 is supported on macOS, *BSD and Windows (Winsock)
#[cfg(any(
    windows,
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd",
    target_os = "dragonfly"
))]
pub const DEFAULT_SOCKET_BACKLOG: i32 = -1;
// Otherwise, use the default backlog size of 4096
#[cfg(not(any(
    windows,
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd",
    target_os = "dragonfly"
)))]
pub const DEFAULT_SOCKET_BACKLOG: i32 = 4096;

/// Distinguishes TCP connections (with peer/listen addresses) from Unix domain
/// socket connections (with a filesystem path).
#[derive(Clone, Debug)]
pub(crate) enum ConnectionAddr {
    Tcp {
        remote_address: SocketAddr,
        local_address: SocketAddr,
    },
    Unix {
        unix_socket_path: PathBuf,
    },
}

impl ConnectionAddr {
    fn local_ip(&self) -> Option<std::net::IpAddr> {
        match self {
            ConnectionAddr::Tcp { local_address, .. } => Some(local_address.ip()),
            ConnectionAddr::Unix { .. } => None,
        }
    }
}

#[inline]
pub(crate) fn emit_connection_error_metric(
    observability: &CompositeEventSink,
    transport: &'static str,
    stage: &'static str,
) {
    observability.emit(Event::Metric(MetricEvent {
        name: "ferron.http.server.connection_errors",
        attributes: vec![
            (
                "network.transport",
                MetricAttributeValue::StaticStr(transport),
            ),
            (
                "ferron.connection.stage",
                MetricAttributeValue::StaticStr(stage),
            ),
        ],
        ty: MetricType::Counter,
        value: MetricValue::U64(1),
        unit: Some("{error}"),
        description: Some("Number of connection lifecycle errors by transport and stage."),
        trace_context: None,
    }));
}

#[inline]
pub(crate) fn build_http1_options(connection_options: &HttpConnectionOptions) -> Http1Options {
    Http1Options::default().enable_early_hints(connection_options.h1_enable_early_hints)
}

#[inline]
pub(crate) fn build_http2_options(connection_options: &HttpConnectionOptions) -> Http2Options {
    let mut options = Http2Options::default();
    if let Some(initial_window_size) = connection_options.h2.initial_window_size {
        options = options.initial_connection_window_size(initial_window_size);
        options = options.initial_stream_window_size(initial_window_size);
    }
    if let Some(max_frame_size) = connection_options.h2.max_frame_size {
        options = options.max_frame_size(max_frame_size);
    }
    if let Some(max_concurrent_streams) = connection_options.h2.max_concurrent_streams {
        options = options.max_concurrent_streams(max_concurrent_streams);
    }
    if let Some(max_header_list_size) = connection_options.h2.max_header_list_size {
        options = options.max_header_list_size(max_header_list_size);
    }
    options = options.enable_connect_protocol(connection_options.h2.enable_connect_protocol);
    options
}

#[inline]
pub(crate) fn build_bad_request_handler(
    state: Arc<RequestHandlerState>,
) -> impl Fn(bool) -> RequestHandlerFuture {
    move |is_timeout: bool| {
        let state = Arc::clone(&state);
        Box::pin(async move {
            let request_observability = state.connection_observability.clone();
            crate::handler::bad_request_handler(
                is_timeout,
                state.local_address,
                state.remote_address,
                state.unix_socket_path.clone(),
                state.error_pipeline.clone(),
                request_observability,
                state.host_control_plane_metadata.clone(),
                state.host_control_plane_span_links.clone(),
            )
            .await
        })
    }
}

/// Build the shared `RequestHandlerState` from transport-specific parameters.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_request_handler_state(
    conn_addr: &ConnectionAddr,
    pipeline: Arc<Pipeline<HttpContext>>,
    file_pipeline: Arc<Pipeline<HttpFileContext>>,
    error_pipeline: Arc<Pipeline<HttpErrorContext>>,
    config_resolver: Arc<ThreeStageResolver>,
    connection_observability: CompositeEventSink,
    observability_resolver: Arc<RadixTree<Vec<ObservabilityProviderEntry>>>,
    hinted_hostname: Option<String>,
    encrypted: bool,
    https_port: Option<u16>,
    http3_alt_svc: bool,
    connection_options: &HttpConnectionOptions,
    peer_identity: Option<Vec<rustls::pki_types::CertificateDer<'static>>>,
    tls_params: Option<TlsConnectionParams>,
) -> Arc<RequestHandlerState> {
    let (local_address, remote_address, unix_socket_path) = match conn_addr {
        ConnectionAddr::Tcp {
            remote_address,
            local_address,
        } => (Some(*local_address), Some(*remote_address), None),
        ConnectionAddr::Unix { unix_socket_path } => (None, None, Some(unix_socket_path.clone())),
    };
    let host_control_plane_metadata = resolve_host_control_plane_metadata(
        &observability_resolver,
        conn_addr.local_ip(),
        hinted_hostname.as_deref(),
    );
    let host_control_plane_span_links = resolve_host_control_plane_span_links(
        &observability_resolver,
        conn_addr.local_ip(),
        hinted_hostname.as_deref(),
    );
    Arc::new(RequestHandlerState {
        pipeline,
        file_pipeline,
        error_pipeline,
        config_resolver,
        connection_observability,
        observability_resolver,
        local_address,
        remote_address,
        unix_socket_path,
        hinted_hostname,
        encrypted,
        https_port,
        http3_alt_svc,
        timeout_duration: connection_options.timeout,
        peer_identity,
        tls_params,
        host_control_plane_metadata,
        host_control_plane_span_links,
    })
}

/// Format connection-error log attributes based on the transport type.
fn connection_error_attrs(
    conn_addr: &ConnectionAddr,
    error: &impl std::fmt::Display,
    error_type: &'static str,
) -> Vec<(&'static str, LogAttributeValue)> {
    let mut attrs = vec![
        ("error.type", LogAttributeValue::String(error_type.into())),
        (
            "error.message",
            LogAttributeValue::String(error.to_string()),
        ),
    ];
    match conn_addr {
        ConnectionAddr::Tcp {
            remote_address,
            local_address,
        } => {
            attrs.push((
                "client.address",
                LogAttributeValue::String(remote_address.ip().to_canonical().to_string()),
            ));
            attrs.push((
                "client.port",
                LogAttributeValue::I64(remote_address.port() as i64),
            ));
            attrs.push((
                "server.address",
                LogAttributeValue::String(local_address.ip().to_canonical().to_string()),
            ));
            attrs.push((
                "server.port",
                LogAttributeValue::I64(local_address.port() as i64),
            ));
        }
        ConnectionAddr::Unix { unix_socket_path } => {
            attrs.push((
                "server.address",
                LogAttributeValue::String(unix_socket_path.display().to_string()),
            ));
        }
    }
    attrs
}

#[allow(clippy::too_many_arguments)]
#[inline]
pub(crate) async fn handle_http1_connection<S>(
    socket: S,
    conn_addr: ConnectionAddr,
    pipeline: Arc<Pipeline<HttpContext>>,
    file_pipeline: Arc<Pipeline<HttpFileContext>>,
    error_pipeline: Arc<Pipeline<HttpErrorContext>>,
    config_resolver: Arc<ThreeStageResolver>,
    hinted_hostname: Option<String>,
    encrypted: bool,
    https_port: Option<u16>,
    connection_options: HttpConnectionOptions,
    observability_resolver: Arc<RadixTree<Vec<ObservabilityProviderEntry>>>,
    connection_observability: CompositeEventSink,
    shutdown_token: CancellationToken,
    reload_token: CancellationToken,
    http3_alt_svc: bool,
    peer_identity: Option<Vec<rustls::pki_types::CertificateDer<'static>>>,
    tls_params: Option<TlsConnectionParams>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + 'static,
{
    let graceful_shutdown = CancellationToken::new();
    let handler_state = build_request_handler_state(
        &conn_addr,
        pipeline,
        file_pipeline,
        error_pipeline,
        config_resolver,
        connection_observability,
        observability_resolver,
        hinted_hostname,
        encrypted,
        https_port,
        http3_alt_svc,
        &connection_options,
        peer_identity,
        tls_params,
    );
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
        let error_type = match conn_addr {
            ConnectionAddr::Tcp { .. } => "tcp_connection_error",
            ConnectionAddr::Unix { .. } => "unix_connection_error",
        };
        let error_msg = match &conn_addr {
            ConnectionAddr::Tcp { .. } => {
                format!("HTTP/1 connection error: {error}")
            }
            ConnectionAddr::Unix { unix_socket_path } => {
                format!(
                    "HTTP/1 connection error on unix:{}: {error}",
                    unix_socket_path.display()
                )
            }
        };
        emit_error(
            &handler_state.connection_observability,
            error_msg,
            connection_error_attrs(&conn_addr, &error, error_type),
        );
    }
}

#[cfg(not(target_os = "linux"))]
#[allow(clippy::too_many_arguments)]
#[inline]
pub(crate) async fn handle_http1_connection_zerocopy<S>(
    socket: S,
    conn_addr: ConnectionAddr,
    pipeline: Arc<Pipeline<HttpContext>>,
    file_pipeline: Arc<Pipeline<HttpFileContext>>,
    error_pipeline: Arc<Pipeline<HttpErrorContext>>,
    config_resolver: Arc<ThreeStageResolver>,
    hinted_hostname: Option<String>,
    encrypted: bool,
    https_port: Option<u16>,
    connection_options: HttpConnectionOptions,
    observability_resolver: Arc<RadixTree<Vec<ObservabilityProviderEntry>>>,
    connection_observability: CompositeEventSink,
    shutdown_token: CancellationToken,
    reload_token: CancellationToken,
    http3_alt_svc: bool,
    peer_identity: Option<Vec<rustls::pki_types::CertificateDer<'static>>>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + 'static,
{
    handle_http1_connection(
        socket,
        conn_addr,
        pipeline,
        file_pipeline,
        error_pipeline,
        config_resolver,
        hinted_hostname,
        encrypted,
        https_port,
        connection_options,
        observability_resolver,
        connection_observability,
        shutdown_token,
        reload_token,
        http3_alt_svc,
        peer_identity,
        None,
    )
    .await
}

#[cfg(target_os = "linux")]
#[allow(clippy::too_many_arguments)]
#[inline]
pub(crate) async fn handle_http1_connection_zerocopy<S>(
    socket: S,
    conn_addr: ConnectionAddr,
    pipeline: Arc<Pipeline<HttpContext>>,
    file_pipeline: Arc<Pipeline<HttpFileContext>>,
    error_pipeline: Arc<Pipeline<HttpErrorContext>>,
    config_resolver: Arc<ThreeStageResolver>,
    hinted_hostname: Option<String>,
    encrypted: bool,
    https_port: Option<u16>,
    connection_options: HttpConnectionOptions,
    observability_resolver: Arc<RadixTree<Vec<ObservabilityProviderEntry>>>,
    connection_observability: CompositeEventSink,
    shutdown_token: CancellationToken,
    reload_token: CancellationToken,
    http3_alt_svc: bool,
    peer_identity: Option<Vec<rustls::pki_types::CertificateDer<'static>>>,
) where
    for<'a> S: tokio::io::AsyncRead
        + tokio::io::AsyncWrite
        + zincio::io::AsInnerRawHandle<'a>
        + Unpin
        + 'static,
{
    let graceful_shutdown = CancellationToken::new();
    let handler_state = build_request_handler_state(
        &conn_addr,
        pipeline,
        file_pipeline,
        error_pipeline,
        config_resolver,
        connection_observability,
        observability_resolver,
        hinted_hostname,
        encrypted,
        https_port,
        http3_alt_svc,
        &connection_options,
        peer_identity,
        None,
    );
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
        let error_type = match conn_addr {
            ConnectionAddr::Tcp { .. } => "tcp_connection_error",
            ConnectionAddr::Unix { .. } => "unix_connection_error",
        };
        let error_msg = match &conn_addr {
            ConnectionAddr::Tcp { .. } => {
                format!("HTTP/1 connection error: {error}")
            }
            ConnectionAddr::Unix { unix_socket_path } => {
                format!(
                    "HTTP/1 connection error on unix:{}: {error}",
                    unix_socket_path.display()
                )
            }
        };
        emit_error(
            &handler_state.connection_observability,
            error_msg,
            connection_error_attrs(&conn_addr, &error, error_type),
        );
    }
}

#[allow(clippy::too_many_arguments)]
#[inline]
pub(crate) async fn handle_http2_connection<S>(
    socket: S,
    conn_addr: ConnectionAddr,
    pipeline: Arc<Pipeline<HttpContext>>,
    file_pipeline: Arc<Pipeline<HttpFileContext>>,
    error_pipeline: Arc<Pipeline<HttpErrorContext>>,
    config_resolver: Arc<ThreeStageResolver>,
    hinted_hostname: Option<String>,
    encrypted: bool,
    https_port: Option<u16>,
    connection_options: HttpConnectionOptions,
    observability_resolver: Arc<RadixTree<Vec<ObservabilityProviderEntry>>>,
    connection_observability: CompositeEventSink,
    shutdown_token: CancellationToken,
    reload_token: CancellationToken,
    http3_alt_svc: bool,
    peer_identity: Option<Vec<rustls::pki_types::CertificateDer<'static>>>,
    tls_params: Option<TlsConnectionParams>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + 'static,
{
    let graceful_shutdown = CancellationToken::new();
    let handler_state = build_request_handler_state(
        &conn_addr,
        pipeline,
        file_pipeline,
        error_pipeline,
        config_resolver,
        connection_observability,
        observability_resolver,
        hinted_hostname,
        encrypted,
        https_port,
        http3_alt_svc,
        &connection_options,
        peer_identity,
        tls_params,
    );
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
        let error_type = match conn_addr {
            ConnectionAddr::Tcp { .. } => "tcp_connection_error",
            ConnectionAddr::Unix { .. } => "unix_connection_error",
        };
        let error_msg = match &conn_addr {
            ConnectionAddr::Tcp { .. } => {
                format!("HTTP/2 connection error: {error}")
            }
            ConnectionAddr::Unix { unix_socket_path } => {
                format!(
                    "HTTP/2 connection error on unix:{}: {error}",
                    unix_socket_path.display()
                )
            }
        };
        emit_error(
            &handler_state.connection_observability,
            error_msg,
            connection_error_attrs(&conn_addr, &error, error_type),
        );
    }
}
