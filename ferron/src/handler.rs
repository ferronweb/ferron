use std::collections::HashMap;
use std::error::Error;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use arc_swap::ArcSwap;
use async_channel::{Receiver, Sender};
use bytes::{Buf, Bytes};
use ferron_common::logging::LogMessage;
use http_body_util::{BodyExt, StreamBody};
use hyper::body::{Frame, Incoming};
use hyper::service::service_fn;
use hyper::{Request, Response};
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
use tokio::runtime::Runtime;
use tokio_rustls::server::TlsStream;
use tokio_rustls::LazyConfigAcceptor;
use tokio_util::sync::CancellationToken;

use crate::acme::ACME_TLS_ALPN_NAME;
use crate::config::ServerConfigurations;
use crate::get_value;
use crate::listener_handler_communication::ConnectionData;
use crate::request_handler::request_handler;
#[cfg(feature = "runtime-monoio")]
use crate::util::SendAsyncIo;
use crate::util::{read_proxy_header, MultiCancel};

static HTTP3_INVALID_HEADERS: [hyper::header::HeaderName; 5] = [
  hyper::header::HeaderName::from_static("keep-alive"),
  hyper::header::HeaderName::from_static("proxy-connection"),
  hyper::header::TRANSFER_ENCODING,
  hyper::header::TE,
  hyper::header::UPGRADE,
];

/// A struct holding reloadable data for handler threads
pub struct ReloadableHandlerData {
  /// ACME TLS-ALPN-01 configurations
  pub acme_tls_alpn_01_configs: Arc<HashMap<u16, Arc<ServerConfig>>>,
  /// ACME HTTP-01 resolvers
  pub acme_http_01_resolvers: Arc<tokio::sync::RwLock<Vec<crate::acme::Http01DataLock>>>,
  /// Server configurations
  pub configurations: Arc<ServerConfigurations>,
  /// TLS configurations
  pub tls_configs: Arc<HashMap<u16, Arc<ServerConfig>>>,
  /// Whether HTTP/3 is enabled
  pub http3_enabled: bool,
  /// Whether PROXY protocol is enabled
  pub enable_proxy_protocol: bool,
  /// The secondary Tokio runtime associated with the data
  pub secondary_runtime: Arc<Runtime>,
}

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
) -> Result<(CancellationToken, Sender<()>), Box<dyn Error + Send + Sync>> {
  let shutdown_tx = CancellationToken::new();
  let shutdown_rx = shutdown_tx.clone();
  let (handler_init_tx, listen_error_rx) = async_channel::unbounded();
  let (graceful_tx, graceful_rx) = async_channel::unbounded();
  std::thread::Builder::new()
    .name("Request handler".to_string())
    .spawn(move || {
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
      secondary_runtime,
    } = &**reloadable_data.load();
    let configurations = configurations.clone();
    let tls_config = if matches!(
      conn_data.connection,
      crate::listener_handler_communication::Connection::Quic(..)
    ) {
      None
    } else {
      tls_configs.get(&conn_data.server_address.port()).cloned()
    };
    let acme_tls_alpn_01_config = if matches!(
      conn_data.connection,
      crate::listener_handler_communication::Connection::Quic(..)
    ) {
      None
    } else {
      acme_tls_alpn_01_configs.get(&conn_data.server_address.port()).cloned()
    };
    let acme_http_01_resolvers = acme_http_01_resolvers.clone();
    let connections_references_cloned = connections_references.clone();
    let shutdown_rx_clone = shutdown_rx.clone();
    let http3_enabled = *http3_enabled;
    let enable_proxy_protocol = *enable_proxy_protocol;
    let secondary_runtime = secondary_runtime.clone();
    let graceful_shutdown_token = graceful_shutdown_token.load().clone();
    crate::runtime::spawn(async move {
      match conn_data.connection {
        crate::listener_handler_communication::Connection::Tcp(tcp_stream) => {
          // Toggle O_NONBLOCK for TCP stream, when using Monoio.
          // Unset it when io_uring is enabled, and set it otherwise.
          #[cfg(feature = "runtime-monoio")]
          let _ = tcp_stream.set_nonblocking(monoio::utils::is_legacy());

          #[cfg(feature = "runtime-monoio")]
          let tcp_stream = match TcpStream::from_std(tcp_stream) {
            Ok(stream) => stream,
            Err(err) => {
              for logging_tx in configurations
                .find_global_configuration()
                .as_ref()
                .map_or(&vec![], |c| &c.observability.log_channels)
              {
                logging_tx
                  .send(LogMessage::new(format!("Cannot accept a connection: {err}"), true))
                  .await
                  .unwrap_or_default();
              }
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
            secondary_runtime,
          )
          .await;
        }
        crate::listener_handler_communication::Connection::Quic(quic_incoming) => {
          http_quic_handler_fn(
            quic_incoming,
            conn_data.client_address,
            conn_data.server_address,
            configurations,
            connections_references_cloned,
            shutdown_rx_clone,
            graceful_shutdown_token,
            secondary_runtime,
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
#[allow(clippy::large_enum_variant)]
#[cfg(feature = "runtime-monoio")]
enum MaybeTlsStream {
  /// TLS stream
  Tls(TlsStream<SendAsyncIo<TcpStreamPoll>>),

  /// Plain TCP stream
  Plain(SendAsyncIo<TcpStreamPoll>),
}

#[allow(clippy::large_enum_variant)]
#[cfg(feature = "runtime-tokio")]
enum MaybeTlsStream {
  /// TLS stream
  Tls(TlsStream<TcpStream>),

  /// Plain TCP stream
  Plain(TcpStream),
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
  acme_http_01_resolvers: Arc<tokio::sync::RwLock<Vec<crate::acme::Http01DataLock>>>,
  enable_proxy_protocol: bool,
  shutdown_rx: CancellationToken,
  graceful_shutdown_token: Arc<CancellationToken>,
  secondary_runtime: Arc<Runtime>,
) {
  let _connection_reference = Arc::downgrade(&connection_reference);
  #[cfg(feature = "runtime-monoio")]
  let tcp_stream = match tcp_stream.into_poll_io() {
    Ok(stream) => SendAsyncIo::new(stream),
    Err(err) => {
      for logging_tx in configurations
        .find_global_configuration()
        .as_ref()
        .map_or(&vec![], |c| &c.observability.log_channels)
      {
        logging_tx
          .send(LogMessage::new(format!("Cannot accept a connection: {err}"), true))
          .await
          .unwrap_or_default();
      }
      return;
    }
  };

  // PROXY protocol header precedes TLS handshakes too...
  let (tcp_stream, proxy_protocol_client_address, proxy_protocol_server_address) = if enable_proxy_protocol {
    // Read and parse the PROXY protocol header
    match read_proxy_header(tcp_stream).await {
      Ok((tcp_stream, client_ip, server_ip)) => (tcp_stream, client_ip, server_ip),
      Err(err) => {
        for logging_tx in configurations
          .find_global_configuration()
          .as_ref()
          .map_or(&vec![], |c| &c.observability.log_channels)
        {
          logging_tx
            .send(LogMessage::new(
              format!("Error reading PROXY protocol header: {err}"),
              true,
            ))
            .await
            .unwrap_or_default();
        }
        return;
      }
    }
  } else {
    (tcp_stream, None, None)
  };

  let maybe_tls_stream = if let Some(tls_config) = tls_config {
    let start_handshake = match LazyConfigAcceptor::new(Acceptor::default(), tcp_stream).await {
      Ok(start_handshake) => start_handshake,
      Err(err) => {
        for logging_tx in configurations
          .find_global_configuration()
          .as_ref()
          .map_or(&vec![], |c| &c.observability.log_channels)
        {
          logging_tx
            .send(LogMessage::new(format!("Error during TLS handshake: {err}"), true))
            .await
            .unwrap_or_default();
        }
        return;
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
          for logging_tx in configurations
            .find_global_configuration()
            .as_ref()
            .map_or(&vec![], |c| &c.observability.log_channels)
          {
            logging_tx
              .send(LogMessage::new(format!("Error during TLS handshake: {err}"), true))
              .await
              .unwrap_or_default();
          }
          return;
        };
        return;
      }
    }

    let tls_stream = match start_handshake.into_stream(tls_config).await {
      Ok(tls_stream) => tls_stream,
      Err(err) => {
        for logging_tx in configurations
          .find_global_configuration()
          .as_ref()
          .map_or(&vec![], |c| &c.observability.log_channels)
        {
          logging_tx
            .send(LogMessage::new(format!("Error during TLS handshake: {err}"), true))
            .await
            .unwrap_or_default();
        }
        return;
      }
    };

    MaybeTlsStream::Tls(tls_stream)
  } else {
    MaybeTlsStream::Plain(tcp_stream)
  };

  if let MaybeTlsStream::Tls(tls_stream) = maybe_tls_stream {
    let alpn_protocol = tls_stream.get_ref().1.alpn_protocol();
    let is_http2 = alpn_protocol == Some("h2".as_bytes());

    #[cfg(feature = "runtime-tokio")]
    let io = TokioIo::new(tls_stream);

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

      let global_configuration = configurations.find_global_configuration();

      if let Some(initial_window_size) = global_configuration
        .as_deref()
        .and_then(|c| get_value!("h2_initial_window_size", c))
        .and_then(|v| v.as_i128())
      {
        http2_builder.initial_stream_window_size(initial_window_size as u32);
      }
      if let Some(max_frame_size) = global_configuration
        .as_deref()
        .and_then(|c| get_value!("h2_max_frame_size", c))
        .and_then(|v| v.as_i128())
      {
        http2_builder.max_frame_size(max_frame_size as u32);
      }
      if let Some(max_concurrent_streams) = global_configuration
        .as_deref()
        .and_then(|c| get_value!("h2_max_concurrent_streams", c))
        .and_then(|v| v.as_i128())
      {
        http2_builder.max_concurrent_streams(max_concurrent_streams as u32);
      }
      if let Some(max_header_list_size) = global_configuration
        .as_deref()
        .and_then(|c| get_value!("h2_max_header_list_size", c))
        .and_then(|v| v.as_i128())
      {
        http2_builder.max_header_list_size(max_header_list_size as u32);
      }
      if let Some(enable_connect_protocol) = global_configuration
        .as_deref()
        .and_then(|c| get_value!("h2_enable_connect_protocol", c))
        .and_then(|v| v.as_bool())
      {
        if enable_connect_protocol {
          http2_builder.enable_connect_protocol();
        }
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
          let _runtime = secondary_runtime.clone();
          request_handler(
            request,
            client_address,
            server_address,
            true,
            configurations_clone.clone(),
            if http3_enabled {
              Some(server_address.port())
            } else {
              None
            },
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
        for logging_tx in configurations
          .find_global_configuration()
          .as_ref()
          .map_or(&vec![], |c| &c.observability.log_channels)
        {
          logging_tx
            .send(LogMessage::new(
              format!("Error serving HTTPS connection: {error_to_log}"),
              true,
            ))
            .await
            .unwrap_or_default();
        }
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
            let _runtime = secondary_runtime.clone();
            request_handler(
              request,
              client_address,
              server_address,
              true,
              configurations_clone.clone(),
              if http3_enabled {
                Some(server_address.port())
              } else {
                None
              },
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
        for logging_tx in configurations
          .find_global_configuration()
          .as_ref()
          .map_or(&vec![], |c| &c.observability.log_channels)
        {
          logging_tx
            .send(LogMessage::new(
              format!("Error serving HTTPS connection: {error_to_log}"),
              true,
            ))
            .await
            .unwrap_or_default();
        }
      }
    }
  } else if let MaybeTlsStream::Plain(stream) = maybe_tls_stream {
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
          let _runtime = secondary_runtime.clone();
          request_handler(
            request,
            client_address,
            server_address,
            false,
            configurations_clone.clone(),
            if http3_enabled {
              Some(server_address.port())
            } else {
              None
            },
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
      for logging_tx in configurations
        .find_global_configuration()
        .as_ref()
        .map_or(&vec![], |c| &c.observability.log_channels)
      {
        logging_tx
          .send(LogMessage::new(
            format!("Error serving HTTP connection: {error_to_log}"),
            true,
          ))
          .await
          .unwrap_or_default();
      }
    }
  }
}

/// HTTP/3 handler function
#[inline]
#[allow(clippy::too_many_arguments)]
async fn http_quic_handler_fn(
  connection_attempt: quinn::Incoming,
  client_address: SocketAddr,
  server_address: SocketAddr,
  configurations: Arc<ServerConfigurations>,
  connection_reference: Arc<()>,
  shutdown_rx: CancellationToken,
  graceful_shutdown_token: Arc<CancellationToken>,
  secondary_runtime: Arc<Runtime>,
) {
  match connection_attempt.await {
    Ok(connection) => {
      let connection_reference = Arc::downgrade(&connection_reference);
      let mut h3_conn: h3::server::Connection<h3_quinn::Connection, Bytes> =
        match h3::server::Connection::new(h3_quinn::Connection::new(connection)).await {
          Ok(h3_conn) => h3_conn,
          Err(err) => {
            for logging_tx in configurations
              .find_global_configuration()
              .as_ref()
              .map_or(&vec![], |c| &c.observability.log_channels)
            {
              logging_tx
                .send(LogMessage::new(format!("Error serving HTTP/3 connection: {err}"), true))
                .await
                .unwrap_or_default();
            }
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
            let secondary_runtime = secondary_runtime.clone();
            crate::runtime::spawn(async move {
              let _connection_reference = connection_reference;
              let (request, stream) = match resolver.resolve_request().await {
                Ok(resolved) => resolved,
                Err(err) => {
                  if !err.is_h3_no_error() {
                    for logging_tx in configurations
                      .find_global_configuration()
                      .as_ref()
                      .map_or(&vec![], |c| &c.observability.log_channels)
                    {
                      logging_tx
                        .send(LogMessage::new(format!("Error serving HTTP/3 connection: {err}"), true))
                        .await
                        .unwrap_or_default();
                    }
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
                          return Some((Ok(Frame::data(data.copy_to_bytes(data.remaining()))), (receive, false)))
                        }
                        Ok(None) => is_body_finished = true,
                        Err(err) => return Some((Err(std::io::Error::other(err.to_string())), (receive, false))),
                      }
                    } else {
                      match receive.recv_trailers().await {
                        Ok(Some(trailers)) => return Some((Ok(Frame::trailers(trailers)), (receive, true))),
                        Ok(None) => {
                          return None;
                        }
                        Err(err) => return Some((Err(std::io::Error::other(err.to_string())), (receive, true))),
                      }
                    }
                  }
                });
              let request_body = BodyExt::boxed(StreamBody::new(request_body_stream));
              let (request_parts, _) = request.into_parts();
              let request = Request::from_parts(request_parts, request_body);
              let _secondary_runtime = secondary_runtime.clone();
              let mut response = match request_handler(
                request,
                client_address,
                server_address,
                true,
                configurations.clone(),
                None,
                Arc::new(tokio::sync::RwLock::new(vec![])),
                None,
                None,
              )
              .await
              {
                Ok(response) => response,
                Err(err) => {
                  for logging_tx in configurations
                    .find_global_configuration()
                    .as_ref()
                    .map_or(&vec![], |c| &c.observability.log_channels)
                  {
                    logging_tx
                      .send(LogMessage::new(format!("Error serving HTTP/3 connection: {err}"), true))
                      .await
                      .unwrap_or_default();
                  }
                  return;
                }
              };
              let response_headers = response.headers_mut();
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
              let (response_parts, mut response_body) = response.into_parts();
              if let Err(err) = send.send_response(Response::from_parts(response_parts, ())).await {
                if !err.is_h3_no_error() {
                  for logging_tx in configurations
                    .find_global_configuration()
                    .as_ref()
                    .map_or(&vec![], |c| &c.observability.log_channels)
                  {
                    logging_tx
                      .send(LogMessage::new(format!("Error serving HTTP/3 connection: {err}"), true))
                      .await
                      .unwrap_or_default();
                  }
                }
                return;
              }
              let mut had_trailers = false;
              while let Some(chunk) = response_body.frame().await {
                match chunk {
                  Ok(frame) => {
                    if frame.is_data() {
                      match frame.into_data() {
                        Ok(data) => {
                          if let Err(err) = send.send_data(data).await {
                            if !err.is_h3_no_error() {
                              for logging_tx in configurations
                                .find_global_configuration()
                                .as_ref()
                                .map_or(&vec![], |c| &c.observability.log_channels)
                              {
                                logging_tx
                                  .send(LogMessage::new(format!("Error serving HTTP/3 connection: {err}"), true))
                                  .await
                                  .unwrap_or_default();
                              }
                            }
                            return;
                          }
                        }
                        Err(_) => {
                          for logging_tx in configurations
                            .find_global_configuration()
                            .as_ref()
                            .map_or(&vec![], |c| &c.observability.log_channels)
                          {
                            logging_tx
                              .send(LogMessage::new(
                                "Error serving HTTP/3 connection: the frame isn't really a data frame".to_string(),
                                true,
                              ))
                              .await
                              .unwrap_or_default();
                          }
                          return;
                        }
                      }
                    } else if frame.is_trailers() {
                      match frame.into_trailers() {
                        Ok(trailers) => {
                          had_trailers = true;
                          if let Err(err) = send.send_trailers(trailers).await {
                            if !err.is_h3_no_error() {
                              for logging_tx in configurations
                                .find_global_configuration()
                                .as_ref()
                                .map_or(&vec![], |c| &c.observability.log_channels)
                              {
                                logging_tx
                                  .send(LogMessage::new(format!("Error serving HTTP/3 connection: {err}"), true))
                                  .await
                                  .unwrap_or_default();
                              }
                            }
                            return;
                          }
                        }
                        Err(_) => {
                          for logging_tx in configurations
                            .find_global_configuration()
                            .as_ref()
                            .map_or(&vec![], |c| &c.observability.log_channels)
                          {
                            logging_tx
                              .send(LogMessage::new(
                                "Error serving HTTP/3 connection: the frame isn't really a trailers frame".to_string(),
                                true,
                              ))
                              .await
                              .unwrap_or_default();
                          }
                          return;
                        }
                      }
                    }
                  }
                  Err(err) => {
                    for logging_tx in configurations
                      .find_global_configuration()
                      .as_ref()
                      .map_or(&vec![], |c| &c.observability.log_channels)
                    {
                      logging_tx
                        .send(LogMessage::new(format!("Error serving HTTP/3 connection: {err}"), true))
                        .await
                        .unwrap_or_default();
                    }

                    return;
                  }
                }
              }
              if !had_trailers {
                if let Err(err) = send.finish().await {
                  if !err.is_h3_no_error() {
                    for logging_tx in configurations
                      .find_global_configuration()
                      .as_ref()
                      .map_or(&vec![], |c| &c.observability.log_channels)
                    {
                      logging_tx
                        .send(LogMessage::new(format!("Error serving HTTP/3 connection: {err}"), true))
                        .await
                        .unwrap_or_default();
                    }
                  }
                }
              }
            });
          }
          Ok(None) => break,
          Err(err) => {
            if !err.is_h3_no_error() {
              for logging_tx in configurations
                .find_global_configuration()
                .as_ref()
                .map_or(&vec![], |c| &c.observability.log_channels)
              {
                logging_tx
                  .send(LogMessage::new(format!("Error serving HTTP/3 connection: {err}"), true))
                  .await
                  .unwrap_or_default();
              }
            }
            return;
          }
        }
      }
    }
    Err(err) => {
      for logging_tx in configurations
        .find_global_configuration()
        .as_ref()
        .map_or(&vec![], |c| &c.observability.log_channels)
      {
        logging_tx
          .send(LogMessage::new(format!("Cannot accept a connection: {err}"), true))
          .await
          .unwrap_or_default();
      }
    }
  }
}
