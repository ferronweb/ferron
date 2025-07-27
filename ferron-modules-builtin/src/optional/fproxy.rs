use std::collections::HashSet;
use std::error::Error;
use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty};
use hyper::{header, Request, Response, StatusCode, Uri};
#[cfg(feature = "runtime-tokio")]
use hyper_util::rt::TokioIo;
#[cfg(feature = "runtime-monoio")]
use monoio::io::IntoPollIo;
#[cfg(feature = "runtime-monoio")]
use monoio::net::TcpStream;
#[cfg(feature = "runtime-monoio")]
use monoio_compat::hyper::MonoioIo;
use tokio::io::{AsyncRead, AsyncWrite};
#[cfg(feature = "runtime-tokio")]
use tokio::net::TcpStream;

use ferron_common::config::ServerConfiguration;
use ferron_common::get_entries_for_validation;
use ferron_common::logging::ErrorLogger;
use ferron_common::modules::{Module, ModuleHandlers, ModuleLoader, ResponseData, SocketData};
use ferron_common::util::ModuleCache;

/// A forward proxy fallback module loader
pub struct ForwardProxyModuleLoader {
  cache: ModuleCache<ForwardProxyModule>,
}

impl Default for ForwardProxyModuleLoader {
  fn default() -> Self {
    Self::new()
  }
}

impl ForwardProxyModuleLoader {
  /// Creates a new module loader
  pub fn new() -> Self {
    Self {
      cache: ModuleCache::new(vec![]),
    }
  }
}

impl ModuleLoader for ForwardProxyModuleLoader {
  fn load_module(
    &mut self,
    config: &ServerConfiguration,
    _global_config: Option<&ServerConfiguration>,
    _secondary_runtime: &tokio::runtime::Runtime,
  ) -> Result<Arc<dyn Module + Send + Sync>, Box<dyn Error + Send + Sync>> {
    Ok(
      self
        .cache
        .get_or_init::<_, Box<dyn std::error::Error + Send + Sync>>(config, move |_| {
          Ok(Arc::new(ForwardProxyModule))
        })?,
    )
  }

  fn get_requirements(&self) -> Vec<&'static str> {
    vec!["forward_proxy"]
  }

  fn validate_configuration(
    &self,
    config: &ServerConfiguration,
    used_properties: &mut HashSet<String>,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    if let Some(entries) = get_entries_for_validation!("forward_proxy", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `forward_proxy` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!("Invalid forward proxy enabling option"))?
        }
      }
    };

    Ok(())
  }
}

/// A forward proxy fallback module
struct ForwardProxyModule;

impl Module for ForwardProxyModule {
  fn get_module_handlers(&self) -> Box<dyn ModuleHandlers> {
    Box::new(ForwardProxyModuleHandlers)
  }
}

/// Handlers for the forward proxy fallback module
struct ForwardProxyModuleHandlers;

#[async_trait(?Send)]
impl ModuleHandlers for ForwardProxyModuleHandlers {
  async fn request_handler(
    &mut self,
    request: Request<BoxBody<Bytes, std::io::Error>>,
    _config: &ServerConfiguration,
    _socket_data: &SocketData,
    error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    // Determine if the request is a forward proxy request
    let is_proxy_request = match request.version() {
      hyper::Version::HTTP_2 | hyper::Version::HTTP_3 => {
        request.method() == hyper::Method::CONNECT && request.uri().host().is_some()
      }
      _ => request.uri().host().is_some(),
    };
    let is_connect_proxy_request = request.method() == hyper::Method::CONNECT;

    if is_connect_proxy_request {
      if let Some(connect_address) = request.uri().authority().map(|auth| auth.to_string()) {
        let error_logger = error_logger.clone();
        ferron_common::runtime::spawn(async move {
          match hyper::upgrade::on(request).await {
            Ok(upgraded_request) => {
              let stream = match TcpStream::connect(connect_address).await {
                Ok(stream) => stream,
                Err(err) => {
                  error_logger
                    .log(&format!("Cannot connect to the remote server: {err}"))
                    .await;
                  return;
                }
              };
              match stream.set_nodelay(true) {
                Ok(_) => (),
                Err(err) => {
                  error_logger
                    .log(&format!(
                      "Cannot disable Nagle algorithm when connecting to the remote server: {err}"
                    ))
                    .await;
                  return;
                }
              };
              #[cfg(feature = "runtime-monoio")]
              let mut stream = match stream.into_poll_io() {
                Ok(stream) => stream,
                Err(err) => {
                  error_logger
                    .log(&format!("Cannot convert the TCP stream into polled I/O: {err}"))
                    .await;
                  return;
                }
              };
              #[cfg(feature = "runtime-tokio")]
              let mut stream = stream;

              #[cfg(feature = "runtime-monoio")]
              let mut upgraded = MonoioIo::new(upgraded_request);
              #[cfg(feature = "runtime-tokio")]
              let mut upgraded = TokioIo::new(upgraded_request);

              tokio::io::copy_bidirectional(&mut upgraded, &mut stream)
                .await
                .unwrap_or_default();
            }
            Err(err) => {
              error_logger
                .log(&format!("Error while upgrading HTTP CONNECT request: {err}"))
                .await;
            }
          }
        });

        Ok(ResponseData {
          request: None,
          response: Some(
            Response::builder()
              .body(Empty::new().map_err(|e| match e {}).boxed())
              .unwrap_or_default(),
          ),
          response_status: None,
          response_headers: None,
          new_remote_address: None,
        })
      } else {
        Ok(ResponseData {
          request: Some(request),
          response: None,
          response_status: Some(StatusCode::BAD_REQUEST),
          response_headers: None,
          new_remote_address: None,
        })
      }
    } else if is_proxy_request {
      // Code taken from reverse proxy module
      let (mut request_parts, request_body) = request.into_parts();

      match request_parts.uri.scheme_str() {
        Some("http") | None => (),
        _ => {
          return Ok(ResponseData {
            request: None,
            response: None,
            response_status: Some(StatusCode::BAD_REQUEST),
            response_headers: None,
            new_remote_address: None,
          });
        }
      };

      let host = match request_parts.uri.host() {
        Some(host) => host,
        None => {
          return Ok(ResponseData {
            request: None,
            response: None,
            response_status: Some(StatusCode::BAD_REQUEST),
            response_headers: None,
            new_remote_address: None,
          });
        }
      };

      let port = request_parts.uri.port_u16().unwrap_or(80);

      let addr = format!("{host}:{port}");
      let stream = match TcpStream::connect(addr).await {
        Ok(stream) => stream,
        Err(err) => {
          match err.kind() {
            tokio::io::ErrorKind::ConnectionRefused
            | tokio::io::ErrorKind::NotFound
            | tokio::io::ErrorKind::HostUnreachable => {
              error_logger.log(&format!("Service unavailable: {err}")).await;
              return Ok(ResponseData {
                request: None,
                response: None,
                response_status: Some(StatusCode::SERVICE_UNAVAILABLE),
                response_headers: None,
                new_remote_address: None,
              });
            }
            tokio::io::ErrorKind::TimedOut => {
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

      let request_path = request_parts.uri.path();

      request_parts.uri = Uri::from_str(&format!(
        "{}{}",
        request_path,
        match request_parts.uri.query() {
          Some(query) => format!("?{query}"),
          None => "".to_string(),
        }
      ))?;

      // Connection header to disable HTTP/1.1 keep-alive
      request_parts.headers.insert(header::CONNECTION, "close".parse()?);

      let proxy_request = Request::from_parts(request_parts, request_body);

      http_proxy(stream, proxy_request, error_logger).await
    } else {
      Ok(ResponseData {
        request: Some(request),
        response: None,
        response_status: None,
        response_headers: None,
        new_remote_address: None,
      })
    }
  }
}

async fn http_proxy(
  stream: impl AsyncRead + AsyncWrite + Unpin + 'static,
  proxy_request: Request<BoxBody<Bytes, std::io::Error>>,
  error_logger: &ErrorLogger,
) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
  #[cfg(feature = "runtime-monoio")]
  let io = MonoioIo::new(stream);
  #[cfg(feature = "runtime-tokio")]
  let io = TokioIo::new(stream);

  let (mut sender, conn) = match hyper::client::conn::http1::handshake(io).await {
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

  Ok(ResponseData {
    request: None,
    response: Some(proxy_response.map(|b| b.map_err(|e| std::io::Error::other(e.to_string())).boxed())),
    response_status: None,
    response_headers: None,
    new_remote_address: None,
  })
}
