use std::collections::HashMap;
use std::error::Error;
use std::fmt::Display;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
#[cfg(not(feature = "runtime-vibeio"))]
use std::time::SystemTime;

use crate::acme::ACME_TLS_ALPN_NAME;
use crate::config::ServerConfigurations;
use crate::get_value;
use crate::listener_handler_communication::ConnectionData;
use crate::request_handler::request_handler;
#[cfg(feature = "runtime-monoio")]
use crate::util::SendAsyncIo;
use crate::util::{read_proxy_header, MultiCancel};
use arc_swap::ArcSwap;
use async_channel::{Receiver, Sender};
#[cfg(not(feature = "runtime-vibeio"))]
use bytes::{Buf, Bytes};
#[cfg(feature = "runtime-vibeio")]
use core_affinity::CoreId;
use ferron_common::logging::LogMessage;
use http_body_util::BodyExt;
#[cfg(not(feature = "runtime-vibeio"))]
use http_body_util::StreamBody;
#[cfg(not(feature = "runtime-vibeio"))]
use hyper::body::{Frame, Incoming};
#[cfg(not(feature = "runtime-vibeio"))]
use hyper::service::service_fn;
use hyper::Request;
#[cfg(not(feature = "runtime-vibeio"))]
use hyper::Response;
#[cfg(feature = "runtime-tokio")]
use hyper_util::rt::{TokioIo, TokioTimer};
#[cfg(feature = "runtime-monoio")]
use monoio::io::IntoPollIo;
#[cfg(feature = "runtime-monoio")]
use monoio::net::tcp::stream_poll::TcpStreamPoll;
#[cfg(feature = "runtime-monoio")]
use monoio::net::TcpStream;
#[cfg(feature = "runtime-monoio")]
use monoio_compat::hyper::{MonoioExecutor, MonoioIo, MonoioTimer};
use rustls::server::Acceptor;
use rustls::ServerConfig;
#[cfg(feature = "runtime-tokio")]
use tokio::net::TcpStream;
use tokio_rustls::server::TlsStream;
use tokio_rustls::LazyConfigAcceptor;
use tokio_util::sync::CancellationToken;
#[cfg(feature = "runtime-vibeio")]
use vibeio::net::PollTcpStream;
#[cfg(feature = "runtime-vibeio")]
use vibeio::net::TcpStream;

#[cfg(not(feature = "runtime-vibeio"))]
static HTTP3_INVALID_HEADERS: [hyper::header::HeaderName; 5] = [
  hyper::header::HeaderName::from_static("keep-alive"),
  hyper::header::HeaderName::from_static("proxy-connection"),
  hyper::header::TRANSFER_ENCODING,
  hyper::header::TE,
  hyper::header::UPGRADE,
];

/// A struct holding reloadable data for handler threads
#[allow(clippy::type_complexity)]
pub struct ReloadableHandlerData {
  /// ACME TLS-ALPN-01 configurations
  pub acme_tls_alpn_01_configs: Arc<HashMap<(Option<IpAddr>, u16), Arc<ServerConfig>>>,
  /// ACME HTTP-01 resolvers
  pub acme_http_01_resolvers: AcmeHttp01Resolvers,
  /// Server configurations
  pub configurations: Arc<ServerConfigurations>,
  /// TLS configurations
  pub tls_configs: Arc<HashMap<(Option<IpAddr>, u16), Arc<ServerConfig>>>,
  /// Whether HTTP/3 is enabled
  pub http3_enabled: bool,
  /// Whether PROXY protocol is enabled
  pub enable_proxy_protocol: bool,
  /// QUIC TLS configurations
  pub quic_tls_configs: Arc<HashMap<(Option<IpAddr>, u16), Arc<quinn::ServerConfig>>>,
}

type AcmeHttp01Resolvers = Arc<tokio::sync::RwLock<Vec<crate::acme::Http01DataLock>>>;

/// Tokio local executor
#[cfg(feature = "runtime-tokio")]
#[derive(Clone, Copy, Debug)]
pub struct TokioLocalExecutor;

#[cfg(feature = "runtime-tokio")]
impl<F> hyper::rt::Executor<F> for TokioLocalExecutor
where
  F: std::future::Future + 'static,
  F::Output: 'static,
{
  #[inline]
  fn execute(&self, fut: F) {
    tokio::task::spawn_local(fut);
  }
}

/// Creates a HTTP request handler
pub fn create_http_handler(
  reloadable_data: Arc<ArcSwap<ReloadableHandlerData>>,
  rx: Receiver<ConnectionData>,
  enable_uring: Option<bool>,
  io_uring_disabled: Sender<Option<std::io::Error>>,
  multi_cancel: Arc<MultiCancel>,
  #[cfg(feature = "runtime-vibeio")] core_affinity: Option<CoreId>,
) -> Result<(CancellationToken, Sender<()>), Box<dyn Error + Send + Sync>> {
  let shutdown_tx = CancellationToken::new();
  let shutdown_rx = shutdown_tx.clone();
  let (handler_init_tx, listen_error_rx) = async_channel::unbounded();
  let (graceful_tx, graceful_rx) = async_channel::unbounded();
  std::thread::Builder::new()
    .name("Request handler".to_string())
    .spawn(move || {
      #[cfg(feature = "runtime-vibeio")]
      if let Some(affinity) = core_affinity {
        core_affinity::set_for_current(affinity);
      }
      let mut rt = match crate::runtime::Runtime::new_runtime(enable_uring) {
        Ok(rt) => rt,
        Err(error) => {
          handler_init_tx
            .send_blocking(Some(
              anyhow::anyhow!("Can't create async runtime: {error}").into_boxed_dyn_error(),
            ))
            .unwrap_or_default();
          return;
        }
      };
      io_uring_disabled
        .send_blocking(rt.return_io_uring_error())
        .unwrap_or_default();
      rt.run(async move {
        if let Some(error) = http_handler_fn(
          reloadable_data,
          rx,
          &handler_init_tx,
          shutdown_rx,
          graceful_rx,
          multi_cancel,
        )
        .await
        .err()
        {
          handler_init_tx.send(Some(error)).await.unwrap_or_default();
        }
      });
    })?;

  if let Some(error) = listen_error_rx.recv_blocking()? {
    Err(error)?;
  }

  Ok((shutdown_tx, graceful_tx))
}

/// HTTP handler function
#[inline]
async fn http_handler_fn(
  reloadable_data: Arc<ArcSwap<ReloadableHandlerData>>,
  rx: Receiver<ConnectionData>,
  handler_init_tx: &Sender<Option<Box<dyn Error + Send + Sync>>>,
  shutdown_rx: CancellationToken,
  graceful_rx: Receiver<()>,
  multi_cancel: Arc<MultiCancel>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
  handler_init_tx.send(None).await.unwrap_or_default();

  let connections_references = Arc::new(());
  let graceful_shutdown_token = Arc::new(ArcSwap::from_pointee(CancellationToken::new()));
  let graceful_shutdown_token_clone = graceful_shutdown_token.clone();

  let mut graceful_rx_recv_future = Box::pin(async move {
    while graceful_rx.recv().await.is_ok() {
      graceful_shutdown_token_clone
        .swap(Arc::new(CancellationToken::new()))
        .cancel();
    }

    futures_util::future::pending::<()>().await;
  });

  loop {
    let conn_data = crate::runtime::select! {
        biased;

        _ = &mut graceful_rx_recv_future => {
            // This future should be always pending...
            break;
        }
        _ = shutdown_rx.cancelled() => {
            break;
        }
        result = rx.recv() => {
            if let Ok(recv_data) = result {
                recv_data
            } else {
                break;
            }
        }
    };
    let ReloadableHandlerData {
      configurations,
      tls_configs,
      http3_enabled,
      acme_tls_alpn_01_configs,
      acme_http_01_resolvers,
      enable_proxy_protocol,
      quic_tls_configs,
    } = &**reloadable_data.load();
    let quic_tls_configs = quic_tls_configs.clone();
    let configurations = configurations.clone();
    let tls_config = if matches!(
      conn_data.connection,
      crate::listener_handler_communication::Connection::Quic(..)
    ) {
      None
    } else {
      tls_configs
        .get(&(
          Some(conn_data.server_address.ip().to_canonical()),
          conn_data.server_address.port(),
        ))
        .cloned()
        .or_else(|| tls_configs.get(&(None, conn_data.server_address.port())).cloned())
    };
    let acme_tls_alpn_01_config = if matches!(
      conn_data.connection,
      crate::listener_handler_communication::Connection::Quic(..)
    ) {
      None
    } else {
      acme_tls_alpn_01_configs
        .get(&(
          Some(conn_data.server_address.ip().to_canonical()),
          conn_data.server_address.port(),
        ))
        .cloned()
        .or_else(|| {
          acme_tls_alpn_01_configs
            .get(&(None, conn_data.server_address.port()))
            .cloned()
        })
    };
    let acme_http_01_resolvers = acme_http_01_resolvers.clone();
    let connections_references_cloned = connections_references.clone();
    let shutdown_rx_clone = shutdown_rx.clone();
    let http3_enabled = *http3_enabled;
    let enable_proxy_protocol = *enable_proxy_protocol;
    let graceful_shutdown_token = graceful_shutdown_token.load().clone();
    crate::runtime::spawn(async move {
      match conn_data.connection {
        crate::listener_handler_communication::Connection::Tcp(tcp_stream) => {
          // Toggle O_NONBLOCK for TCP stream, when using Monoio.
          // Unset it when io_uring is enabled, and set it otherwise.
          #[cfg(feature = "runtime-monoio")]
          let _ = tcp_stream.set_nonblocking(monoio::utils::is_legacy());
          #[cfg(feature = "runtime-vibeio")]
          let _ = tcp_stream.set_nonblocking(vibeio::util::supports_completion());

          #[cfg(any(feature = "runtime-vibeio", feature = "runtime-monoio"))]
          let tcp_stream = match TcpStream::from_std(tcp_stream) {
            Ok(stream) => stream,
            Err(err) => {
              log_connection_accept_error(&configurations, err).await;
              return;
            }
          };
          let encrypted = tls_config.is_some();
          http_tcp_handler_fn(
            tcp_stream,
            conn_data.client_address,
            conn_data.server_address,
            configurations,
            tls_config,
            http3_enabled && encrypted,
            connections_references_cloned,
            acme_tls_alpn_01_config,
            acme_http_01_resolvers,
            enable_proxy_protocol,
            shutdown_rx_clone,
            graceful_shutdown_token,
          )
          .await;
        }
        crate::listener_handler_communication::Connection::Quic(quic_incoming) => {
          http_quic_handler_fn(
            quic_incoming,
            conn_data.client_address,
            conn_data.server_address,
            configurations,
            quic_tls_configs,
            connections_references_cloned,
            shutdown_rx_clone,
            graceful_shutdown_token,
          )
          .await;
        }
      }
    });
  }

  while Arc::weak_count(&connections_references) > 0 {
    crate::runtime::sleep(Duration::from_millis(100)).await;
  }

  // Wait until all connections are closed, then shut down all the previous handler threads
  multi_cancel.cancel().await;

  Ok(())
}

/// Enum for maybe TLS stream
#[cfg(feature = "runtime-monoio")]
type HttpTcpStream = SendAsyncIo<TcpStreamPoll>;

#[cfg(feature = "runtime-vibeio")]
type HttpTcpStream = PollTcpStream;

#[cfg(feature = "runtime-tokio")]
type HttpTcpStream = TcpStream;

/// Enum for maybe TLS stream
#[allow(clippy::large_enum_variant)]
enum MaybeTlsStream {
  /// TLS stream
  Tls(TlsStream<HttpTcpStream>),

  /// Plain TCP stream
  Plain(HttpTcpStream),
}

#[derive(Clone, Copy, Default)]
struct Http2Settings {
  initial_window_size: Option<u32>,
  max_frame_size: Option<u32>,
  max_concurrent_streams: Option<u32>,
  max_header_list_size: Option<u32>,
  enable_connect_protocol: bool,
}

#[inline]
fn get_http2_settings(configurations: &ServerConfigurations) -> Http2Settings {
  let global_configuration = configurations.find_global_configuration();

  Http2Settings {
    initial_window_size: global_configuration
      .as_deref()
      .and_then(|c| get_value!("h2_initial_window_size", c))
      .and_then(|v| v.as_i128())
      .map(|v| v as u32),
    max_frame_size: global_configuration
      .as_deref()
      .and_then(|c| get_value!("h2_max_frame_size", c))
      .and_then(|v| v.as_i128())
      .map(|v| v as u32),
    max_concurrent_streams: global_configuration
      .as_deref()
      .and_then(|c| get_value!("h2_max_concurrent_streams", c))
      .and_then(|v| v.as_i128())
      .map(|v| v as u32),
    max_header_list_size: global_configuration
      .as_deref()
      .and_then(|c| get_value!("h2_max_header_list_size", c))
      .and_then(|v| v.as_i128())
      .map(|v| v as u32),
    enable_connect_protocol: global_configuration
      .as_deref()
      .and_then(|c| get_value!("h2_enable_connect_protocol", c))
      .and_then(|v| v.as_bool())
      .unwrap_or(false),
  }
}

#[inline]
fn get_http3_port(http3_enabled: bool, server_address: SocketAddr) -> Option<u16> {
  if http3_enabled {
    Some(server_address.port())
  } else {
    None
  }
}

#[inline]
fn empty_acme_http_01_resolvers() -> AcmeHttp01Resolvers {
  Arc::new(tokio::sync::RwLock::new(Vec::new()))
}

#[inline]
async fn log_handler_error(configurations: &ServerConfigurations, message: impl Into<String>) {
  let message = message.into();
  let global_configuration = configurations.find_global_configuration();
  let log_channels = global_configuration
    .as_deref()
    .map_or(&[][..], |c| c.observability.log_channels.as_slice());
  for logging_tx in log_channels {
    logging_tx
      .send(LogMessage::new(message.clone(), true))
      .await
      .unwrap_or_default();
  }
}

#[inline]
async fn log_connection_accept_error(configurations: &ServerConfigurations, err: impl Display) {
  log_handler_error(configurations, format!("Cannot accept a connection: {err}")).await;
}

#[inline]
async fn log_http_connection_error(configurations: &ServerConfigurations, protocol: &str, err: impl Display) {
  log_handler_error(configurations, format!("Error serving {protocol} connection: {err}")).await;
}

#[cfg(feature = "runtime-monoio")]
#[inline]
async fn convert_tcp_stream_for_runtime(
  tcp_stream: TcpStream,
  configurations: &Arc<ServerConfigurations>,
) -> Option<HttpTcpStream> {
  match tcp_stream.into_poll_io() {
    Ok(stream) => Some(SendAsyncIo::new(stream)),
    Err(err) => {
      log_connection_accept_error(configurations, err).await;
      None
    }
  }
}

#[cfg(feature = "runtime-vibeio")]
#[inline]
async fn convert_tcp_stream_for_runtime(
  tcp_stream: TcpStream,
  configurations: &Arc<ServerConfigurations>,
) -> Option<HttpTcpStream> {
  match tcp_stream.into_poll() {
    Ok(stream) => Some(stream),
    Err(err) => {
      log_connection_accept_error(configurations, err).await;
      None
    }
  }
}

#[cfg(feature = "runtime-tokio")]
#[inline]
async fn convert_tcp_stream_for_runtime(
  tcp_stream: TcpStream,
  _configurations: &Arc<ServerConfigurations>,
) -> Option<HttpTcpStream> {
  Some(tcp_stream)
}

#[inline]
async fn maybe_read_proxy_protocol_header(
  tcp_stream: HttpTcpStream,
  enable_proxy_protocol: bool,
  configurations: &Arc<ServerConfigurations>,
) -> Option<(HttpTcpStream, Option<SocketAddr>, Option<SocketAddr>)> {
  if !enable_proxy_protocol {
    return Some((tcp_stream, None, None));
  }

  match read_proxy_header(tcp_stream).await {
    Ok((stream, client_address, server_address)) => Some((stream, client_address, server_address)),
    Err(err) => {
      log_handler_error(configurations, format!("Error reading PROXY protocol header: {err}")).await;
      None
    }
  }
}

#[inline]
async fn maybe_accept_tls_stream(
  tcp_stream: HttpTcpStream,
  tls_config: Option<Arc<ServerConfig>>,
  acme_tls_alpn_01_config: Option<Arc<ServerConfig>>,
  configurations: &Arc<ServerConfigurations>,
) -> Option<MaybeTlsStream> {
  let Some(tls_config) = tls_config else {
    return Some(MaybeTlsStream::Plain(tcp_stream));
  };

  let start_handshake = match LazyConfigAcceptor::new(Acceptor::default(), tcp_stream).await {
    Ok(start_handshake) => start_handshake,
    Err(err) => {
      log_handler_error(configurations, format!("Error during TLS handshake: {err}")).await;
      return None;
    }
  };

  if let Some(acme_config) = acme_tls_alpn_01_config {
    if start_handshake
      .client_hello()
      .alpn()
      .into_iter()
      .flatten()
      .eq([ACME_TLS_ALPN_NAME])
    {
      if let Err(err) = start_handshake.into_stream(acme_config).await {
        log_handler_error(configurations, format!("Error during TLS handshake: {err}")).await;
      }
      return None;
    }
  }

  match start_handshake.into_stream(tls_config).await {
    Ok(tls_stream) => Some(MaybeTlsStream::Tls(tls_stream)),
    Err(err) => {
      log_handler_error(configurations, format!("Error during TLS handshake: {err}")).await;
      None
    }
  }
}

#[cfg(not(feature = "runtime-vibeio"))]
#[inline]
fn sanitize_http3_response_headers(response_headers: &mut hyper::HeaderMap) {
  if let Ok(http_date) = httpdate::fmt_http_date(SystemTime::now()).try_into() {
    response_headers.entry(hyper::header::DATE).or_insert(http_date);
  }
  for header in &HTTP3_INVALID_HEADERS {
    response_headers.remove(header);
  }
  if let Some(connection_header) = response_headers
    .remove(hyper::header::CONNECTION)
    .as_ref()
    .and_then(|v| v.to_str().ok())
  {
    for name in connection_header.split(',') {
      response_headers.remove(name.trim());
    }
  }
}

/// HTTP/1.x and HTTP/2 handler function
#[allow(clippy::too_many_arguments)]
#[inline]
async fn http_tcp_handler_fn(
  tcp_stream: TcpStream,
  client_address: SocketAddr,
  server_address: SocketAddr,
  configurations: Arc<ServerConfigurations>,
  tls_config: Option<Arc<ServerConfig>>,
  http3_enabled: bool,
  connection_reference: Arc<()>,
  acme_tls_alpn_01_config: Option<Arc<ServerConfig>>,
  acme_http_01_resolvers: AcmeHttp01Resolvers,
  enable_proxy_protocol: bool,
  shutdown_rx: CancellationToken,
  graceful_shutdown_token: Arc<CancellationToken>,
) {
  let _connection_reference = Arc::downgrade(&connection_reference);
  let Some(tcp_stream) = convert_tcp_stream_for_runtime(tcp_stream, &configurations).await else {
    return;
  };
  let Some((tcp_stream, proxy_protocol_client_address, proxy_protocol_server_address)) =
    maybe_read_proxy_protocol_header(tcp_stream, enable_proxy_protocol, &configurations).await
  else {
    return;
  };
  let Some(maybe_tls_stream) =
    maybe_accept_tls_stream(tcp_stream, tls_config, acme_tls_alpn_01_config, &configurations).await
  else {
    return;
  };

  if let MaybeTlsStream::Tls(tls_stream) = maybe_tls_stream {
    let alpn_protocol = tls_stream.get_ref().1.alpn_protocol();
    let is_http2 = alpn_protocol == Some("h2".as_bytes());

    #[cfg(feature = "runtime-tokio")]
    let io = TokioIo::new(tls_stream);

    // Ferron with Vibeio would use `vibeio-http` for HTTP
    #[cfg(feature = "runtime-vibeio")]
    if is_http2 {
      use vibeio_http::{Http2Options, HttpProtocol};

      let mut h2_options = Http2Options::default();
      let http2_builder = h2_options.h2_builder();
      let http2_settings = get_http2_settings(&configurations);
      if let Some(initial_window_size) = http2_settings.initial_window_size {
        http2_builder.initial_window_size(initial_window_size);
      }
      if let Some(max_frame_size) = http2_settings.max_frame_size {
        http2_builder.max_frame_size(max_frame_size);
      }
      if let Some(max_concurrent_streams) = http2_settings.max_concurrent_streams {
        http2_builder.max_concurrent_streams(max_concurrent_streams);
      }
      if let Some(max_header_list_size) = http2_settings.max_header_list_size {
        http2_builder.max_header_list_size(max_header_list_size);
      }
      if http2_settings.enable_connect_protocol {
        http2_builder.enable_connect_protocol();
      }

      let configurations_clone = configurations.clone();
      let graceful_shutdown_token2 = CancellationToken::new();
      let connection_reference = _connection_reference.clone();
      let http_future = vibeio_http::Http2::new(tls_stream, h2_options)
        .graceful_shutdown_token(graceful_shutdown_token2.clone())
        .handle(move |request: Request<vibeio_http::Incoming>| {
          let (request_parts, request_body) = request.into_parts();
          let request = Request::from_parts(
            request_parts,
            request_body.map_err(|e| std::io::Error::other(e.to_string())).boxed(),
          );
          let fut = request_handler(
            request,
            client_address,
            server_address,
            true,
            configurations_clone.clone(),
            get_http3_port(http3_enabled, server_address),
            acme_http_01_resolvers.clone(),
            proxy_protocol_client_address,
            proxy_protocol_server_address,
          );
          let connection_reference = connection_reference.clone();
          async move {
            let r = fut.await.map_err(|e| std::io::Error::other(e.to_string()));
            drop(connection_reference);
            r
          }
        });
      let mut http_future_pin = std::pin::pin!(http_future);
      let http_future_result = crate::runtime::select! {
        result = &mut http_future_pin => {
          result
        }
        _ = shutdown_rx.cancelled() => {
            graceful_shutdown_token2.cancel();
            http_future_pin.await
        }
        _ = graceful_shutdown_token.cancelled() => {
            graceful_shutdown_token2.cancel();
          http_future_pin.await
        }
      };
      if let Err(err) = http_future_result {
        log_http_connection_error(&configurations, "HTTPS", err).await;
      }
    } else {
      use vibeio_http::{Http1Options, HttpProtocol};

      let configurations_clone = configurations.clone();
      let graceful_shutdown_token2 = CancellationToken::new();
      let connection_reference = _connection_reference.clone();
      let mut http_future = Box::pin(
        vibeio_http::Http1::new(tls_stream, Http1Options::default())
          .graceful_shutdown_token(graceful_shutdown_token2.clone())
          .handle(move |request: Request<vibeio_http::Incoming>| {
            let (request_parts, request_body) = request.into_parts();
            let request = Request::from_parts(
              request_parts,
              request_body.map_err(|e| std::io::Error::other(e.to_string())).boxed(),
            );
            let fut = request_handler(
              request,
              client_address,
              server_address,
              true,
              configurations_clone.clone(),
              get_http3_port(http3_enabled, server_address),
              acme_http_01_resolvers.clone(),
              proxy_protocol_client_address,
              proxy_protocol_server_address,
            );
            let connection_reference = connection_reference.clone();
            async move {
              let r = fut.await.map_err(|e| std::io::Error::other(e.to_string()));
              drop(connection_reference);
              r
            }
          }),
      );
      let http_future_result = crate::runtime::select! {
        result = &mut http_future => {
          result
        }
        _ = shutdown_rx.cancelled() => {
            graceful_shutdown_token2.cancel();
            http_future.await
        }
        _ = graceful_shutdown_token.cancelled() => {
            graceful_shutdown_token2.cancel();
          http_future.await
        }
      };
      if let Err(err) = http_future_result {
        log_http_connection_error(&configurations, "HTTPS", err).await;
      }
    }

    #[cfg(not(feature = "runtime-vibeio"))]
    if is_http2 {
      // Hyper's HTTP/2 connection doesn't require underlying I/O to be `Send`.
      #[cfg(feature = "runtime-monoio")]
      let io = MonoioIo::new(tls_stream);

      #[cfg(feature = "runtime-monoio")]
      let mut http2_builder = {
        let mut http2_builder = hyper::server::conn::http2::Builder::new(MonoioExecutor);
        http2_builder.timer(MonoioTimer);
        http2_builder
      };
      #[cfg(feature = "runtime-tokio")]
      let mut http2_builder = {
        let mut http2_builder = hyper::server::conn::http2::Builder::new(TokioLocalExecutor);
        http2_builder.timer(TokioTimer::new());
        http2_builder
      };

      let http2_settings = get_http2_settings(&configurations);
      if let Some(initial_window_size) = http2_settings.initial_window_size {
        http2_builder.initial_stream_window_size(initial_window_size);
      }
      if let Some(max_frame_size) = http2_settings.max_frame_size {
        http2_builder.max_frame_size(max_frame_size);
      }
      if let Some(max_concurrent_streams) = http2_settings.max_concurrent_streams {
        http2_builder.max_concurrent_streams(max_concurrent_streams);
      }
      if let Some(max_header_list_size) = http2_settings.max_header_list_size {
        http2_builder.max_header_list_size(max_header_list_size);
      }
      if http2_settings.enable_connect_protocol {
        http2_builder.enable_connect_protocol();
      }

      let configurations_clone = configurations.clone();
      let mut http_future = http2_builder.serve_connection(
        io,
        service_fn(move |request: Request<Incoming>| {
          let (request_parts, request_body) = request.into_parts();
          let request = Request::from_parts(
            request_parts,
            request_body.map_err(|e| std::io::Error::other(e.to_string())).boxed(),
          );
          request_handler(
            request,
            client_address,
            server_address,
            true,
            configurations_clone.clone(),
            get_http3_port(http3_enabled, server_address),
            acme_http_01_resolvers.clone(),
            proxy_protocol_client_address,
            proxy_protocol_server_address,
          )
        }),
      );
      let http_future_result = crate::runtime::select! {
        result = &mut http_future => {
          result
        }
        _ = shutdown_rx.cancelled() => {
          std::pin::Pin::new(&mut http_future).graceful_shutdown();
          http_future.await
        }
        _ = graceful_shutdown_token.cancelled() => {
          std::pin::Pin::new(&mut http_future).graceful_shutdown();
          http_future.await
        }
      };
      if let Err(err) = http_future_result {
        let error_to_log = if err.is_user() {
          err.source().unwrap_or(&err)
        } else {
          &err
        };
        log_http_connection_error(&configurations, "HTTPS", error_to_log).await;
      }
    } else {
      #[cfg(feature = "runtime-monoio")]
      let io = MonoioIo::new(tls_stream);

      #[cfg(feature = "runtime-monoio")]
      let http1_builder = {
        let mut http1_builder = hyper::server::conn::http1::Builder::new();

        // The timer is neccessary for the header timeout to work to mitigate Slowloris.
        http1_builder.timer(MonoioTimer);

        http1_builder
      };
      #[cfg(feature = "runtime-tokio")]
      let http1_builder = {
        let mut http1_builder = hyper::server::conn::http1::Builder::new();

        // The timer is neccessary for the header timeout to work to mitigate Slowloris.
        http1_builder.timer(TokioTimer::new());

        http1_builder
      };

      let configurations_clone = configurations.clone();
      let mut http_future = http1_builder
        .serve_connection(
          io,
          service_fn(move |request: Request<Incoming>| {
            let (request_parts, request_body) = request.into_parts();
            let request = Request::from_parts(
              request_parts,
              request_body.map_err(|e| std::io::Error::other(e.to_string())).boxed(),
            );
            request_handler(
              request,
              client_address,
              server_address,
              true,
              configurations_clone.clone(),
              get_http3_port(http3_enabled, server_address),
              acme_http_01_resolvers.clone(),
              proxy_protocol_client_address,
              proxy_protocol_server_address,
            )
          }),
        )
        .with_upgrades();
      let http_future_result = crate::runtime::select! {
        result = &mut http_future => {
          result
        }
        _ = shutdown_rx.cancelled() => {
          std::pin::Pin::new(&mut http_future).graceful_shutdown();
          http_future.await
        }
        _ = graceful_shutdown_token.cancelled() => {
          std::pin::Pin::new(&mut http_future).graceful_shutdown();
          http_future.await
        }
      };
      if let Err(err) = http_future_result {
        let error_to_log = if err.is_user() {
          err.source().unwrap_or(&err)
        } else {
          &err
        };
        log_http_connection_error(&configurations, "HTTPS", error_to_log).await;
      }
    }
  } else if let MaybeTlsStream::Plain(stream) = maybe_tls_stream {
    #[cfg(feature = "runtime-vibeio")]
    {
      use vibeio_http::{Http1Options, HttpProtocol};

      let configurations_clone = configurations.clone();
      let connection_reference = _connection_reference.clone();
      let graceful_shutdown_token2 = CancellationToken::new();
      let http1 = vibeio_http::Http1::new(stream, Http1Options::default())
        .graceful_shutdown_token(graceful_shutdown_token2.clone());

      #[cfg(target_os = "linux")]
      let mut http_future = Box::pin(http1.zerocopy().handle(move |request: Request<vibeio_http::Incoming>| {
        let (request_parts, request_body) = request.into_parts();
        let request = Request::from_parts(
          request_parts,
          request_body.map_err(|e| std::io::Error::other(e.to_string())).boxed(),
        );
        let fut = request_handler(
          request,
          client_address,
          server_address,
          false,
          configurations_clone.clone(),
          get_http3_port(http3_enabled, server_address),
          acme_http_01_resolvers.clone(),
          proxy_protocol_client_address,
          proxy_protocol_server_address,
        );
        let connection_reference = connection_reference.clone();
        async move {
          let r = fut.await.map_err(|e| std::io::Error::other(e.to_string()));
          drop(connection_reference);
          r
        }
      }));
      #[cfg(not(target_os = "linux"))]
      let mut http_future = Box::pin(http1.handle(move |request: Request<vibeio_http::Incoming>| {
        let (request_parts, request_body) = request.into_parts();
        let request = Request::from_parts(
          request_parts,
          request_body.map_err(|e| std::io::Error::other(e.to_string())).boxed(),
        );
        let fut = request_handler(
          request,
          client_address,
          server_address,
          true,
          configurations_clone.clone(),
          get_http3_port(http3_enabled, server_address),
          acme_http_01_resolvers.clone(),
          proxy_protocol_client_address,
          proxy_protocol_server_address,
        );
        let connection_reference = connection_reference.clone();
        async move {
          let r = fut.await.map_err(|e| std::io::Error::other(e.to_string()));
          drop(connection_reference);
          r
        }
      }));
      let http_future_result = crate::runtime::select! {
        result = &mut http_future => {
          result
        }
        _ = shutdown_rx.cancelled() => {
            graceful_shutdown_token2.cancel();
            http_future.await
        }
        _ = graceful_shutdown_token.cancelled() => {
            graceful_shutdown_token2.cancel();
          http_future.await
        }
      };
      if let Err(err) = http_future_result {
        log_http_connection_error(&configurations, "HTTP", err).await;
      }
    }
    #[cfg(not(feature = "runtime-vibeio"))]
    {
      #[cfg(feature = "runtime-monoio")]
      let io = MonoioIo::new(stream);
      #[cfg(feature = "runtime-tokio")]
      let io = TokioIo::new(stream);

      #[cfg(feature = "runtime-monoio")]
      let http1_builder = {
        let mut http1_builder = hyper::server::conn::http1::Builder::new();

        // The timer is neccessary for the header timeout to work to mitigate Slowloris.
        http1_builder.timer(MonoioTimer);

        http1_builder
      };
      #[cfg(feature = "runtime-tokio")]
      let http1_builder = {
        let mut http1_builder = hyper::server::conn::http1::Builder::new();

        // The timer is neccessary for the header timeout to work to mitigate Slowloris.
        http1_builder.timer(TokioTimer::new());

        http1_builder
      };

      let configurations_clone = configurations.clone();
      let mut http_future = http1_builder
        .serve_connection(
          io,
          service_fn(move |request: Request<Incoming>| {
            let (request_parts, request_body) = request.into_parts();
            let request = Request::from_parts(
              request_parts,
              request_body.map_err(|e| std::io::Error::other(e.to_string())).boxed(),
            );
            request_handler(
              request,
              client_address,
              server_address,
              false,
              configurations_clone.clone(),
              get_http3_port(http3_enabled, server_address),
              acme_http_01_resolvers.clone(),
              proxy_protocol_client_address,
              proxy_protocol_server_address,
            )
          }),
        )
        .with_upgrades();
      let http_future_result = crate::runtime::select! {
        result = &mut http_future => {
          result
        }
        _ = shutdown_rx.cancelled() => {
          std::pin::Pin::new(&mut http_future).graceful_shutdown();
          http_future.await
        }
        _ = graceful_shutdown_token.cancelled() => {
          std::pin::Pin::new(&mut http_future).graceful_shutdown();
          http_future.await
        }
      };
      if let Err(err) = http_future_result {
        let error_to_log = if err.is_user() {
          err.source().unwrap_or(&err)
        } else {
          &err
        };
        log_http_connection_error(&configurations, "HTTP", error_to_log).await;
      }
    }
  }
}

/// HTTP/3 handler function
#[inline]
#[cfg(feature = "runtime-vibeio")]
#[allow(clippy::too_many_arguments)]
#[allow(clippy::type_complexity)]
async fn http_quic_handler_fn(
  connection_attempt: quinn::Incoming,
  client_address: SocketAddr,
  server_address: SocketAddr,
  configurations: Arc<ServerConfigurations>,
  quic_tls_configs: Arc<HashMap<(Option<IpAddr>, u16), Arc<quinn::ServerConfig>>>,
  connection_reference: Arc<()>,
  shutdown_rx: CancellationToken,
  graceful_shutdown_token: Arc<CancellationToken>,
) {
  use vibeio_http::{Http3Options, HttpProtocol};

  let connection = if let Some(tls_config) = quic_tls_configs
    .get(&(Some(server_address.ip().to_canonical()), server_address.port()))
    .cloned()
    .or_else(|| quic_tls_configs.get(&(None, server_address.port())).cloned())
  {
    match connection_attempt.accept_with(tls_config) {
      Ok(connecting) => match connecting.await {
        Ok(connection) => connection,
        Err(err) => {
          log_connection_accept_error(&configurations, err).await;
          return;
        }
      },
      Err(err) => {
        log_connection_accept_error(&configurations, err).await;
        return;
      }
    }
  } else {
    match connection_attempt.await {
      Ok(connection) => connection,
      Err(err) => {
        log_connection_accept_error(&configurations, err).await;
        return;
      }
    }
  };

  let _connection_reference = Arc::downgrade(&connection_reference);
  let configurations_clone = configurations.clone();
  let graceful_shutdown_token2 = CancellationToken::new();
  let mut http_future = Box::pin(
    vibeio_http::Http3::new(h3_quinn::Connection::new(connection), Http3Options::default())
      .graceful_shutdown_token(graceful_shutdown_token2.clone())
      .handle(move |request: Request<vibeio_http::Incoming>| {
        let (request_parts, request_body) = request.into_parts();
        let request = Request::from_parts(
          request_parts,
          request_body.map_err(|e| std::io::Error::other(e.to_string())).boxed(),
        );
        let fut = request_handler(
          request,
          client_address,
          server_address,
          true,
          configurations_clone.clone(),
          None,
          empty_acme_http_01_resolvers(),
          None,
          None,
        );
        let connection_reference = connection_reference.clone();
        async move {
          let r = fut.await.map_err(|e| std::io::Error::other(e.to_string()));
          drop(connection_reference);
          r
        }
      }),
  );
  let http_future_result = crate::runtime::select! {
    result = &mut http_future => {
      result
    }
    _ = shutdown_rx.cancelled() => {
        graceful_shutdown_token2.cancel();
        http_future.await
    }
    _ = graceful_shutdown_token.cancelled() => {
        graceful_shutdown_token2.cancel();
      http_future.await
    }
  };
  if let Err(err) = http_future_result {
    log_http_connection_error(&configurations, "HTTP/3", err).await;
  }
}

/// HTTP/3 handler function
#[cfg(not(feature = "runtime-vibeio"))]
#[inline]
#[allow(clippy::too_many_arguments)]
#[allow(clippy::type_complexity)]
async fn http_quic_handler_fn(
  connection_attempt: quinn::Incoming,
  client_address: SocketAddr,
  server_address: SocketAddr,
  configurations: Arc<ServerConfigurations>,
  quic_tls_configs: Arc<HashMap<(Option<IpAddr>, u16), Arc<quinn::ServerConfig>>>,
  connection_reference: Arc<()>,
  shutdown_rx: CancellationToken,
  graceful_shutdown_token: Arc<CancellationToken>,
) {
  let connection = if let Some(tls_config) = quic_tls_configs
    .get(&(Some(server_address.ip().to_canonical()), server_address.port()))
    .cloned()
    .or_else(|| quic_tls_configs.get(&(None, server_address.port())).cloned())
  {
    match connection_attempt.accept_with(tls_config) {
      Ok(connecting) => match connecting.await {
        Ok(connection) => connection,
        Err(err) => {
          log_connection_accept_error(&configurations, err).await;
          return;
        }
      },
      Err(err) => {
        log_connection_accept_error(&configurations, err).await;
        return;
      }
    }
  } else {
    match connection_attempt.await {
      Ok(connection) => connection,
      Err(err) => {
        log_connection_accept_error(&configurations, err).await;
        return;
      }
    }
  };

  let connection_reference = Arc::downgrade(&connection_reference);
  let mut h3_conn: h3::server::Connection<h3_quinn::Connection, Bytes> =
    match h3::server::Connection::new(h3_quinn::Connection::new(connection)).await {
      Ok(h3_conn) => h3_conn,
      Err(err) => {
        log_http_connection_error(&configurations, "HTTP/3", err).await;
        return;
      }
    };

  loop {
    match crate::runtime::select! {
        biased;

        _ = shutdown_rx.cancelled() => {
          h3_conn.shutdown(0).await.unwrap_or_default();
          return;
        }
        _ = graceful_shutdown_token.cancelled() => {
          h3_conn.shutdown(0).await.unwrap_or_default();
          return;
        }
        result = h3_conn.accept() => {
          result
        }
    } {
      Ok(Some(resolver)) => {
        let configurations = configurations.clone();
        let connection_reference = connection_reference.clone();
        crate::runtime::spawn(async move {
          let _connection_reference = connection_reference;
          let (request, stream) = match resolver.resolve_request().await {
            Ok(resolved) => resolved,
            Err(err) => {
              if !err.is_h3_no_error() {
                log_http_connection_error(&configurations, "HTTP/3", err).await;
              }
              return;
            }
          };

          let (mut send, receive) = stream.split();
          let request_body_stream =
            futures_util::stream::unfold((receive, false), |(mut receive, mut is_body_finished)| async move {
              loop {
                if !is_body_finished {
                  match receive.recv_data().await {
                    Ok(Some(mut data)) => {
                      return Some((Ok(Frame::data(data.copy_to_bytes(data.remaining()))), (receive, false)));
                    }
                    Ok(None) => is_body_finished = true,
                    Err(err) => return Some((Err(std::io::Error::other(err.to_string())), (receive, false))),
                  }
                } else {
                  match receive.recv_trailers().await {
                    Ok(Some(trailers)) => return Some((Ok(Frame::trailers(trailers)), (receive, true))),
                    Ok(None) => return None,
                    Err(err) => return Some((Err(std::io::Error::other(err.to_string())), (receive, true))),
                  }
                }
              }
            });
          let request_body = BodyExt::boxed(StreamBody::new(request_body_stream));
          let (request_parts, _) = request.into_parts();
          let request = Request::from_parts(request_parts, request_body);
          let mut response = match request_handler(
            request,
            client_address,
            server_address,
            true,
            configurations.clone(),
            None,
            empty_acme_http_01_resolvers(),
            None,
            None,
          )
          .await
          {
            Ok(response) => response,
            Err(err) => {
              log_http_connection_error(&configurations, "HTTP/3", err).await;
              return;
            }
          };

          sanitize_http3_response_headers(response.headers_mut());

          let (response_parts, mut response_body) = response.into_parts();
          if let Err(err) = send.send_response(Response::from_parts(response_parts, ())).await {
            if !err.is_h3_no_error() {
              log_http_connection_error(&configurations, "HTTP/3", err).await;
            }
            return;
          }

          let mut had_trailers = false;
          while let Some(chunk) = response_body.frame().await {
            match chunk {
              Ok(frame) if frame.is_data() => match frame.into_data() {
                Ok(data) => {
                  if let Err(err) = send.send_data(data).await {
                    if !err.is_h3_no_error() {
                      log_http_connection_error(&configurations, "HTTP/3", err).await;
                    }
                    return;
                  }
                }
                Err(_) => {
                  log_handler_error(
                    &configurations,
                    "Error serving HTTP/3 connection: the frame isn't really a data frame",
                  )
                  .await;
                  return;
                }
              },
              Ok(frame) if frame.is_trailers() => match frame.into_trailers() {
                Ok(trailers) => {
                  had_trailers = true;
                  if let Err(err) = send.send_trailers(trailers).await {
                    if !err.is_h3_no_error() {
                      log_http_connection_error(&configurations, "HTTP/3", err).await;
                    }
                    return;
                  }
                }
                Err(_) => {
                  log_handler_error(
                    &configurations,
                    "Error serving HTTP/3 connection: the frame isn't really a trailers frame",
                  )
                  .await;
                  return;
                }
              },
              Ok(_) => {}
              Err(err) => {
                log_http_connection_error(&configurations, "HTTP/3", err).await;
                return;
              }
            }
          }

          if !had_trailers {
            if let Err(err) = send.finish().await {
              if !err.is_h3_no_error() {
                log_http_connection_error(&configurations, "HTTP/3", err).await;
              }
            }
          }
        });
      }
      Ok(None) => break,
      Err(err) => {
        if !err.is_h3_no_error() {
          log_http_connection_error(&configurations, "HTTP/3", err).await;
        }
        return;
      }
    }
  }
}
