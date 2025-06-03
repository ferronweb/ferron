use std::collections::HashMap;
use std::error::Error;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_channel::{Receiver, Sender};
use bytes::{Buf, Bytes};
#[cfg(feature = "runtime-monoio")]
use futures_util::StreamExt;
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
use rustls_acme::{is_tls_alpn_challenge, ResolvesServerCertAcme};
#[cfg(feature = "runtime-tokio")]
use tokio::net::TcpStream;
use tokio_rustls::server::TlsStream;
use tokio_rustls::LazyConfigAcceptor;
#[cfg(feature = "runtime-monoio")]
use tokio_util::io::{CopyToBytes, SinkWriter, StreamReader};

use crate::config::ServerConfigurations;
use crate::get_value;
use crate::listener_handler_communication::ConnectionData;
use crate::logging::LogMessage;
use crate::request_handler::request_handler;
#[cfg(feature = "runtime-monoio")]
use crate::util::SendRwStream;

// Tokio local executor
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
#[allow(clippy::too_many_arguments)]
pub fn create_http_handler(
  configurations: Arc<ServerConfigurations>,
  rx: Receiver<ConnectionData>,
  enable_uring: bool,
  logging_tx: Sender<LogMessage>,
  tls_configs: HashMap<u16, Arc<ServerConfig>>,
  http3_enabled: bool,
  acme_tls_alpn_01_configs: HashMap<u16, Arc<ServerConfig>>,
  acme_http_01_resolvers: Vec<Arc<ResolvesServerCertAcme>>,
) -> Result<Sender<()>, Box<dyn Error + Send + Sync>> {
  let (shutdown_tx, shutdown_rx) = async_channel::unbounded();
  let (handler_init_tx, listen_error_rx) = async_channel::unbounded();
  std::thread::Builder::new()
    .name("Request handler".to_string())
    .spawn(move || {
      crate::runtime::new_runtime(
        async move {
          if let Some(error) = http_handler_fn(
            configurations,
            rx,
            &handler_init_tx,
            logging_tx,
            shutdown_rx,
            tls_configs,
            http3_enabled,
            acme_tls_alpn_01_configs,
            acme_http_01_resolvers,
          )
          .await
          .err()
          {
            handler_init_tx.send(Some(error)).await.unwrap_or_default();
          }
        },
        enable_uring,
      )
      .unwrap();
    })?;

  if let Some(error) = listen_error_rx.recv_blocking()? {
    Err(error)?;
  }

  Ok(shutdown_tx)
}

/// HTTP handler function
#[allow(clippy::too_many_arguments)]
async fn http_handler_fn(
  configurations: Arc<ServerConfigurations>,
  rx: Receiver<ConnectionData>,
  handler_init_tx: &Sender<Option<Box<dyn Error + Send + Sync>>>,
  logging_tx: Sender<LogMessage>,
  shutdown_rx: Receiver<()>,
  tls_configs: HashMap<u16, Arc<ServerConfig>>,
  http3_enabled: bool,
  acme_tls_alpn_01_configs: HashMap<u16, Arc<ServerConfig>>,
  acme_http_01_resolvers: Vec<Arc<ResolvesServerCertAcme>>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
  handler_init_tx.send(None).await.unwrap_or_default();

  let acme_http_01_resolvers = Arc::new(acme_http_01_resolvers);
  let connections_references = Arc::new(());

  loop {
    let conn_data = crate::runtime::select! {
        result = rx.recv() => {
            if let Ok(recv_data) = result {
                recv_data
            } else {
                break;
            }
        }
        _ = shutdown_rx.recv() => {
            break;
        }
    };
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
      acme_tls_alpn_01_configs
        .get(&conn_data.server_address.port())
        .cloned()
    };
    let acme_http_01_resolvers = acme_http_01_resolvers.clone();
    let logging_tx = logging_tx.clone();
    let connections_references_cloned = connections_references.clone();
    crate::runtime::spawn(async move {
      match conn_data.connection {
        crate::listener_handler_communication::Connection::Tcp(tcp_stream) => {
          #[cfg(feature = "runtime-monoio")]
          let tcp_stream = match TcpStream::from_std(tcp_stream) {
            Ok(stream) => stream,
            Err(err) => {
              logging_tx
                .send(LogMessage::new(
                  format!("Cannot accept a connection: {}", err),
                  true,
                ))
                .await
                .unwrap_or_default();
              return;
            }
          };
          let encrypted = tls_config.is_some();
          http_tcp_handler_fn(
            tcp_stream,
            conn_data.client_address,
            conn_data.server_address,
            logging_tx,
            configurations,
            tls_config,
            http3_enabled && encrypted,
            connections_references_cloned,
            acme_tls_alpn_01_config,
            acme_http_01_resolvers,
          )
          .await;
        }
        crate::listener_handler_communication::Connection::Quic(quic_incoming) => {
          http_quic_handler_fn(
            quic_incoming,
            conn_data.client_address,
            conn_data.server_address,
            logging_tx,
            configurations,
            connections_references_cloned,
          )
          .await;
        }
      }
    });
  }

  while Arc::weak_count(&connections_references) > 0 {
    crate::runtime::sleep(Duration::from_millis(100)).await;
  }

  Ok(())
}

/// Enum for maybe TLS stream
#[allow(clippy::large_enum_variant)]
#[cfg(feature = "runtime-monoio")]
enum MaybeTlsStream {
  /// TLS stream
  Tls(TlsStream<TcpStreamPoll>),

  /// Plain TCP stream
  Plain(TcpStreamPoll),
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
async fn http_tcp_handler_fn(
  tcp_stream: TcpStream,
  client_address: SocketAddr,
  server_address: SocketAddr,
  logging_tx: Sender<LogMessage>,
  configurations: Arc<ServerConfigurations>,
  tls_config: Option<Arc<ServerConfig>>,
  http3_enabled: bool,
  connection_reference: Arc<()>,
  acme_tls_alpn_01_config: Option<Arc<ServerConfig>>,
  acme_http_01_resolvers: Arc<Vec<Arc<ResolvesServerCertAcme>>>,
) {
  let _connection_reference = Arc::downgrade(&connection_reference);
  #[cfg(feature = "runtime-monoio")]
  let tcp_stream = match tcp_stream.into_poll_io() {
    Ok(stream) => stream,
    Err(err) => {
      logging_tx
        .send(LogMessage::new(
          format!("Cannot accept a connection: {}", err),
          true,
        ))
        .await
        .unwrap_or_default();
      return;
    }
  };
  let maybe_tls_stream = if let Some(tls_config) = tls_config {
    let start_handshake = match LazyConfigAcceptor::new(Acceptor::default(), tcp_stream).await {
      Ok(start_handshake) => start_handshake,
      Err(err) => {
        logging_tx
          .send(LogMessage::new(
            format!("Error during TLS handshake: {}", err),
            true,
          ))
          .await
          .unwrap_or_default();
        return;
      }
    };

    if let Some(acme_config) = acme_tls_alpn_01_config {
      if is_tls_alpn_challenge(&start_handshake.client_hello()) {
        match start_handshake.into_stream(acme_config).await {
          Ok(_) => (),
          Err(err) => {
            logging_tx
              .send(LogMessage::new(
                format!("Error during TLS handshake: {}", err),
                true,
              ))
              .await
              .unwrap_or_default();
            return;
          }
        };
        return;
      }
    }

    let tls_stream = match start_handshake.into_stream(tls_config).await {
      Ok(tls_stream) => tls_stream,
      Err(err) => {
        logging_tx
          .send(LogMessage::new(
            format!("Error during TLS handshake: {}", err),
            true,
          ))
          .await
          .unwrap_or_default();
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

    #[cfg(feature = "runtime-monoio")]
    let io = {
      let send_rw_stream = SendRwStream::new(tls_stream);
      let (sink, stream) = send_rw_stream.split();
      let reader = StreamReader::new(stream);
      let writer = SinkWriter::new(CopyToBytes::new(sink));
      let rw = tokio::io::join(reader, writer);
      MonoioIo::new(rw)
    };
    #[cfg(feature = "runtime-tokio")]
    let io = TokioIo::new(tls_stream);

    if is_http2 {
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

      let logging_tx_clone = logging_tx.clone();
      if let Err(err) = http2_builder
        .serve_connection(
          io,
          service_fn(move |request: Request<Incoming>| {
            let (request_parts, request_body) = request.into_parts();
            let request = Request::from_parts(
              request_parts,
              request_body
                .map_err(|e| std::io::Error::other(e.to_string()))
                .boxed(),
            );
            request_handler(
              request,
              client_address,
              server_address,
              true,
              configurations.clone(),
              logging_tx_clone.clone(),
              if http3_enabled {
                Some(server_address.port())
              } else {
                None
              },
              acme_http_01_resolvers.clone(),
            )
          }),
        )
        .await
      {
        logging_tx
          .send(LogMessage::new(
            format!("Error serving HTTPS connection: {}", err),
            true,
          ))
          .await
          .unwrap_or_default();
      }
    } else {
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

      let logging_tx_clone = logging_tx.clone();
      if let Err(err) = http1_builder
        .serve_connection(
          io,
          service_fn(move |request: Request<Incoming>| {
            let (request_parts, request_body) = request.into_parts();
            let request = Request::from_parts(
              request_parts,
              request_body
                .map_err(|e| std::io::Error::other(e.to_string()))
                .boxed(),
            );
            request_handler(
              request,
              client_address,
              server_address,
              true,
              configurations.clone(),
              logging_tx_clone.clone(),
              if http3_enabled {
                Some(server_address.port())
              } else {
                None
              },
              acme_http_01_resolvers.clone(),
            )
          }),
        )
        .with_upgrades()
        .await
      {
        logging_tx
          .send(LogMessage::new(
            format!("Error serving HTTPS connection: {}", err),
            true,
          ))
          .await
          .unwrap_or_default();
      }
    }
  } else if let MaybeTlsStream::Plain(stream) = maybe_tls_stream {
    #[cfg(feature = "runtime-monoio")]
    let io = {
      // Some pesky code...
      let send_rw_stream = SendRwStream::new(stream);
      let (sink, stream) = send_rw_stream.split();
      let reader = StreamReader::new(stream);
      let writer = SinkWriter::new(CopyToBytes::new(sink));
      let rw = tokio::io::join(reader, writer);
      MonoioIo::new(rw)
    };
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

    let logging_tx_clone = logging_tx.clone();
    if let Err(err) = http1_builder
      .serve_connection(
        io,
        service_fn(move |request: Request<Incoming>| {
          let (request_parts, request_body) = request.into_parts();
          let request = Request::from_parts(
            request_parts,
            request_body
              .map_err(|e| std::io::Error::other(e.to_string()))
              .boxed(),
          );
          request_handler(
            request,
            client_address,
            server_address,
            false,
            configurations.clone(),
            logging_tx_clone.clone(),
            if http3_enabled {
              Some(server_address.port())
            } else {
              None
            },
            acme_http_01_resolvers.clone(),
          )
        }),
      )
      .with_upgrades()
      .await
    {
      logging_tx
        .send(LogMessage::new(
          format!("Error serving HTTP connection: {}", err),
          true,
        ))
        .await
        .unwrap_or_default();
    }
  }
}

/// HTTP/3 handler function
#[allow(clippy::too_many_arguments)]
async fn http_quic_handler_fn(
  connection_attempt: quinn::Incoming,
  client_address: SocketAddr,
  server_address: SocketAddr,
  logging_tx: Sender<LogMessage>,
  configurations: Arc<ServerConfigurations>,
  connection_reference: Arc<()>,
) {
  match connection_attempt.await {
    Ok(connection) => {
      let mut h3_conn: h3::server::Connection<h3_quinn::Connection, Bytes> =
        match h3::server::Connection::new(h3_quinn::Connection::new(connection)).await {
          Ok(h3_conn) => h3_conn,
          Err(err) => {
            logging_tx
              .send(LogMessage::new(
                format!("Error serving HTTP/3 connection: {}", err),
                true,
              ))
              .await
              .unwrap_or_default();
            return;
          }
        };

      loop {
        match h3_conn.accept().await {
          Ok(Some(resolver)) => {
            let configurations = configurations.clone();
            let logging_tx = logging_tx.clone();
            let connection_reference = Arc::downgrade(&connection_reference);
            crate::runtime::spawn(async move {
              let _connection_reference = connection_reference;
              let (request, stream) = match resolver.resolve_request().await {
                Ok(resolved) => resolved,
                Err(err) => {
                  logging_tx
                    .send(LogMessage::new(
                      format!("Error serving HTTP/3 connection: {}", err),
                      true,
                    ))
                    .await
                    .unwrap_or_default();
                  return;
                }
              };
              let (mut send, receive) = stream.split();
              let request_body_stream = futures_util::stream::unfold(
                (receive, false),
                async move |(mut receive, mut is_body_finished)| loop {
                  if !is_body_finished {
                    match receive.recv_data().await {
                      Ok(Some(mut data)) => {
                        return Some((
                          Ok(Frame::data(data.copy_to_bytes(data.remaining()))),
                          (receive, false),
                        ))
                      }
                      Ok(None) => is_body_finished = true,
                      Err(err) => {
                        return Some((
                          Err(std::io::Error::other(err.to_string())),
                          (receive, false),
                        ))
                      }
                    }
                  } else {
                    match receive.recv_trailers().await {
                      Ok(Some(trailers)) => {
                        return Some((Ok(Frame::trailers(trailers)), (receive, true)))
                      }
                      Ok(None) => {
                        return None;
                      }
                      Err(err) => {
                        return Some((Err(std::io::Error::other(err.to_string())), (receive, true)))
                      }
                    }
                  }
                },
              );
              let request_body = BodyExt::boxed(StreamBody::new(request_body_stream));
              let (request_parts, _) = request.into_parts();
              let request = Request::from_parts(request_parts, request_body);
              let mut response = match request_handler(
                request,
                client_address,
                server_address,
                true,
                configurations.clone(),
                logging_tx.clone(),
                None,
                Arc::new(vec![]),
              )
              .await
              {
                Ok(response) => response,
                Err(err) => {
                  logging_tx
                    .send(LogMessage::new(
                      format!("Error serving HTTP/3 connection: {}", err),
                      true,
                    ))
                    .await
                    .unwrap_or_default();
                  return;
                }
              };
              if let Ok(http_date) = httpdate::fmt_http_date(SystemTime::now()).try_into() {
                response
                  .headers_mut()
                  .entry(hyper::header::DATE)
                  .or_insert(http_date);
              }
              let (response_parts, mut response_body) = response.into_parts();
              if let Err(err) = send
                .send_response(Response::from_parts(response_parts, ()))
                .await
              {
                logging_tx
                  .send(LogMessage::new(
                    format!("Error serving HTTP/3 connection: {}", err),
                    true,
                  ))
                  .await
                  .unwrap_or_default();
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
                            logging_tx
                              .send(LogMessage::new(
                                format!("Error serving HTTP/3 connection: {}", err),
                                true,
                              ))
                              .await
                              .unwrap_or_default();
                            return;
                          }
                        }
                        Err(_) => {
                          logging_tx
                            .send(LogMessage::new(
                              "Error serving HTTP/3 connection: the frame isn't really a data frame".to_string(),
                              true,
                            ))
                            .await
                            .unwrap_or_default();
                          return;
                        }
                      }
                    } else if frame.is_trailers() {
                      match frame.into_trailers() {
                        Ok(trailers) => {
                          had_trailers = true;
                          if let Err(err) = send.send_trailers(trailers).await {
                            logging_tx
                              .send(LogMessage::new(
                                format!("Error serving HTTP/3 connection: {}", err),
                                true,
                              ))
                              .await
                              .unwrap_or_default();
                            return;
                          }
                        }
                        Err(_) => {
                          logging_tx
                            .send(LogMessage::new(
                              "Error serving HTTP/3 connection: the frame isn't really a trailers frame".to_string(),
                              true,
                            ))
                            .await
                            .unwrap_or_default();
                          return;
                        }
                      }
                    }
                  }
                  Err(err) => {
                    logging_tx
                      .send(LogMessage::new(
                        format!("Error serving HTTP/3 connection: {}", err),
                        true,
                      ))
                      .await
                      .unwrap_or_default();
                    return;
                  }
                }
              }
              if !had_trailers {
                if let Err(err) = send.finish().await {
                  logging_tx
                    .send(LogMessage::new(
                      format!("Error serving HTTP/3 connection: {}", err),
                      true,
                    ))
                    .await
                    .unwrap_or_default();
                }
              }
            });
          }
          Ok(None) => break,
          Err(err) => {
            logging_tx
              .send(LogMessage::new(
                format!("Error serving HTTP/3 connection: {}", err),
                true,
              ))
              .await
              .unwrap_or_default();
            return;
          }
        }
      }
    }
    Err(err) => {
      logging_tx
        .send(LogMessage::new(
          format!("Cannot accept a connection: {}", err),
          true,
        ))
        .await
        .unwrap_or_default();
    }
  }
}
