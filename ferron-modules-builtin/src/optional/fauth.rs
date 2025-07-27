use std::collections::HashMap;
use std::error::Error;
use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
#[cfg(feature = "runtime-monoio")]
use futures_util::stream::StreamExt;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty};
use hyper::Version;
use hyper::{header, header::HeaderName, Method, Request, StatusCode, Uri};
#[cfg(feature = "runtime-tokio")]
use hyper_util::rt::{TokioExecutor, TokioIo};
#[cfg(feature = "runtime-monoio")]
use monoio::io::IntoPollIo;
#[cfg(feature = "runtime-monoio")]
use monoio::net::TcpStream;
#[cfg(feature = "runtime-monoio")]
use monoio_compat::hyper::{MonoioExecutor, MonoioIo};
use rustls_pki_types::ServerName;
use rustls_platform_verifier::BuilderVerifierExt;
use tokio::io::{AsyncRead, AsyncWrite};
#[cfg(feature = "runtime-tokio")]
use tokio::net::TcpStream;
use tokio::sync::RwLock;
use tokio_rustls::TlsConnector;
#[cfg(feature = "runtime-monoio")]
use tokio_util::io::{CopyToBytes, SinkWriter, StreamReader};

#[cfg(feature = "runtime-monoio")]
use crate::util::SendRwStream;
use ferron_common::logging::ErrorLogger;
use ferron_common::modules::{Module, ModuleHandlers, ModuleLoader, ResponseData, SocketData};
use ferron_common::util::NoServerVerifier;
use ferron_common::{config::ServerConfiguration, util::ModuleCache};
use ferron_common::{get_entries_for_validation, get_entry, get_value, get_values};

const DEFAULT_CONCURRENT_CONNECTIONS_PER_HOST: u32 = 32;

enum SendRequest {
  Http1(hyper::client::conn::http1::SendRequest<BoxBody<Bytes, std::io::Error>>),
  Http2(hyper::client::conn::http2::SendRequest<BoxBody<Bytes, std::io::Error>>),
}

/// A forwarded authentication module loader
#[allow(clippy::type_complexity)]
pub struct ForwardedAuthenticationModuleLoader {
  cache: ModuleCache<ForwardedAuthenticationModule>,
  connections: Arc<Vec<RwLock<HashMap<String, SendRequest>>>>,
}

impl Default for ForwardedAuthenticationModuleLoader {
  fn default() -> Self {
    Self::new()
  }
}

impl ForwardedAuthenticationModuleLoader {
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

impl ModuleLoader for ForwardedAuthenticationModuleLoader {
  fn load_module(
    &mut self,
    config: &ServerConfiguration,
    _global_config: Option<&ServerConfiguration>,
    _secondary_runtime: &tokio::runtime::Runtime,
  ) -> Result<Arc<dyn Module + Send + Sync>, Box<dyn Error + Send + Sync>> {
    Ok(
      self
        .cache
        .get_or_init::<_, Box<dyn std::error::Error + Send + Sync>>(config, |_| {
          Ok(Arc::new(ForwardedAuthenticationModule {
            connections: self.connections.clone(),
          }))
        })?,
    )
  }

  fn get_requirements(&self) -> Vec<&'static str> {
    vec!["auth_to"]
  }

  fn validate_configuration(
    &self,
    config: &ServerConfiguration,
    used_properties: &mut std::collections::HashSet<String>,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    if let Some(entries) = get_entries_for_validation!("auth_to", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `auth_to` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_string() && !entry.values[0].is_null() {
          Err(anyhow::anyhow!("Invalid forwarded authentication backend server"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("auth_to_no_verification", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `auth_to_no_verification` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!(
            "Invalid authentication backend server certificate verification option"
          ))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("auth_to_copy", config, used_properties) {
      for entry in &entries.inner {
        for value in &entry.values {
          if !value.is_string() {
            Err(anyhow::anyhow!(
              "Invalid request headers to copy to the authentication server request configuration"
            ))?
          }
        }
      }
    }

    Ok(())
  }
}

/// A forwarded authentication module
#[allow(clippy::type_complexity)]
struct ForwardedAuthenticationModule {
  connections: Arc<Vec<RwLock<HashMap<String, SendRequest>>>>,
}

impl Module for ForwardedAuthenticationModule {
  fn get_module_handlers(&self) -> Box<dyn ModuleHandlers> {
    Box::new(ForwardedAuthenticationModuleHandlers {
      connections: self.connections.clone(),
    })
  }
}

/// Handlers for the forwarded authentication proxy module
#[allow(clippy::type_complexity)]
struct ForwardedAuthenticationModuleHandlers {
  connections: Arc<Vec<RwLock<HashMap<String, SendRequest>>>>,
}

#[async_trait(?Send)]
impl ModuleHandlers for ForwardedAuthenticationModuleHandlers {
  async fn request_handler(
    &mut self,
    request: Request<BoxBody<Bytes, std::io::Error>>,
    config: &ServerConfiguration,
    socket_data: &SocketData,
    error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    let disable_certificate_verification = get_value!("auth_to_no_verification", config)
      .and_then(|v| v.as_bool())
      .unwrap_or(false);
    let forwarded_auth_copy_headers = get_values!("auth_to_copy", config)
      .into_iter()
      .filter_map(|v| v.as_str().map(|v| v.to_string()))
      .collect::<Vec<_>>();
    if let Some(auth_to) = get_entry!("auth_to", config)
      .and_then(|e| e.values.first())
      .and_then(|v| v.as_str())
    {
      let (request_parts, request_body) = request.into_parts();

      let auth_request_url = auth_to.parse::<hyper::Uri>()?;
      let scheme_str = auth_request_url.scheme_str();
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

      let host = match auth_request_url.host() {
        Some(host) => host,
        None => Err(anyhow::anyhow!("The reverse proxy URL doesn't include the host"))?,
      };

      let port = auth_request_url.port_u16().unwrap_or(match scheme_str {
        Some("http") => 80,
        Some("https") => 443,
        _ => 80,
      });

      let addr = format!("{host}:{port}");
      let authority = auth_request_url.authority().cloned();

      let request_path = request_parts.uri.path();

      let path_and_query = format!(
        "{}{}",
        request_path,
        match request_parts.uri.query() {
          Some(query) => format!("?{query}"),
          None => "".to_string(),
        }
      );

      let mut auth_request_parts = request_parts.clone();

      auth_request_parts.uri = Uri::from_str(&format!(
        "{}{}",
        auth_request_url.path(),
        match auth_request_url.query() {
          Some(query) => format!("?{query}"),
          None => "".to_string(),
        }
      ))?;

      let original_host = request_parts.headers.get(header::HOST).cloned();

      // Host header for host identification
      match authority {
        Some(authority) => {
          auth_request_parts
            .headers
            .insert(header::HOST, authority.to_string().parse()?);
        }
        None => {
          auth_request_parts.headers.remove(header::HOST);
        }
      }

      // Connection header to enable HTTP/1.1 keep-alive
      auth_request_parts
        .headers
        .insert(header::CONNECTION, "keep-alive".parse()?);

      // X-Forwarded-* headers to send the client's data to a forwarded authentication server
      auth_request_parts.headers.insert(
        "x-forwarded-for",
        socket_data.remote_addr.ip().to_canonical().to_string().parse()?,
      );

      if socket_data.encrypted {
        auth_request_parts.headers.insert("x-forwarded-proto", "https".parse()?);
      } else {
        auth_request_parts.headers.insert("x-forwarded-proto", "http".parse()?);
      }

      if let Some(original_host) = original_host {
        auth_request_parts.headers.insert("x-forwarded-host", original_host);
      }

      auth_request_parts
        .headers
        .insert("x-forwarded-uri", path_and_query.parse()?);

      auth_request_parts
        .headers
        .insert("x-forwarded-method", request_parts.method.as_str().parse()?);

      auth_request_parts.method = Method::GET;
      auth_request_parts.version = Version::HTTP_11;
      let auth_request = Request::from_parts(auth_request_parts, Empty::new().map_err(|e| match e {}).boxed());

      let original_request = Request::from_parts(request_parts, request_body);

      let connections = &self.connections[rand::random_range(..self.connections.len())];

      let rwlock_read = connections.read().await;
      let sender_read_option = rwlock_read.get(&addr);

      if let Some(sender_read) = sender_read_option {
        if match sender_read {
          SendRequest::Http1(sender) => !sender.is_closed(),
          SendRequest::Http2(sender) => !sender.is_closed(),
        } {
          drop(rwlock_read);
          let mut rwlock_write = connections.write().await;
          let sender_option = rwlock_write.get_mut(&addr);

          if let Some(sender) = sender_option {
            if match sender {
              SendRequest::Http1(sender) => !sender.is_closed() && sender.ready().await.is_ok(),
              SendRequest::Http2(sender) => !sender.is_closed() && sender.ready().await.is_ok(),
            } {
              let result = http_forwarded_auth_kept_alive(
                sender,
                auth_request,
                error_logger,
                original_request,
                forwarded_auth_copy_headers,
              )
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

        http_forwarded_auth(
          connections,
          addr,
          rw,
          auth_request,
          error_logger,
          original_request,
          forwarded_auth_copy_headers,
          false,
        )
        .await
      } else {
        let mut tls_client_config = (if disable_certificate_verification {
          rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoServerVerifier::new()))
        } else {
          rustls::ClientConfig::builder().with_platform_verifier()?
        })
        .with_no_client_auth();
        tls_client_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec(), b"http/1.0".to_vec()];
        let connector = TlsConnector::from(Arc::new(tls_client_config));
        let domain = ServerName::try_from(host)?.to_owned();

        let tls_stream = match connector.connect(domain, stream).await {
          Ok(stream) => stream,
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

        // Enable HTTP/2 when the ALPN protocol is "h2"
        let enable_http2 = tls_stream.get_ref().1.alpn_protocol() == Some(b"h2");

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

        http_forwarded_auth(
          connections,
          addr,
          rw,
          auth_request,
          error_logger,
          original_request,
          forwarded_auth_copy_headers,
          enable_http2,
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

#[allow(clippy::too_many_arguments)]
async fn http_forwarded_auth(
  connections: &RwLock<HashMap<String, SendRequest>>,
  connect_addr: String,
  stream: impl AsyncRead + AsyncWrite + Send + Unpin + 'static,
  proxy_request: Request<BoxBody<Bytes, std::io::Error>>,
  error_logger: &ErrorLogger,
  mut original_request: Request<BoxBody<Bytes, std::io::Error>>,
  forwarded_auth_copy_headers: Vec<String>,
  use_http2: bool,
) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
  #[cfg(feature = "runtime-monoio")]
  let io = MonoioIo::new(stream);
  #[cfg(feature = "runtime-tokio")]
  let io = TokioIo::new(stream);

  let mut sender = if use_http2 {
    #[cfg(feature = "runtime-monoio")]
    let executor = MonoioExecutor;
    #[cfg(feature = "runtime-tokio")]
    let executor = TokioExecutor::new();

    let (sender, conn) = match hyper::client::conn::http2::handshake(executor, io).await {
      Ok(data) => data,
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

    ferron_common::runtime::spawn(async move {
      conn.await.unwrap_or_default();
    });

    SendRequest::Http2(sender)
  } else {
    let (sender, conn) = match hyper::client::conn::http1::handshake(io).await {
      Ok(data) => data,
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

    ferron_common::runtime::spawn(async move {
      conn.await.unwrap_or_default();
    });

    SendRequest::Http1(sender)
  };

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

  let response = if proxy_response.status().is_success() {
    if !forwarded_auth_copy_headers.is_empty() {
      let response_headers = proxy_response.headers();
      let request_headers = original_request.headers_mut();
      for forwarded_auth_copy_header_string in forwarded_auth_copy_headers.iter() {
        let forwarded_auth_copy_header = HeaderName::from_str(forwarded_auth_copy_header_string)?;
        if response_headers.contains_key(&forwarded_auth_copy_header) {
          while request_headers.remove(&forwarded_auth_copy_header).is_some() {}
          for header_value in response_headers.get_all(&forwarded_auth_copy_header).iter() {
            request_headers.append(&forwarded_auth_copy_header, header_value.clone());
          }
        }
      }
    }
    ResponseData {
      request: Some(original_request),
      response: None,
      response_status: None,
      response_headers: None,
      new_remote_address: None,
    }
  } else {
    ResponseData {
      request: None,
      response: Some(proxy_response.map(|b| b.map_err(|e| std::io::Error::other(e.to_string())).boxed())),
      response_status: None,
      response_headers: None,
      new_remote_address: None,
    }
  };

  if !(match &sender {
    SendRequest::Http1(sender) => sender.is_closed(),
    SendRequest::Http2(sender) => sender.is_closed(),
  }) {
    let mut rwlock_write = connections.write().await;
    rwlock_write.insert(connect_addr, sender);
    drop(rwlock_write);
  }

  Ok(response)
}

async fn http_forwarded_auth_kept_alive(
  sender: &mut SendRequest,
  proxy_request: Request<BoxBody<Bytes, std::io::Error>>,
  error_logger: &ErrorLogger,
  mut original_request: Request<BoxBody<Bytes, std::io::Error>>,
  forwarded_auth_copy_headers: Vec<String>,
) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
  let send_request_result = match sender {
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

  let response = if proxy_response.status().is_success() {
    if !forwarded_auth_copy_headers.is_empty() {
      let response_headers = proxy_response.headers();
      let request_headers = original_request.headers_mut();
      for forwarded_auth_copy_header_string in forwarded_auth_copy_headers.iter() {
        let forwarded_auth_copy_header = HeaderName::from_str(forwarded_auth_copy_header_string)?;
        if response_headers.contains_key(&forwarded_auth_copy_header) {
          while request_headers.remove(&forwarded_auth_copy_header).is_some() {}
          for header_value in response_headers.get_all(&forwarded_auth_copy_header).iter() {
            request_headers.append(&forwarded_auth_copy_header, header_value.clone());
          }
        }
      }
    }
    ResponseData {
      request: Some(original_request),
      response: None,
      response_status: None,
      response_headers: None,
      new_remote_address: None,
    }
  } else {
    ResponseData {
      request: None,
      response: Some(proxy_response.map(|b| b.map_err(|e| std::io::Error::other(e.to_string())).boxed())),
      response_status: None,
      response_headers: None,
      new_remote_address: None,
    }
  };

  Ok(response)
}
