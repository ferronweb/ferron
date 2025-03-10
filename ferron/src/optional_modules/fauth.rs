// The "fauth" module is derived from the "rproxy" module, and inspired by Traefik's ForwardAuth middleware.

use std::collections::HashMap;
use std::error::Error;
use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use ferron_common::{
  ErrorLogger, HyperUpgraded, RequestData, ResponseData, ServerConfig, ServerConfigRoot,
  ServerModule, ServerModuleHandlers, SocketData,
};
use ferron_common::{HyperResponse, WithRuntime};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty};
use hyper::body::Bytes;
use hyper::client::conn::http1::SendRequest;
use hyper::header::HeaderName;
use hyper::{header, Method, Request, StatusCode, Uri};
use hyper_tungstenite::HyperWebsocket;
use hyper_util::rt::TokioIo;
use rustls::pki_types::ServerName;
use rustls::RootCertStore;
use rustls_native_certs::load_native_certs;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::runtime::Handle;
use tokio::sync::RwLock;
use tokio_rustls::TlsConnector;

const DEFAULT_CONCURRENT_CONNECTIONS_PER_HOST: u32 = 32;

pub fn server_module_init(
  _config: &ServerConfig,
) -> Result<Box<dyn ServerModule + Send + Sync>, Box<dyn Error + Send + Sync>> {
  let mut roots: RootCertStore = RootCertStore::empty();
  let certs_result = load_native_certs();
  if !certs_result.errors.is_empty() {
    Err(anyhow::anyhow!(format!(
      "Couldn't load the native certificate store: {}",
      certs_result.errors[0]
    )))?
  }
  let certs = certs_result.certs;

  for cert in certs {
    match roots.add(cert) {
      Ok(_) => (),
      Err(err) => Err(anyhow::anyhow!(format!(
        "Couldn't add a certificate to the certificate store: {}",
        err
      )))?,
    }
  }

  let mut connections_vec = Vec::new();
  for _ in 0..DEFAULT_CONCURRENT_CONNECTIONS_PER_HOST {
    connections_vec.push(RwLock::new(HashMap::new()));
  }
  Ok(Box::new(ForwardedAuthenticationModule::new(
    Arc::new(roots),
    Arc::new(connections_vec),
  )))
}

#[allow(clippy::type_complexity)]
struct ForwardedAuthenticationModule {
  roots: Arc<RootCertStore>,
  connections: Arc<Vec<RwLock<HashMap<String, SendRequest<BoxBody<Bytes, hyper::Error>>>>>>,
}

impl ForwardedAuthenticationModule {
  #[allow(clippy::type_complexity)]
  fn new(
    roots: Arc<RootCertStore>,
    connections: Arc<Vec<RwLock<HashMap<String, SendRequest<BoxBody<Bytes, hyper::Error>>>>>>,
  ) -> Self {
    ForwardedAuthenticationModule { roots, connections }
  }
}

impl ServerModule for ForwardedAuthenticationModule {
  fn get_handlers(&self, handle: Handle) -> Box<dyn ServerModuleHandlers + Send> {
    Box::new(ForwardedAuthenticationModuleHandlers {
      roots: self.roots.clone(),
      connections: self.connections.clone(),
      handle,
    })
  }
}

#[allow(clippy::type_complexity)]
struct ForwardedAuthenticationModuleHandlers {
  handle: Handle,
  roots: Arc<RootCertStore>,
  connections: Arc<Vec<RwLock<HashMap<String, SendRequest<BoxBody<Bytes, hyper::Error>>>>>>,
}

#[async_trait]
impl ServerModuleHandlers for ForwardedAuthenticationModuleHandlers {
  async fn request_handler(
    &mut self,
    request: RequestData,
    config: &ServerConfigRoot,
    socket_data: &SocketData,
    error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    WithRuntime::new(self.handle.clone(), async move {
      let mut auth_to = None;

      if let Some(auth_to_str) = config.get("authTo").as_str() {
        auth_to = Some(auth_to_str.to_string());
      }

      let forwarded_auth_copy_headers = match config.get("forwardedAuthCopyHeaders").as_vec() {
        Some(vector) => {
          let mut new_vector = Vec::new();
          for yaml_value in vector.iter() {
            if let Some(str_value) = yaml_value.as_str() {
              new_vector.push(str_value.to_string());
            }
          }
          new_vector
        }
        None => Vec::new(),
      };

      if let Some(auth_to) = auth_to {
        let (hyper_request, auth_user) = request.into_parts();
        let (hyper_request_parts, request_body) = hyper_request.into_parts();

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
          _ => Err(anyhow::anyhow!(
            "Only HTTP and HTTPS reverse proxy URLs are supported."
          ))?,
        };

        let host = match auth_request_url.host() {
          Some(host) => host,
          None => Err(anyhow::anyhow!(
            "The reverse proxy URL doesn't include the host"
          ))?,
        };

        let port = auth_request_url.port_u16().unwrap_or(match scheme_str {
          Some("http") => 80,
          Some("https") => 443,
          _ => 80,
        });

        let addr = format!("{}:{}", host, port);
        let authority = auth_request_url.authority().cloned();

        let hyper_request_path = hyper_request_parts.uri.path();

        let path_and_query = format!(
          "{}{}",
          hyper_request_path,
          match hyper_request_parts.uri.query() {
            Some(query) => format!("?{}", query),
            None => "".to_string(),
          }
        );

        let mut auth_hyper_request_parts = hyper_request_parts.clone();

        auth_hyper_request_parts.uri = Uri::from_str(&format!(
          "{}{}",
          auth_request_url.path(),
          match auth_request_url.query() {
            Some(query) => format!("?{}", query),
            None => "".to_string(),
          }
        ))?;

        let original_host = hyper_request_parts.headers.get(header::HOST).cloned();

        // Host header for host identification
        match authority {
          Some(authority) => {
            auth_hyper_request_parts
              .headers
              .insert(header::HOST, authority.to_string().parse()?);
          }
          None => {
            auth_hyper_request_parts.headers.remove(header::HOST);
          }
        }

        // Connection header to enable HTTP/1.1 keep-alive
        auth_hyper_request_parts
          .headers
          .insert(header::CONNECTION, "keep-alive".parse()?);

        // X-Forwarded-* headers to send the client's data to a forwarded authentication server
        auth_hyper_request_parts.headers.insert(
          "x-forwarded-for",
          socket_data
            .remote_addr
            .ip()
            .to_canonical()
            .to_string()
            .parse()?,
        );

        if socket_data.encrypted {
          auth_hyper_request_parts
            .headers
            .insert("x-forwarded-proto", "https".parse()?);
        } else {
          auth_hyper_request_parts
            .headers
            .insert("x-forwarded-proto", "http".parse()?);
        }

        if let Some(original_host) = original_host {
          auth_hyper_request_parts
            .headers
            .insert("x-forwarded-host", original_host);
        }

        auth_hyper_request_parts
          .headers
          .insert("x-forwarded-uri", path_and_query.parse()?);

        auth_hyper_request_parts.headers.insert(
          "x-forwarded-method",
          hyper_request_parts.method.as_str().parse()?,
        );

        auth_hyper_request_parts.method = Method::GET;

        let auth_request = Request::from_parts(
          auth_hyper_request_parts,
          Empty::new().map_err(|e| match e {}).boxed(),
        );
        let original_hyper_request = Request::from_parts(hyper_request_parts, request_body);
        let original_request = RequestData::new(original_hyper_request, auth_user);

        let connections = &self.connections[rand::random_range(..self.connections.len())];

        let rwlock_read = connections.read().await;
        let sender_read_option = rwlock_read.get(&addr);

        if let Some(sender_read) = sender_read_option {
          if !sender_read.is_closed() {
            drop(rwlock_read);
            let mut rwlock_write = connections.write().await;
            let sender_option = rwlock_write.get_mut(&addr);

            if let Some(sender) = sender_option {
              if !sender.is_closed() {
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
              tokio::io::ErrorKind::ConnectionRefused
              | tokio::io::ErrorKind::NotFound
              | tokio::io::ErrorKind::HostUnreachable => {
                error_logger
                  .log(&format!("Service unavailable: {}", err))
                  .await;
                return Ok(
                  ResponseData::builder_without_request()
                    .status(StatusCode::SERVICE_UNAVAILABLE)
                    .build(),
                );
              }
              tokio::io::ErrorKind::TimedOut => {
                error_logger.log(&format!("Gateway timeout: {}", err)).await;
                return Ok(
                  ResponseData::builder_without_request()
                    .status(StatusCode::GATEWAY_TIMEOUT)
                    .build(),
                );
              }
              _ => {
                error_logger.log(&format!("Bad gateway: {}", err)).await;
                return Ok(
                  ResponseData::builder_without_request()
                    .status(StatusCode::BAD_GATEWAY)
                    .build(),
                );
              }
            };
          }
        };

        match stream.set_nodelay(true) {
          Ok(_) => (),
          Err(err) => {
            error_logger.log(&format!("Bad gateway: {}", err)).await;
            return Ok(
              ResponseData::builder_without_request()
                .status(StatusCode::BAD_GATEWAY)
                .build(),
            );
          }
        };

        if !encrypted {
          http_forwarded_auth(
            connections,
            addr,
            stream,
            auth_request,
            error_logger,
            original_request,
            forwarded_auth_copy_headers,
          )
          .await
        } else {
          let tls_client_config = rustls::ClientConfig::builder()
            .with_root_certificates(self.roots.clone())
            .with_no_client_auth();
          let connector = TlsConnector::from(Arc::new(tls_client_config));
          let domain = ServerName::try_from(host)?.to_owned();

          let tls_stream = match connector.connect(domain, stream).await {
            Ok(stream) => stream,
            Err(err) => {
              error_logger.log(&format!("Bad gateway: {}", err)).await;
              return Ok(
                ResponseData::builder_without_request()
                  .status(StatusCode::BAD_GATEWAY)
                  .build(),
              );
            }
          };

          http_forwarded_auth(
            connections,
            addr,
            tls_stream,
            auth_request,
            error_logger,
            original_request,
            forwarded_auth_copy_headers,
          )
          .await
        }
      } else {
        Ok(ResponseData::builder(request).build())
      }
    })
    .await
  }

  async fn proxy_request_handler(
    &mut self,
    request: RequestData,
    _config: &ServerConfigRoot,
    _socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    Ok(ResponseData::builder(request).build())
  }

  async fn response_modifying_handler(
    &mut self,
    response: HyperResponse,
  ) -> Result<HyperResponse, Box<dyn Error + Send + Sync>> {
    Ok(response)
  }

  async fn proxy_response_modifying_handler(
    &mut self,
    response: HyperResponse,
  ) -> Result<HyperResponse, Box<dyn Error + Send + Sync>> {
    Ok(response)
  }

  async fn connect_proxy_request_handler(
    &mut self,
    _upgraded_request: HyperUpgraded,
    _connect_address: &str,
    _config: &ServerConfigRoot,
    _socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    Ok(())
  }

  fn does_connect_proxy_requests(&mut self) -> bool {
    false
  }

  async fn websocket_request_handler(
    &mut self,
    _websocket: HyperWebsocket,
    _config: &ServerConfigRoot,
    _socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    Ok(())
  }

  fn does_websocket_requests(&mut self, _config: &ServerConfigRoot) -> bool {
    false
  }
}

async fn http_forwarded_auth(
  connections: &RwLock<HashMap<String, SendRequest<BoxBody<Bytes, hyper::Error>>>>,
  connect_addr: String,
  stream: impl AsyncRead + AsyncWrite + Send + Unpin + 'static,
  proxy_request: Request<BoxBody<Bytes, hyper::Error>>,
  error_logger: &ErrorLogger,
  mut original_request: RequestData,
  forwarded_auth_copy_headers: Vec<String>,
) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
  let io = TokioIo::new(stream);

  let (mut sender, conn) = match hyper::client::conn::http1::handshake(io).await {
    Ok(data) => data,
    Err(err) => {
      error_logger.log(&format!("Bad gateway: {}", err)).await;
      return Ok(
        ResponseData::builder_without_request()
          .status(StatusCode::BAD_GATEWAY)
          .build(),
      );
    }
  };

  let send_request = sender.send_request(proxy_request);

  let mut pinned_conn = Box::pin(conn);
  tokio::pin!(send_request);

  let response;

  loop {
    tokio::select! {
      biased;

       proxy_response = &mut send_request => {
        let proxy_response = match proxy_response {
          Ok(response) => response,
          Err(err) => {
            error_logger.log(&format!("Bad gateway: {}", err)).await;
            return Ok(ResponseData::builder_without_request().status(StatusCode::BAD_GATEWAY).build());
          }
        };

        if proxy_response.status().is_success() {
          if !forwarded_auth_copy_headers.is_empty() {
            let response_headers = proxy_response.headers();
            let request_headers = original_request.get_mut_hyper_request().headers_mut();
            for forwarded_auth_copy_header_string in forwarded_auth_copy_headers.iter() {
              let forwarded_auth_copy_header= HeaderName::from_str(forwarded_auth_copy_header_string)?;
              if response_headers.contains_key(&forwarded_auth_copy_header) {
                while request_headers.remove(&forwarded_auth_copy_header).is_some() {}
                for header_value in response_headers.get_all(&forwarded_auth_copy_header).iter() {
                  request_headers.append(&forwarded_auth_copy_header, header_value.clone());
                }
              }
            }
          }
          response = ResponseData::builder(original_request).build();
        } else {
          response = ResponseData::builder_without_request()
          .response(proxy_response.map(|b| {
            b.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
              .boxed()
          }))
          .parallel_fn(async move {
            pinned_conn.await.unwrap_or_default();
          })
          .build();

        }

        break;
      },
      state = &mut pinned_conn => {
        if state.is_err() {
          error_logger.log("Bad gateway: incomplete response").await;
          return Ok(ResponseData::builder_without_request().status(StatusCode::BAD_GATEWAY).build());
        }
      },
    };
  }

  if !sender.is_closed() {
    let mut rwlock_write = connections.write().await;
    rwlock_write.insert(connect_addr, sender);
    drop(rwlock_write);
  }

  Ok(response)
}

async fn http_forwarded_auth_kept_alive(
  sender: &mut SendRequest<BoxBody<Bytes, hyper::Error>>,
  proxy_request: Request<BoxBody<Bytes, hyper::Error>>,
  error_logger: &ErrorLogger,
  mut original_request: RequestData,
  forwarded_auth_copy_headers: Vec<String>,
) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
  let proxy_response = match sender.send_request(proxy_request).await {
    Ok(response) => response,
    Err(err) => {
      error_logger.log(&format!("Bad gateway: {}", err)).await;
      return Ok(
        ResponseData::builder_without_request()
          .status(StatusCode::BAD_GATEWAY)
          .build(),
      );
    }
  };

  let response = if proxy_response.status().is_success() {
    if !forwarded_auth_copy_headers.is_empty() {
      let response_headers = proxy_response.headers();
      let request_headers = original_request.get_mut_hyper_request().headers_mut();
      for forwarded_auth_copy_header_string in forwarded_auth_copy_headers.iter() {
        let forwarded_auth_copy_header = HeaderName::from_str(forwarded_auth_copy_header_string)?;
        if response_headers.contains_key(&forwarded_auth_copy_header) {
          while request_headers
            .remove(&forwarded_auth_copy_header)
            .is_some()
          {}
          for header_value in response_headers.get_all(&forwarded_auth_copy_header).iter() {
            request_headers.append(&forwarded_auth_copy_header, header_value.clone());
          }
        }
      }
    }
    ResponseData::builder(original_request).build()
  } else {
    ResponseData::builder_without_request()
      .response(proxy_response.map(|b| {
        b.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
          .boxed()
      }))
      .build()
  };

  Ok(response)
}
