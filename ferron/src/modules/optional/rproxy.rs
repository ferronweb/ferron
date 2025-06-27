use std::collections::HashMap;
use std::error::Error;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
#[cfg(feature = "runtime-monoio")]
use futures_util::stream::StreamExt;
use http_body_util::combinators::BoxBody;
use http_body_util::BodyExt;
use hyper::client::conn::http1::SendRequest;
use hyper::header::{HeaderName, HeaderValue};
use hyper::{header, Request, Response, StatusCode, Uri, Version};
#[cfg(feature = "runtime-tokio")]
use hyper_util::rt::TokioIo;
#[cfg(feature = "runtime-monoio")]
use monoio::io::IntoPollIo;
#[cfg(feature = "runtime-monoio")]
use monoio::net::TcpStream;
#[cfg(feature = "runtime-monoio")]
use monoio_compat::hyper::MonoioIo;
use rustls_pki_types::ServerName;
use rustls_platform_verifier::BuilderVerifierExt;
use tokio::io::{AsyncRead, AsyncWrite};
#[cfg(feature = "runtime-tokio")]
use tokio::net::TcpStream;
use tokio::sync::RwLock;
use tokio_rustls::TlsConnector;
#[cfg(feature = "runtime-monoio")]
use tokio_util::io::{CopyToBytes, SinkWriter, StreamReader};

use crate::logging::ErrorLogger;
use crate::modules::{Module, ModuleHandlers, ModuleLoader, ResponseData, SocketData};
#[cfg(feature = "runtime-monoio")]
use crate::util::SendRwStream;
use crate::util::{get_entries, get_entries_for_validation, get_value, NoServerVerifier, TtlCache};
use crate::{config::ServerConfiguration, util::ModuleCache};

const DEFAULT_CONCURRENT_CONNECTIONS_PER_HOST: u32 = 32;

/// A reverse proxy module loader
#[allow(clippy::type_complexity)]
pub struct ReverseProxyModuleLoader {
  cache: ModuleCache<ReverseProxyModule>,
  connections: Arc<Vec<RwLock<HashMap<String, SendRequest<BoxBody<Bytes, std::io::Error>>>>>>,
}

impl ReverseProxyModuleLoader {
  /// Creates a new module loader
  pub fn new() -> Self {
    let mut connections_vec = Vec::new();
    for _ in 0..DEFAULT_CONCURRENT_CONNECTIONS_PER_HOST {
      connections_vec.push(RwLock::new(HashMap::new()));
    }

    Self {
      cache: ModuleCache::new(vec![]),
      connections: Arc::new(connections_vec),
    }
  }
}

impl ModuleLoader for ReverseProxyModuleLoader {
  fn load_module(
    &mut self,
    config: &ServerConfiguration,
    global_config: Option<&ServerConfiguration>,
  ) -> Result<Arc<dyn Module + Send + Sync>, Box<dyn Error + Send + Sync>> {
    Ok(
      self
        .cache
        .get_or_init::<_, Box<dyn std::error::Error + Send + Sync>>(config, |_| {
          Ok(Arc::new(ReverseProxyModule {
            connections: self.connections.clone(),
            failed_backends: Arc::new(RwLock::new(TtlCache::new(Duration::from_millis(
              global_config
                .and_then(|c| get_value!("lb_health_check_window", c))
                .and_then(|v| v.as_i128())
                .unwrap_or(5000) as u64,
            )))),
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
          Err(anyhow::anyhow!(
            "Invalid load balancer health check enabling option"
          ))?
        }
      }
    };

    if let Some(entries) =
      get_entries_for_validation!("lb_health_check_max_fails", config, used_properties)
    {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `lb_health_check_max_fails` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_integer() && !entry.values[0].is_null() {
          Err(anyhow::anyhow!(
            "Invalid load balancer health check maximum failures"
          ))?
        } else if let Some(value) = entry.values[0].as_i128() {
          if value < 0 {
            Err(anyhow::anyhow!(
              "Invalid load balancer health check maximum failures"
            ))?
          }
        }
      }
    };

    if let Some(entries) =
      get_entries_for_validation!("lb_health_check_window", config, used_properties)
    {
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
        }
      }
    };

    if let Some(entries) =
      get_entries_for_validation!("proxy_intercept_errors", config, used_properties)
    {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `proxy_intercept_errors` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!(
            "Invalid proxy error interception enabling option"
          ))?
        }
      }
    };

    if let Some(entries) =
      get_entries_for_validation!("proxy_no_verification", config, used_properties)
    {
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

    if let Some(entries) =
      get_entries_for_validation!("proxy_request_header", config, used_properties)
    {
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

    if let Some(entries) =
      get_entries_for_validation!("proxy_request_header_remove", config, used_properties)
    {
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

    Ok(())
  }
}

/// A reverse proxy module
#[allow(clippy::type_complexity)]
struct ReverseProxyModule {
  connections: Arc<Vec<RwLock<HashMap<String, SendRequest<BoxBody<Bytes, std::io::Error>>>>>>,
  failed_backends: Arc<RwLock<TtlCache<String, u64>>>,
}

impl Module for ReverseProxyModule {
  fn get_module_handlers(&self) -> Box<dyn ModuleHandlers> {
    Box::new(ReverseProxyModuleHandlers {
      connections: self.connections.clone(),
      failed_backends: self.failed_backends.clone(),
    })
  }
}

/// Handlers for the reverse proxy module
#[allow(clippy::type_complexity)]
struct ReverseProxyModuleHandlers {
  connections: Arc<Vec<RwLock<HashMap<String, SendRequest<BoxBody<Bytes, std::io::Error>>>>>>,
  failed_backends: Arc<RwLock<TtlCache<String, u64>>>,
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
    if let Some(proxy_to) = determine_proxy_to(
      config,
      &self.failed_backends,
      enable_health_check,
      health_check_max_fails,
    )
    .await
    {
      let (mut request_parts, request_body) = request.into_parts();

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
        _ => Err(anyhow::anyhow!(
          "Only HTTP and HTTPS reverse proxy URLs are supported."
        ))?,
      };

      let host = match proxy_request_url.host() {
        Some(host) => host,
        None => Err(anyhow::anyhow!(
          "The reverse proxy URL doesn't include the host"
        ))?,
      };

      let port = proxy_request_url.port_u16().unwrap_or(match scheme_str {
        Some("http") => 80,
        Some("https") => 443,
        _ => 80,
      });

      let addr = format!("{host}:{port}");
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
        if connection_str
          .to_lowercase()
          .split(",")
          .any(|c| c == "keep-alive")
        {
          request_parts.headers.insert(
            header::CONNECTION,
            format!("keep-alive, {connection_str}").parse()?,
          );
        }
      } else {
        request_parts
          .headers
          .insert(header::CONNECTION, "keep-alive".parse()?);
      }

      // X-Forwarded-* headers to send the client's data to a server that's behind the reverse proxy
      request_parts.headers.insert(
        "x-forwarded-for",
        socket_data
          .remote_addr
          .ip()
          .to_canonical()
          .to_string()
          .parse()?,
      );

      if socket_data.encrypted {
        request_parts
          .headers
          .insert("x-forwarded-proto", "https".parse()?);
      } else {
        request_parts
          .headers
          .insert("x-forwarded-proto", "http".parse()?);
      }

      if let Some(original_host) = original_host {
        request_parts
          .headers
          .insert("x-forwarded-host", original_host);
      }

      if let Some(custom_headers) = get_entries!("proxy_request_header", config) {
        for custom_header in custom_headers.inner.iter().rev() {
          if let Some(header_name) = custom_header.values.first().and_then(|v| v.as_str()) {
            if let Some(header_value) = custom_header.values.get(1).and_then(|v| v.as_str()) {
              if !request_parts.headers.contains_key(header_name) {
                if let Ok(header_name) = HeaderName::from_str(header_name) {
                  if let Ok(header_value) = HeaderValue::from_str(header_value) {
                    request_parts.headers.insert(header_name, header_value);
                  }
                }
              }
            }
          }
        }
      }

      if let Some(custom_headers_to_remove) = get_entries!("proxy_request_header_remove", config) {
        for custom_header in custom_headers_to_remove.inner.iter().rev() {
          if let Some(header_name) = custom_header.values.first().and_then(|v| v.as_str()) {
            if !request_parts.headers.contains_key(header_name) {
              if let Ok(header_name) = HeaderName::from_str(header_name) {
                while request_parts.headers.remove(&header_name).is_some() {}
              }
            }
          }
        }
      }

      request_parts.version = Version::HTTP_11;

      let proxy_request = Request::from_parts(request_parts, request_body);

      let connections = &self.connections[rand::random_range(..self.connections.len())];

      let rwlock_read = connections.read().await;
      let sender_read_option = rwlock_read.get(&addr);

      if let Some(sender_read) = sender_read_option {
        if !sender_read.is_closed() {
          drop(rwlock_read);
          let mut rwlock_write = connections.write().await;
          let sender_option = rwlock_write.get_mut(&addr);

          if let Some(sender) = sender_option {
            if !sender.is_closed() && sender.ready().await.is_ok() {
              let result =
                http_proxy_kept_alive(sender, proxy_request, error_logger, proxy_intercept_errors)
                  .await;
              drop(rwlock_write);
              return result;
            } else {
              drop(rwlock_write);
            }
          } else {
            drop(rwlock_write);
          }
        } else {
          drop(rwlock_read);
        }
      } else {
        drop(rwlock_read);
      }

      let stream = match TcpStream::connect(&addr).await {
        Ok(stream) => stream,
        Err(err) => {
          if enable_health_check {
            let mut failed_backends_write = self.failed_backends.write().await;
            let proxy_to = proxy_to.clone();
            let failed_attempts = failed_backends_write.get(&proxy_to);
            failed_backends_write.insert(proxy_to, failed_attempts.map_or(1, |x| x + 1));
          }
          match err.kind() {
            std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::NotFound
            | std::io::ErrorKind::HostUnreachable => {
              error_logger
                .log(&format!("Service unavailable: {err}"))
                .await;
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
            let proxy_to = proxy_to.clone();
            let failed_attempts = failed_backends_write.get(&proxy_to);
            failed_backends_write.insert(proxy_to, failed_attempts.map_or(1, |x| x + 1));
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

      let failed_backends_option_borrowed = if enable_health_check {
        Some(&*self.failed_backends)
      } else {
        None
      };

      #[cfg(feature = "runtime-monoio")]
      let stream = match stream.into_poll_io() {
        Ok(stream) => stream,
        Err(err) => {
          if enable_health_check {
            let mut failed_backends_write = self.failed_backends.write().await;
            let proxy_to = proxy_to.clone();
            let failed_attempts = failed_backends_write.get(&proxy_to);
            failed_backends_write.insert(proxy_to, failed_attempts.map_or(1, |x| x + 1));
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

      if !encrypted {
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

        http_proxy(
          connections,
          addr,
          rw,
          proxy_request,
          error_logger,
          proxy_to,
          failed_backends_option_borrowed,
          proxy_intercept_errors,
        )
        .await
      } else {
        let tls_client_config = (if disable_certificate_verification {
          rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoServerVerifier::new()))
        } else {
          rustls::ClientConfig::builder().with_platform_verifier()?
        })
        .with_no_client_auth();
        let connector = TlsConnector::from(Arc::new(tls_client_config));
        let domain = ServerName::try_from(host)?.to_owned();

        let tls_stream = match connector.connect(domain, stream).await {
          Ok(stream) => stream,
          Err(err) => {
            if enable_health_check {
              let mut failed_backends_write = self.failed_backends.write().await;
              let proxy_to = proxy_to.clone();
              let failed_attempts = failed_backends_write.get(&proxy_to);
              failed_backends_write.insert(proxy_to, failed_attempts.map_or(1, |x| x + 1));
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
        let rw = {
          let send_rw_stream = SendRwStream::new(tls_stream);
          let (sink, stream) = send_rw_stream.split();
          let reader = StreamReader::new(stream);
          let writer = SinkWriter::new(CopyToBytes::new(sink));
          tokio::io::join(reader, writer)
        };
        #[cfg(feature = "runtime-tokio")]
        let rw = tls_stream;

        http_proxy(
          connections,
          addr,
          rw,
          proxy_request,
          error_logger,
          proxy_to,
          failed_backends_option_borrowed,
          proxy_intercept_errors,
        )
        .await
      }
    } else {
      return Ok(ResponseData {
        request: Some(request),
        response: None,
        response_status: None,
        response_headers: None,
        new_remote_address: None,
      });
    }
  }
}

/// Determines which backend server to proxy the request to, based on configuration
///
/// This function:
/// 1. Retrieves the list of configured proxy backends from the config
/// 2. Selects an appropriate backend server using different strategies:
///    - Direct selection if only one backend exists
///    - Random selection from healthy backends if health checking is enabled
///    - Random selection from all backends if health checking is disabled
/// 3. Takes into account any failed backends when health checking is enabled
///
/// # Parameters
/// * `config` - Server configuration containing proxy settings
/// * `failed_backends` - Cache tracking failed backend attempts
/// * `enable_health_check` - Whether backend health checking is enabled
/// * `health_check_max_fails` - Maximum number of failures before considering a backend unhealthy
///
/// # Returns
/// * `Option<String>` - The URL of the selected backend server, or None if no valid backend exists
async fn determine_proxy_to(
  config: &ServerConfiguration,
  failed_backends: &RwLock<TtlCache<String, u64>>,
  enable_health_check: bool,
  health_check_max_fails: u64,
) -> Option<String> {
  let mut proxy_to = None;
  // When the array is supplied with non-string values, the reverse proxy may have undesirable behavior
  // The "proxy" directive is validated though.

  if let Some(proxy_to_vector) = get_entries!("proxy", config) {
    if proxy_to_vector.inner.len() == 1 {
      proxy_to = proxy_to_vector.inner[0]
        .values
        .first()
        .and_then(|v| v.as_str().map(|v| v.to_string()));
    } else if enable_health_check {
      let mut proxy_to_vector = proxy_to_vector.inner.clone();
      loop {
        if !proxy_to_vector.is_empty() {
          let index = rand::random_range(..proxy_to_vector.len());
          if let Some(proxy_to_str) = proxy_to_vector[index]
            .values
            .first()
            .and_then(|v| v.as_str())
          {
            proxy_to = Some(proxy_to_str.to_string());
            let failed_backends_read = failed_backends.read().await;
            let failed_backend_fails = match failed_backends_read.get(&proxy_to_str.to_string()) {
              Some(fails) => fails,
              None => break,
            };
            if failed_backend_fails > health_check_max_fails {
              proxy_to_vector.remove(index);
            } else {
              break;
            }
          }
        } else {
          break;
        }
      }
    } else if !proxy_to_vector.inner.is_empty() {
      // If we have backends available and health checking is disabled,
      // randomly select one backend from all available options
      if let Some(proxy_to_str) = proxy_to_vector.inner
        [rand::random_range(..proxy_to_vector.inner.len())]
      .values
      .first()
      .and_then(|v| v.as_str())
      {
        proxy_to = Some(proxy_to_str.to_string());
      }
    }
  }

  proxy_to
}

#[allow(clippy::too_many_arguments)]
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
/// * `connections` - Connection pool for storing and reusing HTTP connections
/// * `connect_addr` - The address (host:port) to connect to
/// * `stream` - The network stream to the backend server (TCP or TLS)
/// * `proxy_request` - The HTTP request to forward to the backend
/// * `error_logger` - Logger for reporting errors
/// * `proxy_to` - The full URL of the backend server (used for health checking)
/// * `failed_backends` - Cache for tracking failed backend attempts (for health checking)
/// * `proxy_intercept_errors` - Whether to intercept 4xx/5xx responses and handle them directly
///
/// # Returns
/// * `Result<ResponseData, Box<dyn Error + Send + Sync>>` - The HTTP response or error
async fn http_proxy(
  connections: &RwLock<HashMap<String, SendRequest<BoxBody<Bytes, std::io::Error>>>>,
  connect_addr: String,
  stream: impl AsyncRead + AsyncWrite + Send + Unpin + 'static,
  proxy_request: Request<BoxBody<Bytes, std::io::Error>>,
  error_logger: &ErrorLogger,
  proxy_to: String,
  failed_backends: Option<&tokio::sync::RwLock<TtlCache<std::string::String, u64>>>,
  proxy_intercept_errors: bool,
) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
  // Convert the async stream to a Monoio- or Tokio-compatible I/O type
  #[cfg(feature = "runtime-monoio")]
  let io = MonoioIo::new(stream);
  #[cfg(feature = "runtime-tokio")]
  let io = TokioIo::new(stream);

  // Establish an HTTP/1.1 connection to the backend server
  let (mut sender, conn) = match hyper::client::conn::http1::handshake(io).await {
    Ok(data) => data,
    Err(err) => {
      // Handle connection failure by:
      // 1. Incrementing the failure count for this backend if health checking is enabled
      if let Some(failed_backends) = failed_backends {
        let mut failed_backends_write = failed_backends.write().await;
        let failed_attempts = failed_backends_write.get(&proxy_to);
        failed_backends_write.insert(proxy_to, failed_attempts.map_or(1, |x| x + 1));
      }
      // 2. Logging the error
      error_logger.log(&format!("Bad gateway: {err}")).await;
      // 3. Returning a 502 Bad Gateway response
      return Ok(ResponseData {
        request: None,
        response: None,
        response_status: Some(StatusCode::BAD_GATEWAY),
        response_headers: None,
        new_remote_address: None,
      });
    }
  };

  // Enable HTTP protocol upgrades (e.g., WebSockets) and spawn a task to drive the connection
  let conn_with_upgrades = conn.with_upgrades();
  crate::runtime::spawn(async move {
    conn_with_upgrades.await.unwrap_or_default();
  });

  let (proxy_request_parts, proxy_request_body) = proxy_request.into_parts();
  let proxy_request_cloned = Request::from_parts(proxy_request_parts.clone(), ());
  let proxy_request = Request::from_parts(proxy_request_parts, proxy_request_body);

  let proxy_response = match sender.send_request(proxy_request).await {
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
        crate::runtime::spawn(async move {
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
              crate::runtime::spawn(async move {
                tokio::io::copy_bidirectional(&mut upgraded_backend, &mut upgraded_proxy)
                  .await
                  .unwrap_or_default();
              });
            }
            Err(err) => {
              // Could not upgrade the client connection
              error_logger
                .log(&format!("HTTP upgrade error: {err}"))
                .await;
            }
          }
        });
      }
      Err(err) => {
        // Could not upgrade the backend connection
        error_logger
          .log(&format!("HTTP upgrade error: {err}"))
          .await;
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
    ResponseData {
      request: None,
      response: Some(
        proxy_response.map(|b| b.map_err(|e| std::io::Error::other(e.to_string())).boxed()),
      ),
      response_status: None,
      response_headers: None,
      new_remote_address: None,
    }
  };

  // Store the HTTP connection in the connection pool for future reuse if it's still open
  if !sender.is_closed() {
    let mut rwlock_write = connections.write().await;
    rwlock_write.insert(connect_addr, sender);
    drop(rwlock_write);
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
///
/// # Returns
/// * `Result<ResponseData, Box<dyn Error + Send + Sync>>` - The HTTP response or error
async fn http_proxy_kept_alive(
  sender: &mut SendRequest<BoxBody<Bytes, std::io::Error>>,
  proxy_request: Request<BoxBody<Bytes, std::io::Error>>,
  error_logger: &ErrorLogger,
  proxy_intercept_errors: bool,
) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
  let (proxy_request_parts, proxy_request_body) = proxy_request.into_parts();
  let proxy_request_cloned = Request::from_parts(proxy_request_parts.clone(), ());
  let proxy_request = Request::from_parts(proxy_request_parts, proxy_request_body);

  // Send the request over the existing connection and await the response
  let proxy_response = match sender.send_request(proxy_request).await {
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
        crate::runtime::spawn(async move {
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
              crate::runtime::spawn(async move {
                tokio::io::copy_bidirectional(&mut upgraded_backend, &mut upgraded_proxy)
                  .await
                  .unwrap_or_default();
              });
            }
            Err(err) => {
              // Could not upgrade the client connection
              error_logger
                .log(&format!("HTTP upgrade error: {err}"))
                .await;
            }
          }
        });
      }
      Err(err) => {
        // Could not upgrade the backend connection
        error_logger
          .log(&format!("HTTP upgrade error: {err}"))
          .await;
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
    ResponseData {
      request: None,
      response: Some(
        proxy_response.map(|b| b.map_err(|e| std::io::Error::other(e.to_string())).boxed()),
      ),
      response_status: None,
      response_headers: None,
      new_remote_address: None,
    }
  };

  Ok(response)
}
