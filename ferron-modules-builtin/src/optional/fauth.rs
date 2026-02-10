use std::cell::UnsafeCell;
use std::error::Error;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use connpool::{Item, Pool};
use futures_util::FutureExt;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty};
use hyper::body::Body;
use hyper::header::{self, HeaderName};
use hyper::{Method, Request, StatusCode, Uri, Version};
#[cfg(feature = "runtime-tokio")]
use hyper_util::rt::{TokioExecutor, TokioIo};
#[cfg(feature = "runtime-monoio")]
use monoio::net::TcpStream;
#[cfg(feature = "runtime-monoio")]
use monoio_compat::hyper::{MonoioExecutor, MonoioIo};
use rustls::client::WebPkiServerVerifier;
use rustls_pki_types::ServerName;
use rustls_platform_verifier::BuilderVerifierExt;
use tokio::io::{AsyncRead, AsyncWrite};
#[cfg(feature = "runtime-tokio")]
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use ferron_common::logging::ErrorLogger;
use ferron_common::modules::{Module, ModuleHandlers, ModuleLoader, ResponseData, SocketData};
use ferron_common::util::NoServerVerifier;
use ferron_common::{config::ServerConfiguration, util::ModuleCache};
use ferron_common::{get_entries_for_validation, get_entry, get_value, get_values};

use crate::util::http_proxy::{SendRequest, SendRequestWrapper};
#[cfg(feature = "runtime-monoio")]
use crate::util::SendTcpStreamPoll;

const DEFAULT_CONCURRENT_CONNECTIONS: usize = 16384;
const DEFAULT_KEEPALIVE_IDLE_TIMEOUT: u64 = 60000;

type ConnectionPool = Arc<Pool<Arc<String>, SendRequestWrapper>>;
type ConnectionPoolItem = Item<Arc<String>, SendRequestWrapper>;

/// A tracked response body
struct TrackedBody<B> {
  inner: B,
  _tracker_pool: Option<Arc<UnsafeCell<ConnectionPoolItem>>>,
}

impl<B> TrackedBody<B> {
  fn new(inner: B, tracker_pool: Option<Arc<UnsafeCell<ConnectionPoolItem>>>) -> Self {
    Self {
      inner,
      _tracker_pool: tracker_pool,
    }
  }
}

impl<B> Body for TrackedBody<B>
where
  B: Body + Unpin,
{
  type Data = B::Data;
  type Error = B::Error;

  #[inline]
  fn poll_frame(
    mut self: Pin<&mut Self>,
    cx: &mut Context<'_>,
  ) -> Poll<Option<Result<hyper::body::Frame<Self::Data>, Self::Error>>> {
    Pin::new(&mut self.inner).poll_frame(cx)
  }

  #[inline]
  fn is_end_stream(&self) -> bool {
    self.inner.is_end_stream()
  }

  #[inline]
  fn size_hint(&self) -> hyper::body::SizeHint {
    self.inner.size_hint()
  }
}

// Safety: after construction, the value inside `UnsafeCell` is never mutated.
// All accesses after sharing are read-only, so sharing across threads is safe.
unsafe impl<B> Send for TrackedBody<B> where B: Send {}
unsafe impl<B> Sync for TrackedBody<B> where B: Sync {}

/// A forwarded authentication module loader
pub struct ForwardedAuthenticationModuleLoader {
  cache: ModuleCache<ForwardedAuthenticationModule>,
  connections: Option<ConnectionPool>,
}

impl Default for ForwardedAuthenticationModuleLoader {
  fn default() -> Self {
    Self::new()
  }
}

impl ForwardedAuthenticationModuleLoader {
  /// Creates a new module loader
  pub fn new() -> Self {
    Self {
      cache: ModuleCache::new(vec!["auth_to"]),
      connections: None,
    }
  }
}

impl ModuleLoader for ForwardedAuthenticationModuleLoader {
  fn load_module(
    &mut self,
    config: &ServerConfiguration,
    global_config: Option<&ServerConfiguration>,
    _secondary_runtime: &tokio::runtime::Runtime,
  ) -> Result<Arc<dyn Module + Send + Sync>, Box<dyn Error + Send + Sync>> {
    let concurrency_limit = global_config
      .and_then(|c| get_value!("auth_to_concurrent_conns", c))
      .map_or(Some(DEFAULT_CONCURRENT_CONNECTIONS), |v| {
        if v.is_null() {
          None
        } else {
          Some(
            v.as_i128()
              .map(|v| v as usize)
              .unwrap_or(DEFAULT_CONCURRENT_CONNECTIONS),
          )
        }
      });
    let connections = self
      .connections
      .get_or_insert(Arc::new(if let Some(limit) = concurrency_limit {
        Pool::new(limit)
      } else {
        Pool::new_unbounded()
      }))
      .clone();
    Ok(
      self
        .cache
        .get_or_init::<_, Box<dyn std::error::Error + Send + Sync>>(config, |config| {
          let auth_to_entry = get_entry!("auth_to", config);

          Ok(Arc::new(ForwardedAuthenticationModule {
            auth_to: auth_to_entry
              .and_then(|e| e.values.first())
              .and_then(|v| v.as_str())
              .map(|v| Arc::new(v.to_owned())),
            local_limit_index: auth_to_entry
              .and_then(|e| e.props.get("limit"))
              .and_then(|v| v.as_i128())
              .map(|v| v as usize)
              .map(|l| connections.set_local_limit(l)),
            idle_timeout: auth_to_entry
              .and_then(|e| e.props.get("idle_timeout"))
              .map_or(Some(DEFAULT_KEEPALIVE_IDLE_TIMEOUT), |v| {
                if v.is_null() {
                  None
                } else {
                  Some(v.as_i128().map(|v| v as u64).unwrap_or(DEFAULT_KEEPALIVE_IDLE_TIMEOUT))
                }
              })
              .map(Duration::from_millis),
            connections,
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
        if let Some(prop) = entry.props.get("limit") {
          if !prop.is_null() && prop.as_i128().unwrap_or(0) < 1 {
            Err(anyhow::anyhow!(
              "Invalid forwarded authentication connection limit for a backend server"
            ))?
          }
        }
        if let Some(prop) = entry.props.get("idle_timeout") {
          if !prop.is_null() && prop.as_i128().unwrap_or(0) < 1 {
            Err(anyhow::anyhow!(
              "Invalid forwarded authentication idle keep-alive connection timeout for a backend server"
            ))?
          }
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

    if let Some(entries) = get_entries_for_validation!("auth_to_concurrent_conns", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          return Err(
            anyhow::anyhow!("The `auth_to_concurrent_conns` configuration property must have exactly one value").into(),
          );
        } else if (!entry.values[0].is_integer() && !entry.values[0].is_null())
          || entry.values[0].as_i128().is_some_and(|v| v < 0)
        {
          return Err(
            anyhow::anyhow!("Invalid global maximum concurrent connections for forwarded authentication configuration")
              .into(),
          );
        }
      }
    }

    Ok(())
  }
}

/// A forwarded authentication module
struct ForwardedAuthenticationModule {
  auth_to: Option<Arc<String>>,
  local_limit_index: Option<usize>,
  idle_timeout: Option<Duration>,
  connections: ConnectionPool,
}

impl Module for ForwardedAuthenticationModule {
  fn get_module_handlers(&self) -> Box<dyn ModuleHandlers> {
    Box::new(ForwardedAuthenticationModuleHandlers {
      auth_to: self.auth_to.clone(),
      local_limit_index: self.local_limit_index,
      idle_timeout: self.idle_timeout,
      connections: self.connections.clone(),
    })
  }
}

/// Handlers for the forwarded authentication proxy module
struct ForwardedAuthenticationModuleHandlers {
  auth_to: Option<Arc<String>>,
  local_limit_index: Option<usize>,
  idle_timeout: Option<Duration>,
  connections: ConnectionPool,
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
    if let Some(auth_to_arc) = self.auth_to.clone() {
      let auth_to = &*auth_to_arc;
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
        HeaderName::from_static("x-forwarded-for"),
        socket_data.remote_addr.ip().to_canonical().to_string().parse()?,
      );

      if socket_data.encrypted {
        auth_request_parts
          .headers
          .insert(HeaderName::from_static("x-forwarded-proto"), "https".parse()?);
      } else {
        auth_request_parts
          .headers
          .insert(HeaderName::from_static("x-forwarded-proto"), "http".parse()?);
      }

      if let Some(original_host) = original_host {
        auth_request_parts
          .headers
          .insert(HeaderName::from_static("x-forwarded-host"), original_host);
      }

      auth_request_parts
        .headers
        .insert(HeaderName::from_static("x-forwarded-uri"), path_and_query.parse()?);

      auth_request_parts.headers.insert(
        HeaderName::from_static("x-forwarded-method"),
        request_parts.method.as_str().parse()?,
      );

      auth_request_parts.method = Method::GET;
      auth_request_parts.version = Version::HTTP_11;
      let auth_request = Request::from_parts(auth_request_parts, Empty::new().map_err(|e| match e {}).boxed());

      let original_request = Request::from_parts(request_parts, request_body);

      let connection_pool_item = {
        let connections = &self.connections;
        let sender;
        let mut send_request_items = Vec::new();
        loop {
          let mut send_request_item = if send_request_items.is_empty() {
            connections
              .pull_with_wait_local_limit(auth_to_arc.clone(), self.local_limit_index)
              .await
          } else if let Poll::Ready(send_request_item_option) = connections
            .pull_with_wait_local_limit(auth_to_arc.clone(), self.local_limit_index)
            .boxed_local()
            .poll_unpin(&mut Context::from_waker(Waker::noop()))
          {
            send_request_item_option
          } else {
            let send_request_items_taken = send_request_items;
            send_request_items = Vec::new();
            let fetch_nonready_send_request_fut = async {
              let result = futures_util::future::select_ok(send_request_items_taken).await;
              if let Ok((item, send_request_items_smaller)) = result {
                send_request_items = send_request_items_smaller;
                item
              } else {
                futures_util::future::pending().await
              }
            };
            ferron_common::runtime::select! {
              item = connections
                .pull_with_wait_local_limit(auth_to_arc.clone(), self.local_limit_index)
              => {
                item
              },
              item = fetch_nonready_send_request_fut => {
                item
              }
            }
          };
          if let Some(send_request) = send_request_item.inner_mut() {
            match send_request.get(self.idle_timeout) {
              (Some(send_request), true) => {
                // Connection ready, send a request to it
                send_request_items.clear();
                let _ = send_request_item.inner_mut().take();
                let result = http_forwarded_auth_kept_alive(
                  send_request,
                  send_request_item,
                  auth_request,
                  error_logger,
                  original_request,
                  forwarded_auth_copy_headers,
                )
                .await;
                return result;
              }
              (None, true) => {
                // Connection not ready
                let idle_timeout = self.idle_timeout;
                send_request_items.push(Box::pin(async move {
                  let inner_item = send_request_item.inner_mut();
                  if let Some(inner_item_2) = inner_item {
                    if !inner_item_2.wait_ready(idle_timeout).await {
                      // Connection closed or timed out
                      inner_item.take();
                      return Err(());
                    }
                    let _ = inner_item;
                    Ok(send_request_item)
                  } else {
                    Err(())
                  }
                }));
                continue;
              }
              (_, false) => {
                // Connection closed
                let _ = send_request_item.inner_mut().take();
                continue;
              }
            }
          }
          send_request_items.clear();
          sender = send_request_item;
          break;
        }
        sender
      };

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

      if let Err(err) = stream.set_nodelay(true) {
        error_logger.log(&format!("Bad gateway: {err}")).await;
        return Ok(ResponseData {
          request: None,
          response: None,
          response_status: Some(StatusCode::BAD_GATEWAY),
          response_headers: None,
          new_remote_address: None,
        });
      };

      #[cfg(feature = "runtime-monoio")]
      let stream = match SendTcpStreamPoll::new_comp_io(stream) {
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
        http_forwarded_auth(
          connection_pool_item,
          stream,
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
        } else if let Ok(client_config) = BuilderVerifierExt::with_platform_verifier(rustls::ClientConfig::builder()) {
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

        http_forwarded_auth(
          connection_pool_item,
          tls_stream,
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
  connection_pool_item: ConnectionPoolItem,
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

  let send_request_result = sender.send_request(proxy_request).await;
  #[allow(clippy::arc_with_non_send_sync)]
  let connection_pool_item = Arc::new(UnsafeCell::new(connection_pool_item));

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
      response: Some(proxy_response.map(|b| {
        TrackedBody::new(
          b.map_err(|e| std::io::Error::other(e.to_string())),
          if !sender.is_closed() {
            None
          } else {
            // Safety: this should be not modified, see the "unsafe" block below
            Some(connection_pool_item.clone())
          },
        )
        .boxed()
      })),
      response_status: None,
      response_headers: None,
      new_remote_address: None,
    }
  };

  // Store the HTTP connection in the connection pool for future reuse if it's still open
  if !sender.is_closed() {
    // Safety: this Arc is cloned twice (when there's HTTP upgrade and when keepalive is disabled),
    // but the clones' inner value isn't modified, so no race condition.
    // We could wrap this value in a Mutex, but it's not really necessary in this case.
    let connection_pool_item = unsafe { &mut *connection_pool_item.get() };
    connection_pool_item
      .inner_mut()
      .replace(SendRequestWrapper::new(sender));
  }

  drop(connection_pool_item);

  Ok(response)
}

async fn http_forwarded_auth_kept_alive(
  mut sender: SendRequest,
  connection_pool_item: ConnectionPoolItem,
  proxy_request: Request<BoxBody<Bytes, std::io::Error>>,
  error_logger: &ErrorLogger,
  mut original_request: Request<BoxBody<Bytes, std::io::Error>>,
  forwarded_auth_copy_headers: Vec<String>,
) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
  let send_request_result = sender.send_request(proxy_request).await;
  #[allow(clippy::arc_with_non_send_sync)]
  let connection_pool_item = Arc::new(UnsafeCell::new(connection_pool_item));

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
      response: Some(proxy_response.map(|b| {
        TrackedBody::new(
          b.map_err(|e| std::io::Error::other(e.to_string())),
          if !sender.is_closed() {
            None
          } else {
            // Safety: this should be not modified, see the "unsafe" block below
            Some(connection_pool_item.clone())
          },
        )
        .boxed()
      })),
      response_status: None,
      response_headers: None,
      new_remote_address: None,
    }
  };

  if !sender.is_closed() {
    // Safety: this Arc is cloned twice (when there's HTTP upgrade and when keepalive is disabled),
    // but the clones' inner value isn't modified, so no race condition.
    // We could wrap this value in a Mutex, but it's not really necessary in this case.
    let connection_pool_item = unsafe { &mut *connection_pool_item.get() };
    connection_pool_item
      .inner_mut()
      .replace(SendRequestWrapper::new(sender));
  }

  drop(connection_pool_item);

  Ok(response)
}
