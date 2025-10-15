use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
#[cfg(feature = "runtime-monoio")]
use futures_util::stream::StreamExt;
use http_body_util::combinators::BoxBody;
use http_body_util::BodyExt;
use hyper::body::Body;
use hyper::header::{HeaderName, HeaderValue};
use hyper::{header, HeaderMap, Request, Response, StatusCode, Uri, Version};
#[cfg(feature = "runtime-tokio")]
use hyper_util::rt::{TokioExecutor, TokioIo};
#[cfg(feature = "runtime-monoio")]
use monoio::io::IntoPollIo;
#[cfg(feature = "runtime-monoio")]
use monoio::net::TcpStream;
#[cfg(all(feature = "runtime-monoio", unix))]
use monoio::net::UnixStream;
#[cfg(feature = "runtime-monoio")]
use monoio_compat::hyper::{MonoioExecutor, MonoioIo};
use rustls::client::WebPkiServerVerifier;
use rustls_pki_types::ServerName;
use rustls_platform_verifier::BuilderVerifierExt;
use tokio::io::{AsyncRead, AsyncWrite};
#[cfg(feature = "runtime-tokio")]
use tokio::net::TcpStream;
#[cfg(all(feature = "runtime-tokio", unix))]
use tokio::net::UnixStream;
use tokio::sync::{Mutex, RwLock};
use tokio_rustls::TlsConnector;
#[cfg(feature = "runtime-monoio")]
use tokio_util::io::{CopyToBytes, SinkWriter, StreamReader};

use ferron_common::logging::ErrorLogger;
use ferron_common::modules::{Module, ModuleHandlers, ModuleLoader, ResponseData, SocketData};
#[cfg(feature = "runtime-monoio")]
use ferron_common::util::SendRwStream;
use ferron_common::util::{NoServerVerifier, TtlCache};
use ferron_common::{
  config::ServerConfiguration,
  util::{replace_header_placeholders, ModuleCache},
};
use ferron_common::{get_entries, get_entries_for_validation, get_value};

const DEFAULT_CONCURRENT_CONNECTIONS_PER_HOST: u32 = 48;

#[allow(clippy::type_complexity)]
enum LoadBalancerAlgorithm {
  Random,
  RoundRobin(Arc<AtomicUsize>),
  LeastConnections(Arc<RwLock<HashMap<(String, Option<String>), Arc<()>>>>),
  TwoRandomChoices(Arc<RwLock<HashMap<(String, Option<String>), Arc<()>>>>),
}

enum SendRequest {
  Http1(hyper::client::conn::http1::SendRequest<BoxBody<Bytes, std::io::Error>>),
  Http2(hyper::client::conn::http2::SendRequest<BoxBody<Bytes, std::io::Error>>),
}

type Connections = Arc<Vec<Mutex<Option<SendRequest>>>>;

enum Connection {
  #[cfg(feature = "runtime-monoio")]
  Tcp(monoio::net::tcp::stream_poll::TcpStreamPoll),
  #[cfg(not(feature = "runtime-monoio"))]
  Tcp(TcpStream),
  #[cfg(all(feature = "runtime-monoio", unix))]
  Unix(monoio::net::unix::stream_poll::UnixStreamPoll),
  #[cfg(all(not(feature = "runtime-monoio"), unix))]
  Unix(UnixStream),
}

impl AsyncRead for Connection {
  fn poll_read(
    mut self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    buf: &mut tokio::io::ReadBuf,
  ) -> Poll<Result<(), std::io::Error>> {
    match &mut *self {
      Connection::Tcp(stream) => Pin::new(stream).poll_read(cx, buf),
      #[cfg(unix)]
      Connection::Unix(stream) => Pin::new(stream).poll_read(cx, buf),
    }
  }
}

impl AsyncWrite for Connection {
  fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize, std::io::Error>> {
    match &mut *self {
      Connection::Tcp(stream) => Pin::new(stream).poll_write(cx, buf),
      #[cfg(unix)]
      Connection::Unix(stream) => Pin::new(stream).poll_write(cx, buf),
    }
  }

  fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
    match &mut *self {
      Connection::Tcp(stream) => Pin::new(stream).poll_flush(cx),
      #[cfg(unix)]
      Connection::Unix(stream) => Pin::new(stream).poll_flush(cx),
    }
  }

  fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
    match &mut *self {
      Connection::Tcp(stream) => Pin::new(stream).poll_shutdown(cx),
      #[cfg(unix)]
      Connection::Unix(stream) => Pin::new(stream).poll_shutdown(cx),
    }
  }

  fn is_write_vectored(&self) -> bool {
    match self {
      Connection::Tcp(stream) => stream.is_write_vectored(),
      #[cfg(unix)]
      Connection::Unix(stream) => stream.is_write_vectored(),
    }
  }

  fn poll_write_vectored(
    mut self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    bufs: &[std::io::IoSlice<'_>],
  ) -> Poll<Result<usize, std::io::Error>> {
    match &mut *self {
      Connection::Tcp(stream) => Pin::new(stream).poll_write_vectored(cx, bufs),
      #[cfg(unix)]
      Connection::Unix(stream) => Pin::new(stream).poll_write_vectored(cx, bufs),
    }
  }
}

/// A tracked response body
struct TrackedBody<B> {
  inner: B,
  _tracker: Arc<()>,
}

impl<B> TrackedBody<B> {
  fn new(inner: B, tracker: Arc<()>) -> Self {
    Self {
      inner,
      _tracker: tracker,
    }
  }
}

impl<B> Body for TrackedBody<B>
where
  B: Body + Unpin,
{
  type Data = B::Data;
  type Error = B::Error;

  fn poll_frame(
    mut self: Pin<&mut Self>,
    cx: &mut Context<'_>,
  ) -> Poll<Option<Result<hyper::body::Frame<Self::Data>, Self::Error>>> {
    Pin::new(&mut self.inner).poll_frame(cx)
  }

  fn is_end_stream(&self) -> bool {
    self.inner.is_end_stream()
  }

  fn size_hint(&self) -> hyper::body::SizeHint {
    self.inner.size_hint()
  }
}

/// A reverse proxy module loader
#[allow(clippy::type_complexity)]
pub struct ReverseProxyModuleLoader {
  cache: ModuleCache<ReverseProxyModule>,
}

impl Default for ReverseProxyModuleLoader {
  fn default() -> Self {
    Self::new()
  }
}

impl ReverseProxyModuleLoader {
  /// Creates a new module loader
  pub fn new() -> Self {
    Self {
      cache: ModuleCache::new(vec![]),
    }
  }
}

impl ModuleLoader for ReverseProxyModuleLoader {
  fn load_module(
    &mut self,
    config: &ServerConfiguration,
    global_config: Option<&ServerConfiguration>,
    _secondary_runtime: &tokio::runtime::Runtime,
  ) -> Result<Arc<dyn Module + Send + Sync>, Box<dyn Error + Send + Sync>> {
    Ok(
      self
        .cache
        .get_or_init::<_, Box<dyn std::error::Error + Send + Sync>>(config, |_| {
          Ok(Arc::new(ReverseProxyModule {
            failed_backends: Arc::new(RwLock::new(TtlCache::new(Duration::from_millis(
              global_config
                .and_then(|c| get_value!("lb_health_check_window", c))
                .and_then(|v| v.as_i128())
                .unwrap_or(5000) as u64,
            )))),
            round_robin_index: Arc::new(AtomicUsize::new(0)),
            connection_track: Arc::new(RwLock::new(HashMap::new())),
            proxy_to: Arc::new(get_entries!("proxy", config).map_or(vec![], |e| {
              e.inner
                .iter()
                .filter_map(|e| {
                  e.values
                    .first()
                    .and_then(|v| v.as_str().map(|s| s.to_owned()))
                    .map(|v| {
                      (
                        v,
                        e.props.get("unix").and_then(|v| v.as_str()).map(|s| s.to_owned()),
                        Arc::new({
                          let mut connections_vec = Vec::new();
                          for _ in 0..DEFAULT_CONCURRENT_CONNECTIONS_PER_HOST {
                            connections_vec.push(Mutex::new(None));
                          }
                          connections_vec
                        }),
                      )
                    })
                })
                .collect()
            })),
          }))
        })?,
    )
  }

  fn get_requirements(&self) -> Vec<&'static str> {
    vec!["proxy"]
  }

  fn validate_configuration(
    &self,
    config: &ServerConfiguration,
    used_properties: &mut std::collections::HashSet<String>,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    if let Some(entries) = get_entries_for_validation!("lb_health_check", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `lb_health_check` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!("Invalid load balancer health check enabling option"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("lb_health_check_max_fails", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `lb_health_check_max_fails` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_integer() && !entry.values[0].is_null() {
          Err(anyhow::anyhow!("Invalid load balancer health check maximum failures"))?
        } else if let Some(value) = entry.values[0].as_i128() {
          if value < 0 {
            Err(anyhow::anyhow!("Invalid load balancer health check maximum failures"))?
          }
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("lb_health_check_window", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `lb_health_check_window` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_integer() && !entry.values[0].is_null() {
          Err(anyhow::anyhow!("Invalid load balancer health check window"))?
        } else if let Some(value) = entry.values[0].as_i128() {
          if value < 0 {
            Err(anyhow::anyhow!("Invalid load balancer health check window"))?
          }
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("proxy", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `proxy` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_string() && !entry.values[0].is_null() {
          Err(anyhow::anyhow!("Invalid proxy backend server"))?
        } else if !entry.props.get("unix").is_none_or(|v| v.is_string()) {
          Err(anyhow::anyhow!("Invalid proxy Unix socket path"))?
        }

        #[cfg(not(unix))]
        if entry.props.get("unix").is_some() {
          Err(anyhow::anyhow!("Unix sockets are not supported on this platform"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("proxy_intercept_errors", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `proxy_intercept_errors` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!("Invalid proxy error interception enabling option"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("proxy_no_verification", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `proxy_no_verification` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!(
            "Invalid proxy backend server certificate verification option"
          ))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("proxy_request_header", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 2 {
          Err(anyhow::anyhow!(
            "The `proxy_request_header` configuration property must have exactly two values"
          ))?
        } else if !entry.values[0].is_string() {
          Err(anyhow::anyhow!("The header name must be a string"))?
        } else if !entry.values[1].is_string() {
          Err(anyhow::anyhow!("The header value must be a string"))?
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("proxy_request_header_remove", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `proxy_request_header_remove` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_string() {
          Err(anyhow::anyhow!("The header name must be a string"))?
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("proxy_keepalive", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `proxy_keepalive` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!("Invalid reverse proxy HTTP keep-alive enabling option"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("proxy_request_header_replace", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 2 {
          Err(anyhow::anyhow!(
            "The `proxy_request_header_replace` configuration property must have exactly two values"
          ))?
        } else if !entry.values[0].is_string() {
          Err(anyhow::anyhow!("The header name must be a string"))?
        } else if !entry.values[1].is_string() {
          Err(anyhow::anyhow!("The header value must be a string"))?
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("proxy_http2", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `proxy_http2` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!("Invalid reverse proxy HTTP/2 enabling option"))?
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("lb_retry_connection", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `lb_retry_connection` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!(
            "Invalid load balancer retry connection enabling option"
          ))?
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("lb_algorithm", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `lb_algorithm` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_string() && !entry.values[0].is_null() {
          Err(anyhow::anyhow!("Invalid load balancer algorithm"))?
        }
      }
    }

    Ok(())
  }
}

/// A reverse proxy module
#[allow(clippy::type_complexity)]
struct ReverseProxyModule {
  failed_backends: Arc<RwLock<TtlCache<(String, Option<String>), u64>>>,
  round_robin_index: Arc<AtomicUsize>,
  connection_track: Arc<RwLock<HashMap<(String, Option<String>), Arc<()>>>>,
  proxy_to: Arc<Vec<(String, Option<String>, Connections)>>,
}

impl Module for ReverseProxyModule {
  fn get_module_handlers(&self) -> Box<dyn ModuleHandlers> {
    Box::new(ReverseProxyModuleHandlers {
      failed_backends: self.failed_backends.clone(),
      round_robin_index: self.round_robin_index.clone(),
      connection_track: self.connection_track.clone(),
      proxy_to: self.proxy_to.clone(),
    })
  }
}

/// Handlers for the reverse proxy module
#[allow(clippy::type_complexity)]
struct ReverseProxyModuleHandlers {
  failed_backends: Arc<RwLock<TtlCache<(String, Option<String>), u64>>>,
  round_robin_index: Arc<AtomicUsize>,
  connection_track: Arc<RwLock<HashMap<(String, Option<String>), Arc<()>>>>,
  proxy_to: Arc<Vec<(String, Option<String>, Connections)>>,
}

#[async_trait(?Send)]
impl ModuleHandlers for ReverseProxyModuleHandlers {
  /// Handles incoming HTTP requests and proxies them to the configured backend server(s)
  ///
  /// This handler:
  /// 1. Determines which backend server to proxy to (supports load balancing)
  /// 2. Transforms the request by:
  ///    - Converting the URL to match the backend format
  ///    - Setting appropriate headers (Host, X-Forwarded-*)
  /// 3. Establishes a connection to the backend (HTTP or HTTPS)
  /// 4. Forwards the request and returns the response
  ///
  /// The handler supports:
  /// - Load balancing across multiple backends
  /// - Connection pooling/reuse
  /// - Health checking (marking failed backends)
  /// - TLS/SSL for secure connections
  /// - HTTP protocol upgrades (e.g., WebSockets)
  async fn request_handler(
    &mut self,
    request: Request<BoxBody<Bytes, std::io::Error>>,
    config: &ServerConfiguration,
    socket_data: &SocketData,
    error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    let enable_health_check = get_value!("lb_health_check", config)
      .and_then(|v| v.as_bool())
      .unwrap_or(false);
    let health_check_max_fails = get_value!("lb_health_check_max_fails", config)
      .and_then(|v| v.as_i128())
      .unwrap_or(3) as u64;
    let disable_certificate_verification = get_value!("proxy_no_verification", config)
      .and_then(|v| v.as_bool())
      .unwrap_or(false);
    let proxy_intercept_errors = get_value!("proxy_intercept_errors", config)
      .and_then(|v| v.as_bool())
      .unwrap_or(false);
    let mut proxy_to_vector = self
      .proxy_to
      .iter()
      .enumerate()
      .map(|(index, e)| (&*e.0, e.1.as_deref(), index, e.2.clone()))
      .collect();
    let load_balancer_algorithm =
      get_value!("lb_algorithm", config)
        .and_then(|v| v.as_str())
        .map_or(LoadBalancerAlgorithm::Random, |v| match v {
          "two_random" => LoadBalancerAlgorithm::TwoRandomChoices(self.connection_track.clone()),
          "least_conn" => LoadBalancerAlgorithm::LeastConnections(self.connection_track.clone()),
          "round_robin" => LoadBalancerAlgorithm::RoundRobin(self.round_robin_index.clone()),
          "random" => LoadBalancerAlgorithm::Random,
          _ => LoadBalancerAlgorithm::Random,
        });
    let track_connections = matches!(
      load_balancer_algorithm,
      LoadBalancerAlgorithm::LeastConnections(_) | LoadBalancerAlgorithm::TwoRandomChoices(_)
    );
    let retry_connection = get_value!("lb_retry_connection", config)
      .and_then(|v| v.as_bool())
      .unwrap_or(true);
    let mut excluded_backend_indexes = HashSet::new();
    let (request_parts, request_body) = request.into_parts();
    let mut request_parts = Some(request_parts);

    loop {
      if let Some((proxy_to, proxy_unix, connections)) = determine_proxy_to(
        &mut proxy_to_vector,
        &self.failed_backends,
        enable_health_check,
        health_check_max_fails,
        &load_balancer_algorithm,
        &mut excluded_backend_indexes,
      )
      .await
      {
        let proxy_request_url = proxy_to.parse::<hyper::Uri>()?;
        let scheme_str = proxy_request_url.scheme_str();
        let mut encrypted = false;

        match scheme_str {
          Some("http") => {
            encrypted = false;
          }
          Some("https") => {
            encrypted = true;
          }
          _ => Err(anyhow::anyhow!("Only HTTP and HTTPS reverse proxy URLs are supported."))?,
        };

        let host = match proxy_request_url.host() {
          Some(host) => host,
          None => Err(anyhow::anyhow!("The reverse proxy URL doesn't include the host"))?,
        };

        let port = proxy_request_url.port_u16().unwrap_or(match scheme_str {
          Some("http") => 80,
          Some("https") => 443,
          _ => 80,
        });

        let addr = format!("{host}:{port}");

        let request_parts_option = if proxy_to_vector.is_empty() {
          request_parts.take()
        } else {
          request_parts.clone()
        };
        let request_parts = request_parts_option.ok_or(anyhow::anyhow!("Request parts not found"))?;
        let proxy_request_parts =
          construct_proxy_request_parts(request_parts, config, socket_data, &proxy_request_url)?;

        let tracked_connection = if track_connections {
          let connection_track_key = (proxy_to.clone(), proxy_unix.clone());
          let connection_track_read = self.connection_track.read().await;
          Some(
            if let Some(connection_count) = connection_track_read.get(&connection_track_key) {
              connection_count.clone()
            } else {
              let tracked_connection = Arc::new(());
              drop(connection_track_read);
              self
                .connection_track
                .write()
                .await
                .insert(connection_track_key, tracked_connection.clone());
              tracked_connection
            },
          )
        } else {
          None
        };

        let connections = if get_value!("proxy_keepalive", config)
          .and_then(|v| v.as_bool())
          .unwrap_or(true)
        {
          let mut connection: &Mutex<Option<SendRequest>>;
          let mut connections_indexes = Vec::new();
          for i in 0..connections.len() {
            connections_indexes.push(i);
          }
          loop {
            let connections_indexes_index = rand::random_range(..connections_indexes.len());
            connection = &connections[connections_indexes[connections_indexes_index]];

            let mut mutex = connection.try_lock();

            match mutex.as_deref_mut() {
              Ok(Some(sender)) => {
                if match sender {
                  SendRequest::Http1(sender) => !sender.is_closed() && sender.ready().await.is_ok(),
                  SendRequest::Http2(sender) => !sender.is_closed() && sender.ready().await.is_ok(),
                } {
                  let proxy_request = Request::from_parts(proxy_request_parts, request_body);
                  let result = http_proxy_kept_alive(
                    sender,
                    proxy_request,
                    error_logger,
                    proxy_intercept_errors,
                    tracked_connection,
                  )
                  .await;
                  drop(mutex);
                  return result;
                } else {
                  drop(mutex);
                }
              }
              Ok(None) => {
                drop(mutex);
              }
              Err(_) => {
                drop(mutex);
                connections_indexes.remove(connections_indexes_index);
                if connections_indexes.is_empty() {
                  break;
                }
                continue;
              }
            }
            break;
          }
          Some(connection)
        } else {
          None
        };

        let stream = if let Some(proxy_unix_str) = &proxy_unix {
          #[cfg(not(unix))]
          {
            let _ = proxy_unix_str; // Discard the variable to avoid unused variable warning
            Err(anyhow::anyhow!("Unix sockets are not supported on this platform"))?
          }

          #[cfg(unix)]
          {
            let stream = match UnixStream::connect(proxy_unix_str).await {
              Ok(stream) => stream,
              Err(err) => {
                if enable_health_check {
                  let mut failed_backends_write = self.failed_backends.write().await;
                  let proxy_key = (proxy_to.clone(), proxy_unix.clone());
                  let failed_attempts = failed_backends_write.get(&proxy_key);
                  failed_backends_write.insert(proxy_key, failed_attempts.map_or(1, |x| x + 1));
                }

                if retry_connection && !proxy_to_vector.is_empty() {
                  error_logger
                    .log(&format!("Failed to connect to backend, trying another backend: {err}"))
                    .await;
                  continue;
                }

                match err.kind() {
                  std::io::ErrorKind::ConnectionRefused
                  | std::io::ErrorKind::NotFound
                  | std::io::ErrorKind::HostUnreachable => {
                    error_logger.log(&format!("Service unavailable: {err}")).await;
                    return Ok(ResponseData {
                      request: None,
                      response: None,
                      response_status: Some(StatusCode::SERVICE_UNAVAILABLE),
                      response_headers: None,
                      new_remote_address: None,
                    });
                  }
                  std::io::ErrorKind::TimedOut => {
                    error_logger.log(&format!("Gateway timeout: {err}")).await;
                    return Ok(ResponseData {
                      request: None,
                      response: None,
                      response_status: Some(StatusCode::GATEWAY_TIMEOUT),
                      response_headers: None,
                      new_remote_address: None,
                    });
                  }
                  _ => {
                    error_logger.log(&format!("Bad gateway: {err}")).await;
                    return Ok(ResponseData {
                      request: None,
                      response: None,
                      response_status: Some(StatusCode::BAD_GATEWAY),
                      response_headers: None,
                      new_remote_address: None,
                    });
                  }
                };
              }
            };

            #[cfg(feature = "runtime-monoio")]
            let stream = match stream.into_poll_io() {
              Ok(stream) => stream,
              Err(err) => {
                if enable_health_check {
                  let mut failed_backends_write = self.failed_backends.write().await;
                  let proxy_key = (proxy_to.clone(), proxy_unix.clone());
                  let failed_attempts = failed_backends_write.get(&proxy_key);
                  failed_backends_write.insert(proxy_key, failed_attempts.map_or(1, |x| x + 1));
                }

                if retry_connection && !proxy_to_vector.is_empty() {
                  error_logger
                    .log(&format!("Failed to connect to backend, trying another backend: {err}"))
                    .await;
                  continue;
                }

                error_logger.log(&format!("Bad gateway: {err}")).await;
                return Ok(ResponseData {
                  request: None,
                  response: None,
                  response_status: Some(StatusCode::BAD_GATEWAY),
                  response_headers: None,
                  new_remote_address: None,
                });
              }
            };

            Connection::Unix(stream)
          }
        } else {
          let stream = match TcpStream::connect(&addr).await {
            Ok(stream) => stream,
            Err(err) => {
              if enable_health_check {
                let mut failed_backends_write = self.failed_backends.write().await;
                let proxy_key = (proxy_to.clone(), proxy_unix.clone());
                let failed_attempts = failed_backends_write.get(&proxy_key);
                failed_backends_write.insert(proxy_key, failed_attempts.map_or(1, |x| x + 1));
              }

              if retry_connection && !proxy_to_vector.is_empty() {
                error_logger
                  .log(&format!("Failed to connect to backend, trying another backend: {err}"))
                  .await;
                continue;
              }

              match err.kind() {
                std::io::ErrorKind::ConnectionRefused
                | std::io::ErrorKind::NotFound
                | std::io::ErrorKind::HostUnreachable => {
                  error_logger.log(&format!("Service unavailable: {err}")).await;
                  return Ok(ResponseData {
                    request: None,
                    response: None,
                    response_status: Some(StatusCode::SERVICE_UNAVAILABLE),
                    response_headers: None,
                    new_remote_address: None,
                  });
                }
                std::io::ErrorKind::TimedOut => {
                  error_logger.log(&format!("Gateway timeout: {err}")).await;
                  return Ok(ResponseData {
                    request: None,
                    response: None,
                    response_status: Some(StatusCode::GATEWAY_TIMEOUT),
                    response_headers: None,
                    new_remote_address: None,
                  });
                }
                _ => {
                  error_logger.log(&format!("Bad gateway: {err}")).await;
                  return Ok(ResponseData {
                    request: None,
                    response: None,
                    response_status: Some(StatusCode::BAD_GATEWAY),
                    response_headers: None,
                    new_remote_address: None,
                  });
                }
              };
            }
          };

          match stream.set_nodelay(true) {
            Ok(_) => (),
            Err(err) => {
              if enable_health_check {
                let mut failed_backends_write = self.failed_backends.write().await;
                let proxy_key = (proxy_to.clone(), proxy_unix.clone());
                let failed_attempts = failed_backends_write.get(&proxy_key);
                failed_backends_write.insert(proxy_key, failed_attempts.map_or(1, |x| x + 1));
              }

              if retry_connection && !proxy_to_vector.is_empty() {
                error_logger
                  .log(&format!("Failed to connect to backend, trying another backend: {err}"))
                  .await;
                continue;
              }

              error_logger.log(&format!("Bad gateway: {err}")).await;
              return Ok(ResponseData {
                request: None,
                response: None,
                response_status: Some(StatusCode::BAD_GATEWAY),
                response_headers: None,
                new_remote_address: None,
              });
            }
          };

          #[cfg(feature = "runtime-monoio")]
          let stream = match stream.into_poll_io() {
            Ok(stream) => stream,
            Err(err) => {
              if enable_health_check {
                let mut failed_backends_write = self.failed_backends.write().await;
                let proxy_key = (proxy_to.clone(), proxy_unix.clone());
                let failed_attempts = failed_backends_write.get(&proxy_key);
                failed_backends_write.insert(proxy_key, failed_attempts.map_or(1, |x| x + 1));
              }

              if retry_connection && !proxy_to_vector.is_empty() {
                error_logger
                  .log(&format!("Failed to connect to backend, trying another backend: {err}"))
                  .await;
                continue;
              }

              error_logger.log(&format!("Bad gateway: {err}")).await;
              return Ok(ResponseData {
                request: None,
                response: None,
                response_status: Some(StatusCode::BAD_GATEWAY),
                response_headers: None,
                new_remote_address: None,
              });
            }
          };

          Connection::Tcp(stream)
        };

        let sender = if !encrypted {
          #[cfg(feature = "runtime-monoio")]
          let rw = {
            let send_rw_stream = SendRwStream::new(stream);
            let (sink, stream) = send_rw_stream.split();
            let reader = StreamReader::new(stream);
            let writer = SinkWriter::new(CopyToBytes::new(sink));
            tokio::io::join(reader, writer)
          };
          #[cfg(feature = "runtime-tokio")]
          let rw = stream;

          let sender = match http_proxy_handshake(rw, false).await {
            Ok(sender) => sender,
            Err(err) => {
              if enable_health_check {
                let mut failed_backends_write = self.failed_backends.write().await;
                let proxy_key = (proxy_to.clone(), proxy_unix.clone());
                let failed_attempts = failed_backends_write.get(&proxy_key);
                failed_backends_write.insert(proxy_key, failed_attempts.map_or(1, |x| x + 1));
              }

              if retry_connection && !proxy_to_vector.is_empty() {
                error_logger
                  .log(&format!("Failed to connect to backend, trying another backend: {err}"))
                  .await;
                continue;
              }

              error_logger.log(&format!("Bad gateway: {err}")).await;
              return Ok(ResponseData {
                request: None,
                response: None,
                response_status: Some(StatusCode::BAD_GATEWAY),
                response_headers: None,
                new_remote_address: None,
              });
            }
          };

          sender
        } else {
          let enable_http2_config = get_value!("proxy_http2", config)
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
          let mut tls_client_config = (if disable_certificate_verification {
            rustls::ClientConfig::builder()
              .dangerous()
              .with_custom_certificate_verifier(Arc::new(NoServerVerifier::new()))
          } else if let Ok(client_config) = BuilderVerifierExt::with_platform_verifier(rustls::ClientConfig::builder())
          {
            client_config
          } else {
            rustls::ClientConfig::builder().with_webpki_verifier(
              WebPkiServerVerifier::builder(Arc::new(rustls::RootCertStore {
                roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
              }))
              .build()?,
            )
          })
          .with_no_client_auth();
          if enable_http2_config {
            tls_client_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec(), b"http/1.0".to_vec()];
          } else {
            tls_client_config.alpn_protocols = vec![b"http/1.1".to_vec(), b"http/1.0".to_vec()];
          }
          let connector = TlsConnector::from(Arc::new(tls_client_config));
          let domain = ServerName::try_from(host)?.to_owned();

          let tls_stream = match connector.connect(domain, stream).await {
            Ok(stream) => stream,
            Err(err) => {
              if enable_health_check {
                let mut failed_backends_write = self.failed_backends.write().await;
                let proxy_key = (proxy_to.clone(), proxy_unix.clone());
                let failed_attempts = failed_backends_write.get(&proxy_key);
                failed_backends_write.insert(proxy_key, failed_attempts.map_or(1, |x| x + 1));
              }

              if retry_connection && !proxy_to_vector.is_empty() {
                error_logger
                  .log(&format!("Failed to connect to backend, trying another backend: {err}"))
                  .await;
                continue;
              }

              error_logger.log(&format!("Bad gateway: {err}")).await;
              return Ok(ResponseData {
                request: None,
                response: None,
                response_status: Some(StatusCode::BAD_GATEWAY),
                response_headers: None,
                new_remote_address: None,
              });
            }
          };

          // Enable HTTP/2 when the ALPN protocol is "h2"
          let enable_http2 = enable_http2_config && tls_stream.get_ref().1.alpn_protocol() == Some(b"h2");

          #[cfg(feature = "runtime-monoio")]
          let rw = {
            let send_rw_stream = SendRwStream::new(tls_stream);
            let (sink, stream) = send_rw_stream.split();
            let reader = StreamReader::new(stream);
            let writer = SinkWriter::new(CopyToBytes::new(sink));
            tokio::io::join(reader, writer)
          };
          #[cfg(feature = "runtime-tokio")]
          let rw = tls_stream;

          let sender = match http_proxy_handshake(rw, enable_http2).await {
            Ok(sender) => sender,
            Err(err) => {
              if enable_health_check {
                let mut failed_backends_write = self.failed_backends.write().await;
                let proxy_key = (proxy_to.clone(), proxy_unix.clone());
                let failed_attempts = failed_backends_write.get(&proxy_key);
                failed_backends_write.insert(proxy_key, failed_attempts.map_or(1, |x| x + 1));
              }

              if retry_connection && !proxy_to_vector.is_empty() {
                error_logger
                  .log(&format!("Failed to connect to backend, trying another backend: {err}"))
                  .await;
                continue;
              }

              error_logger.log(&format!("Bad gateway: {err}")).await;
              return Ok(ResponseData {
                request: None,
                response: None,
                response_status: Some(StatusCode::BAD_GATEWAY),
                response_headers: None,
                new_remote_address: None,
              });
            }
          };

          sender
        };

        let proxy_request = Request::from_parts(proxy_request_parts, request_body);

        return http_proxy(
          sender,
          connections,
          proxy_request,
          error_logger,
          proxy_intercept_errors,
          tracked_connection,
        )
        .await;
      } else {
        let request_parts = request_parts.ok_or(anyhow::anyhow!("Request parts are missing"))?;
        return Ok(ResponseData {
          request: Some(Request::from_parts(request_parts, request_body)),
          response: None,
          response_status: None,
          response_headers: None,
          new_remote_address: None,
        });
      }
    }
  }
}

/// Selects an index for a backend server based on the load balancing algorithm.
///
/// # Parameters
/// * `load_balancer_algorithm`: The load balancing algorithm to use.
/// * `excluded_backend_indexes`: A set of backend indexes that should be excluded.
/// * `backends`: The list of backend servers to choose from
///
/// # Returns
/// * `usize` - The index of the selected backend server.
async fn select_backend_index(
  load_balancer_algorithm: &LoadBalancerAlgorithm,
  excluded_backend_indexes: &HashSet<usize>,
  backends: &[(&str, Option<&str>, usize, Connections)],
) -> usize {
  match load_balancer_algorithm {
    LoadBalancerAlgorithm::TwoRandomChoices(connection_track) => {
      let random_choice1 = rand::random_range(..backends.len());
      let mut random_choice2 = if backends.len() > 1 {
        rand::random_range(..(backends.len() - 1))
      } else {
        0
      };
      if backends.len() > 1 && random_choice2 >= random_choice1 {
        random_choice2 += 1;
      }
      let backend1 = backends[random_choice1].clone();
      let backend2 = backends[random_choice2].clone();
      let connection_track_key1 = (backend1.0.to_string(), backend1.1.as_ref().map(|s| s.to_string()));
      let connection_track_key2 = (backend2.0.to_string(), backend2.1.as_ref().map(|s| s.to_string()));
      let connection_track_read = connection_track.read().await;
      let connection_count_option1 = connection_track_read
        .get(&connection_track_key1)
        .map(|connection_count| Arc::strong_count(connection_count) - 1);
      let connection_count_option2 = connection_track_read
        .get(&connection_track_key2)
        .map(|connection_count| Arc::strong_count(connection_count) - 1);
      drop(connection_track_read);
      let connection_count1 = if let Some(count) = connection_count_option1 {
        count
      } else {
        connection_track
          .write()
          .await
          .insert(connection_track_key1, Arc::new(()));
        0
      };
      let connection_count2 = if let Some(count) = connection_count_option2 {
        count
      } else {
        connection_track
          .write()
          .await
          .insert(connection_track_key2, Arc::new(()));
        0
      };
      if connection_count2 >= connection_count1 {
        random_choice1
      } else {
        random_choice2
      }
    }
    LoadBalancerAlgorithm::LeastConnections(connection_track) => {
      let mut min_indexes = Vec::new();
      let mut min_connections = None;
      for (index, (uri, unix, _, _)) in backends.iter().enumerate() {
        let connection_track_key = (uri.to_string(), unix.as_ref().map(|s| s.to_string()));
        let connection_track_read = connection_track.read().await;
        let connection_count = if let Some(connection_count) = connection_track_read.get(&connection_track_key) {
          Arc::strong_count(connection_count) - 1
        } else {
          drop(connection_track_read);
          connection_track
            .write()
            .await
            .insert(connection_track_key, Arc::new(()));
          0
        };
        if min_connections.is_none_or(|min| connection_count < min) {
          min_indexes = vec![index];
          min_connections = Some(connection_count);
        } else {
          min_indexes.push(index);
        }
      }
      match min_indexes.len() {
        0 => 0,
        1 => min_indexes[0],
        _ => min_indexes[rand::random_range(0..min_indexes.len())],
      }
    }
    LoadBalancerAlgorithm::RoundRobin(round_robin_index) => {
      let index;
      loop {
        let index_init = round_robin_index.fetch_add(1, Ordering::Relaxed) % backends.len();
        if excluded_backend_indexes.contains(&index_init) {
          continue;
        }
        index = index_init;
        break;
      }
      index
    }
    LoadBalancerAlgorithm::Random => rand::random_range(..backends.len()),
  }
}

/// Determines which backend server to proxy the request to, based on the list of backend servers
///
/// This function:
/// 1. Selects an appropriate backend server using different strategies:
///    - Direct selection if only one backend exists
///    - Random selection from healthy backends if health checking is enabled
///    - Random selection from all backends if health checking is disabled
/// 2. Takes into account any failed backends when health checking is enabled
///
/// # Parameters
/// * `proxy_to_vector` - List of backend servers to choose from
/// * `failed_backends` - Cache tracking failed backend attempts
/// * `enable_health_check` - Whether backend health checking is enabled
/// * `health_check_max_fails` - Maximum number of failures before considering a backend unhealthy
/// * `load_balancer_algorithm` - The load balancing algorithm to use
/// * `excluded_backend_indexes` - A set of backend indexes that should be excluded
///
/// # Returns
/// * `Option<(String, Option<String>, Connections)>` - The URL, the optional Unix socket path,
///   and the connections of the selected backend server, or None if no valid backend exists
async fn determine_proxy_to(
  proxy_to_vector: &mut Vec<(&str, Option<&str>, usize, Connections)>,
  failed_backends: &RwLock<TtlCache<(String, Option<String>), u64>>,
  enable_health_check: bool,
  health_check_max_fails: u64,
  load_balancer_algorithm: &LoadBalancerAlgorithm,
  excluded_backend_indexes: &mut HashSet<usize>,
) -> Option<(String, Option<String>, Connections)> {
  let mut proxy_to = None;
  // When the array is supplied with non-string values, the reverse proxy may have undesirable behavior
  // The "proxy" directive is validated though.

  if proxy_to_vector.is_empty() {
    return None;
  } else if proxy_to_vector.len() == 1 {
    let proxy_to_borrowed = proxy_to_vector.remove(0);
    excluded_backend_indexes.insert(proxy_to_borrowed.2);
    let proxy_to_url = proxy_to_borrowed.0.to_string();
    let proxy_to_header = proxy_to_borrowed.1.map(|header| header.to_string());
    let proxy_to_connections = proxy_to_borrowed.3.clone();
    proxy_to = Some((proxy_to_url, proxy_to_header, proxy_to_connections));
  } else if enable_health_check {
    loop {
      if !proxy_to_vector.is_empty() {
        let index = select_backend_index(load_balancer_algorithm, excluded_backend_indexes, proxy_to_vector).await;
        let proxy_to_borrowed = proxy_to_vector.remove(index);
        excluded_backend_indexes.insert(proxy_to_borrowed.2);
        let proxy_to_url = proxy_to_borrowed.0.to_string();
        let proxy_to_header = proxy_to_borrowed.1.map(|header| header.to_string());
        let proxy_to_connections = proxy_to_borrowed.3.clone();
        proxy_to = Some((proxy_to_url.clone(), proxy_to_header.clone(), proxy_to_connections));
        let failed_backends_read = failed_backends.read().await;
        let failed_backend_fails = match failed_backends_read.get(&(proxy_to_url, proxy_to_header)) {
          Some(fails) => fails,
          None => break,
        };
        if failed_backend_fails <= health_check_max_fails {
          break;
        }
      } else {
        break;
      }
    }
  } else if !proxy_to_vector.is_empty() {
    // If we have backends available and health checking is disabled,
    // select one backend from all available options
    let index = select_backend_index(load_balancer_algorithm, excluded_backend_indexes, proxy_to_vector).await;
    let proxy_to_borrowed = proxy_to_vector.remove(index);
    excluded_backend_indexes.insert(proxy_to_borrowed.2);
    let proxy_to_url = proxy_to_borrowed.0.to_string();
    let proxy_to_header = proxy_to_borrowed.1.map(|header| header.to_string());
    let proxy_to_connections = proxy_to_borrowed.3.clone();
    proxy_to = Some((proxy_to_url, proxy_to_header, proxy_to_connections));
  }

  proxy_to
}

/// Establishes a new HTTP connection to a backend server
///
/// # Parameters
/// * `stream` - The network stream to the backend server (TCP or TLS)
/// * `use_http2` - Whether to use HTTP/2 for the connection
///
/// # Returns
/// * `Result<SendRequest, Box<dyn Error + Send + Sync>>` - The HTTP connection sender side or error
async fn http_proxy_handshake(
  stream: impl AsyncRead + AsyncWrite + Send + Unpin + 'static,
  use_http2: bool,
) -> Result<SendRequest, Box<dyn Error + Send + Sync>> {
  // Convert the async stream to a Monoio- or Tokio-compatible I/O type
  #[cfg(feature = "runtime-monoio")]
  let io = MonoioIo::new(stream);
  #[cfg(feature = "runtime-tokio")]
  let io = TokioIo::new(stream);

  // Establish an HTTP/1.1 or HTTP/2 connection to the backend server
  Ok(if use_http2 {
    #[cfg(feature = "runtime-monoio")]
    let executor = MonoioExecutor;
    #[cfg(feature = "runtime-tokio")]
    let executor = TokioExecutor::new();

    let (sender, conn) = hyper::client::conn::http2::handshake(executor, io).await?;

    // Spawn a task to drive the connection
    ferron_common::runtime::spawn(async move {
      conn.await.unwrap_or_default();
    });

    SendRequest::Http2(sender)
  } else {
    let (sender, conn) = hyper::client::conn::http1::handshake(io).await?;

    // Enable HTTP protocol upgrades (e.g., WebSockets) and spawn a task to drive the connection
    let conn_with_upgrades = conn.with_upgrades();
    ferron_common::runtime::spawn(async move {
      conn_with_upgrades.await.unwrap_or_default();
    });

    SendRequest::Http1(sender)
  })
}

/// Establishes a new HTTP connection to a backend server and forwards the request
///
/// This function:
/// 1. Creates a new HTTP client connection to the specified backend
/// 2. Forwards the request to the backend server
/// 3. Handles protocol upgrades (e.g., WebSockets)
/// 4. Processes the response from the backend
/// 5. Stores the connection in the connection pool for future reuse if possible
///
/// # Parameters
/// * `sender` - The sender for the HTTP request
/// * `connections` - Optional connection pool for storing and reusing HTTP connections
/// * `connect_addr` - The address (host:port) to connect to
/// * `proxy_request` - The HTTP request to forward to the backend
/// * `error_logger` - Logger for reporting errors
/// * `proxy_intercept_errors` - Whether to intercept 4xx/5xx responses and handle them directly
/// * `tracked_connection` - The optional tracked connection to the backend server
///
/// # Returns
/// * `Result<ResponseData, Box<dyn Error + Send + Sync>>` - The HTTP response or error
async fn http_proxy(
  mut sender: SendRequest,
  connections: Option<&Mutex<Option<SendRequest>>>,
  proxy_request: Request<BoxBody<Bytes, std::io::Error>>,
  error_logger: &ErrorLogger,
  proxy_intercept_errors: bool,
  tracked_connection: Option<Arc<()>>,
) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
  let (proxy_request_parts, proxy_request_body) = proxy_request.into_parts();
  let proxy_request_cloned = Request::from_parts(proxy_request_parts.clone(), ());
  let proxy_request = Request::from_parts(proxy_request_parts, proxy_request_body);

  let send_request_result = match &mut sender {
    SendRequest::Http1(sender) => sender.send_request(proxy_request).await,
    SendRequest::Http2(sender) => sender.send_request(proxy_request).await,
  };

  let proxy_response = match send_request_result {
    Ok(response) => response,
    Err(err) => {
      error_logger.log(&format!("Bad gateway: {err}")).await;
      return Ok(ResponseData {
        request: None,
        response: None,
        response_status: Some(StatusCode::BAD_GATEWAY),
        response_headers: None,
        new_remote_address: None,
      });
    }
  };

  let status_code = proxy_response.status();

  let (proxy_response_parts, proxy_response_body) = proxy_response.into_parts();
  // Handle HTTP protocol upgrades (e.g., WebSockets)
  if proxy_response_parts.status == StatusCode::SWITCHING_PROTOCOLS {
    let proxy_response_cloned = Response::from_parts(proxy_response_parts.clone(), ());
    match hyper::upgrade::on(proxy_response_cloned).await {
      Ok(upgraded_backend) => {
        // Needed to wrap in monoio::spawn call, since otherwise HTTP upgrades wouldn't work...
        let error_logger = error_logger.clone();
        ferron_common::runtime::spawn(async move {
          // Try to upgrade the client connection
          match hyper::upgrade::on(proxy_request_cloned).await {
            Ok(upgraded_proxy) => {
              // Successfully upgraded both connections
              // Now create Monoio- or Tokio-compatible I/O types
              #[cfg(feature = "runtime-monoio")]
              let mut upgraded_backend = MonoioIo::new(upgraded_backend);
              #[cfg(feature = "runtime-tokio")]
              let mut upgraded_backend = TokioIo::new(upgraded_backend);

              #[cfg(feature = "runtime-monoio")]
              let mut upgraded_proxy = MonoioIo::new(upgraded_proxy);
              #[cfg(feature = "runtime-tokio")]
              let mut upgraded_proxy = TokioIo::new(upgraded_proxy);

              // Spawn a task to copy data bidirectionally between client and backend
              ferron_common::runtime::spawn(async move {
                tokio::io::copy_bidirectional(&mut upgraded_backend, &mut upgraded_proxy)
                  .await
                  .unwrap_or_default();
              });
            }
            Err(err) => {
              // Could not upgrade the client connection
              error_logger.log(&format!("HTTP upgrade error: {err}")).await;
            }
          }
        });
      }
      Err(err) => {
        // Could not upgrade the backend connection
        error_logger.log(&format!("HTTP upgrade error: {err}")).await;
      }
    }
  }
  let proxy_response = Response::from_parts(proxy_response_parts, proxy_response_body);

  let response = if proxy_intercept_errors && status_code.as_u16() >= 400 {
    ResponseData {
      request: None,
      response: None,
      response_status: Some(status_code),
      response_headers: None,
      new_remote_address: None,
    }
  } else {
    let (response_parts, response_body) = proxy_response.into_parts();
    let mut boxed_body = response_body.map_err(|e| std::io::Error::other(e.to_string())).boxed();
    if let Some(tracked_connection) = tracked_connection {
      boxed_body = TrackedBody::new(boxed_body, tracked_connection).boxed();
    }
    ResponseData {
      request: None,
      response: Some(Response::from_parts(response_parts, boxed_body)),
      response_status: None,
      response_headers: None,
      new_remote_address: None,
    }
  };

  // Store the HTTP connection in the connection pool for future reuse if it's still open
  if let Some(connections) = connections {
    if !(match &sender {
      SendRequest::Http1(sender) => sender.is_closed(),
      SendRequest::Http2(sender) => sender.is_closed(),
    }) {
      let mut mutex = connections.lock().await;
      mutex.replace(sender);
      drop(mutex);
    }
  }

  Ok(response)
}

/// Forwards a request using an existing, kept-alive HTTP connection to a backend server
///
/// This function:
/// 1. Uses an existing HTTP connection from the connection pool
/// 2. Forwards the request to the backend server over this connection
/// 3. Handles protocol upgrades (e.g., WebSockets)
/// 4. Processes the response from the backend
///
/// This is an optimization that avoids the overhead of establishing new TCP/TLS connections
/// when an existing connection to the same backend server is available and reusable.
///
/// # Parameters
/// * `sender` - The existing HTTP client connection to the backend
/// * `proxy_request` - The HTTP request to forward to the backend
/// * `error_logger` - Logger for reporting errors
/// * `proxy_intercept_errors` - Whether to intercept 4xx/5xx responses and handle them directly
/// * `tracked_connection` - The optional tracked connection to the backend server
///
/// # Returns
/// * `Result<ResponseData, Box<dyn Error + Send + Sync>>` - The HTTP response or error
async fn http_proxy_kept_alive(
  sender: &mut SendRequest,
  proxy_request: Request<BoxBody<Bytes, std::io::Error>>,
  error_logger: &ErrorLogger,
  proxy_intercept_errors: bool,
  tracked_connection: Option<Arc<()>>,
) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
  let (proxy_request_parts, proxy_request_body) = proxy_request.into_parts();
  let proxy_request_cloned = Request::from_parts(proxy_request_parts.clone(), ());
  let proxy_request = Request::from_parts(proxy_request_parts, proxy_request_body);

  let send_request_result = match sender {
    SendRequest::Http1(sender) => sender.send_request(proxy_request).await,
    SendRequest::Http2(sender) => sender.send_request(proxy_request).await,
  };

  // Send the request over the existing connection and await the response
  let proxy_response = match send_request_result {
    Ok(response) => response,
    Err(err) => {
      // Log the error and return a 502 Bad Gateway response
      error_logger.log(&format!("Bad gateway: {err}")).await;
      return Ok(ResponseData {
        request: None,
        response: None,
        response_status: Some(StatusCode::BAD_GATEWAY),
        response_headers: None,
        new_remote_address: None,
      });
    }
  };

  let (proxy_response_parts, proxy_response_body) = proxy_response.into_parts();
  // Handle HTTP protocol upgrades (e.g., WebSockets)
  if proxy_response_parts.status == StatusCode::SWITCHING_PROTOCOLS {
    let proxy_response_cloned = Response::from_parts(proxy_response_parts.clone(), ());
    match hyper::upgrade::on(proxy_response_cloned).await {
      Ok(upgraded_backend) => {
        // Needed to wrap in monoio::spawn call, since otherwise HTTP upgrades wouldn't work...
        let error_logger = error_logger.clone();
        ferron_common::runtime::spawn(async move {
          // Try to upgrade the client connection
          match hyper::upgrade::on(proxy_request_cloned).await {
            Ok(upgraded_proxy) => {
              // Successfully upgraded both connections
              // Now create Monoio- or Tokio-compatible I/O types
              #[cfg(feature = "runtime-monoio")]
              let mut upgraded_backend = MonoioIo::new(upgraded_backend);
              #[cfg(feature = "runtime-tokio")]
              let mut upgraded_backend = TokioIo::new(upgraded_backend);

              #[cfg(feature = "runtime-monoio")]
              let mut upgraded_proxy = MonoioIo::new(upgraded_proxy);
              #[cfg(feature = "runtime-tokio")]
              let mut upgraded_proxy = TokioIo::new(upgraded_proxy);

              // Spawn a task to copy data bidirectionally between client and backend
              ferron_common::runtime::spawn(async move {
                tokio::io::copy_bidirectional(&mut upgraded_backend, &mut upgraded_proxy)
                  .await
                  .unwrap_or_default();
              });
            }
            Err(err) => {
              // Could not upgrade the client connection
              error_logger.log(&format!("HTTP upgrade error: {err}")).await;
            }
          }
        });
      }
      Err(err) => {
        // Could not upgrade the backend connection
        error_logger.log(&format!("HTTP upgrade error: {err}")).await;
      }
    }
  }
  let proxy_response = Response::from_parts(proxy_response_parts, proxy_response_body);

  // Get the status code from the proxy response
  let status_code = proxy_response.status();

  // Handle the response differently based on whether we intercept error responses
  let response = if proxy_intercept_errors && status_code.as_u16() >= 400 {
    // If intercepting errors and status code is 400+, create a direct response with just the status code
    // This allows the server to potentially apply custom error handling
    ResponseData {
      request: None,
      response: None,
      response_status: Some(status_code),
      response_headers: None,
      new_remote_address: None,
    }
  } else {
    // For successful responses or when not intercepting errors, pass the backend response directly
    let (response_parts, response_body) = proxy_response.into_parts();
    let mut boxed_body = response_body.map_err(|e| std::io::Error::other(e.to_string())).boxed();
    if let Some(tracked_connection) = tracked_connection {
      boxed_body = TrackedBody::new(boxed_body, tracked_connection).boxed();
    }
    ResponseData {
      request: None,
      response: Some(Response::from_parts(response_parts, boxed_body)),
      response_status: None,
      response_headers: None,
      new_remote_address: None,
    }
  };

  Ok(response)
}

/// Constructs a proxy request based on the original request.
fn construct_proxy_request_parts(
  mut request_parts: hyper::http::request::Parts,
  config: &ServerConfiguration,
  socket_data: &SocketData,
  proxy_request_url: &Uri,
) -> Result<hyper::http::request::Parts, Box<dyn Error + Send + Sync>> {
  // Determine headers to add/remove/replace
  let mut headers_to_add = HeaderMap::new();
  let mut headers_to_replace = HeaderMap::new();
  let mut headers_to_remove = Vec::new();
  if let Some(custom_headers) = get_entries!("proxy_request_header", config) {
    for custom_header in custom_headers.inner.iter().rev() {
      if let Some(header_name) = custom_header.values.first().and_then(|v| v.as_str()) {
        if let Some(header_value) = custom_header.values.get(1).and_then(|v| v.as_str()) {
          if !headers_to_add.contains_key(header_name) {
            if let Ok(header_name) = HeaderName::from_str(header_name) {
              if let Ok(header_value) = HeaderValue::from_str(&replace_header_placeholders(
                header_value,
                &request_parts,
                Some(socket_data),
              )) {
                headers_to_add.insert(header_name, header_value);
              }
            }
          }
        }
      }
    }
  }
  if let Some(custom_headers) = get_entries!("proxy_request_header_replace", config) {
    for custom_header in custom_headers.inner.iter().rev() {
      if let Some(header_name) = custom_header.values.first().and_then(|v| v.as_str()) {
        if let Some(header_value) = custom_header.values.get(1).and_then(|v| v.as_str()) {
          if let Ok(header_name) = HeaderName::from_str(header_name) {
            if let Ok(header_value) = HeaderValue::from_str(&replace_header_placeholders(
              header_value,
              &request_parts,
              Some(socket_data),
            )) {
              headers_to_replace.insert(header_name, header_value);
            }
          }
        }
      }
    }
  }
  if let Some(custom_headers_to_remove) = get_entries!("proxy_request_header_remove", config) {
    for custom_header in custom_headers_to_remove.inner.iter().rev() {
      if let Some(header_name) = custom_header.values.first().and_then(|v| v.as_str()) {
        if let Ok(header_name) = HeaderName::from_str(header_name) {
          headers_to_remove.push(header_name);
        }
      }
    }
  }

  let authority = proxy_request_url.authority().cloned();

  let request_path = request_parts.uri.path();

  let path = match request_path.as_bytes().first() {
    Some(b'/') => {
      let mut proxy_request_path = proxy_request_url.path();
      while proxy_request_path.as_bytes().last().copied() == Some(b'/') {
        proxy_request_path = &proxy_request_path[..(proxy_request_path.len() - 1)];
      }
      format!("{proxy_request_path}{request_path}")
    }
    _ => request_path.to_string(),
  };

  request_parts.uri = Uri::from_str(&format!(
    "{}{}",
    path,
    match request_parts.uri.query() {
      Some(query) => format!("?{query}"),
      None => "".to_string(),
    }
  ))?;

  let original_host = request_parts.headers.get(header::HOST).cloned();

  // Host header for host identification
  match authority {
    Some(authority) => {
      request_parts
        .headers
        .insert(header::HOST, authority.to_string().parse()?);
    }
    None => {
      request_parts.headers.remove(header::HOST);
    }
  }

  // Connection header to enable HTTP/1.1 keep-alive
  if let Some(connection_header) = request_parts.headers.get(&header::CONNECTION) {
    let connection_str = String::from_utf8_lossy(connection_header.as_bytes());
    if connection_str.to_lowercase().split(",").any(|c| c == "keep-alive") {
      request_parts
        .headers
        .insert(header::CONNECTION, format!("keep-alive, {connection_str}").parse()?);
    }
  } else {
    request_parts.headers.insert(header::CONNECTION, "keep-alive".parse()?);
  }

  // X-Forwarded-* headers to send the client's data to a server that's behind the reverse proxy
  request_parts.headers.insert(
    "x-forwarded-for",
    socket_data.remote_addr.ip().to_canonical().to_string().parse()?,
  );

  if socket_data.encrypted {
    request_parts.headers.insert("x-forwarded-proto", "https".parse()?);
  } else {
    request_parts.headers.insert("x-forwarded-proto", "http".parse()?);
  }

  if let Some(original_host) = original_host {
    request_parts.headers.insert("x-forwarded-host", original_host);
  }

  for (header_name_option, header_value) in headers_to_add {
    if let Some(header_name) = header_name_option {
      if !request_parts.headers.contains_key(&header_name) {
        request_parts.headers.insert(header_name, header_value);
      }
    }
  }

  for (header_name_option, header_value) in headers_to_replace {
    if let Some(header_name) = header_name_option {
      request_parts.headers.insert(header_name, header_value);
    }
  }

  for header_to_remove in headers_to_remove.into_iter().rev() {
    if request_parts.headers.contains_key(&header_to_remove) {
      while request_parts.headers.remove(&header_to_remove).is_some() {}
    }
  }

  request_parts.version = Version::HTTP_11;

  Ok(request_parts)
}
