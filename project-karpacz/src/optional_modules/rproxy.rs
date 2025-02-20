use std::collections::HashMap;
use std::error::Error;
use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use http_body_util::combinators::BoxBody;
use http_body_util::BodyExt;
use hyper::body::Bytes;
use hyper::client::conn::http1::SendRequest;
use hyper::{header, Request, StatusCode, Uri};
use hyper_util::rt::TokioIo;
use project_karpacz_common::{
  ErrorLogger, HyperUpgraded, RequestData, ResponseData, ServerConfig, ServerConfigRoot,
  ServerModule, ServerModuleHandlers, SocketData,
};
use project_karpacz_common::{HyperResponse, WithRuntime};
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
        "Couldn't load add a certificate to the certificate store: {}",
        err
      )))?,
    }
  }

  let mut connections_vec = Vec::new();
  for _ in 0..DEFAULT_CONCURRENT_CONNECTIONS_PER_HOST {
    connections_vec.push(RwLock::new(HashMap::new()));
  }
  Ok(Box::new(ReverseProxyModule::new(
    Arc::new(roots),
    Arc::new(connections_vec),
  )))
}

#[allow(clippy::type_complexity)]
struct ReverseProxyModule {
  roots: Arc<RootCertStore>,
  connections: Arc<Vec<RwLock<HashMap<String, SendRequest<BoxBody<Bytes, hyper::Error>>>>>>,
}

impl ReverseProxyModule {
  #[allow(clippy::type_complexity)]
  fn new(
    roots: Arc<RootCertStore>,
    connections: Arc<Vec<RwLock<HashMap<String, SendRequest<BoxBody<Bytes, hyper::Error>>>>>>,
  ) -> Self {
    ReverseProxyModule { roots, connections }
  }
}

impl ServerModule for ReverseProxyModule {
  fn get_handlers(&self, handle: Handle) -> Box<dyn ServerModuleHandlers + Send> {
    Box::new(ReverseProxyModuleHandlers {
      roots: self.roots.clone(),
      connections: self.connections.clone(),
      handle,
    })
  }
}

struct ReverseProxyModuleHandlers {
  handle: Handle,
  roots: Arc<RootCertStore>,
  connections: Arc<Vec<RwLock<HashMap<String, SendRequest<BoxBody<Bytes, hyper::Error>>>>>>,
}

#[async_trait]
impl ServerModuleHandlers for ReverseProxyModuleHandlers {
  async fn request_handler(
    &mut self,
    request: RequestData,
    config: &ServerConfigRoot,
    socket_data: &SocketData,
    error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    WithRuntime::new(self.handle.clone(), async move {
      let mut proxy_to = None;

      if socket_data.encrypted {
        if let Some(secure_proxy_to) = config.get("secureProxyTo").as_str() {
          proxy_to = Some(secure_proxy_to.to_string());
        }
      }

      if proxy_to.is_none() {
        let proxy_to_yaml = config.get("proxyTo");
        proxy_to = proxy_to_yaml.as_str().map(|s| s.to_string());
      }

      if let Some(proxy_to) = proxy_to {
        let (hyper_request, _auth_user) = request.into_parts();
        let (mut hyper_request_parts, request_body) = hyper_request.into_parts();

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

        let addr = format!("{}:{}", host, port);
        let authority = proxy_request_url.authority().cloned();

        let hyper_request_path = hyper_request_parts.uri.path();

        let path = match hyper_request_path.as_bytes().first() {
          Some(b'/') => {
            let mut proxy_request_path = proxy_request_url.path();
            while proxy_request_path.as_bytes().last().copied() == Some(b'/') {
              proxy_request_path = &proxy_request_path[..(proxy_request_path.len() - 1)];
            }
            format!("{}{}", proxy_request_path, hyper_request_path)
          }
          _ => hyper_request_path.to_string(),
        };

        hyper_request_parts.uri = Uri::from_str(&format!(
          "{}{}",
          path,
          match hyper_request_parts.uri.query() {
            Some(query) => format!("?{}", query),
            None => "".to_string(),
          }
        ))?;

        // Host header for host identification
        match authority {
          Some(authority) => {
            hyper_request_parts
              .headers
              .insert(header::HOST, authority.to_string().parse()?);
          }
          None => {
            hyper_request_parts.headers.remove(header::HOST);
          }
        }

        // Connection header to enable HTTP/1.1 keep-alive
        hyper_request_parts
          .headers
          .insert(header::CONNECTION, "keep-alive".parse()?);

        // X-Forwarded-For header to send the client's IP to a server that's behind the reverse proxy
        hyper_request_parts.headers.insert(
          "x-forwarded-for",
          socket_data
            .remote_addr
            .ip()
            .to_canonical()
            .to_string()
            .parse()?,
        );

        let proxy_request = Request::from_parts(hyper_request_parts, request_body.boxed());

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
                let result = http_proxy_kept_alive(sender, proxy_request, error_logger).await;
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
          http_proxy(connections, addr, stream, proxy_request, error_logger).await
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

          http_proxy(connections, addr, tls_stream, proxy_request, error_logger).await
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
}

async fn http_proxy(
  connections: &RwLock<HashMap<String, SendRequest<BoxBody<Bytes, hyper::Error>>>>,
  connect_addr: String,
  stream: impl AsyncRead + AsyncWrite + Send + Unpin + 'static,
  proxy_request: Request<BoxBody<Bytes, hyper::Error>>,
  error_logger: &ErrorLogger,
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

        response = ResponseData::builder_without_request()
                  .response(proxy_response.map(|b| {
                    b.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
                      .boxed()
                  }))
                  .parallel_fn(async move {
                    pinned_conn.await.unwrap_or_default();
                  })
                  .build();

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

async fn http_proxy_kept_alive(
  sender: &mut SendRequest<BoxBody<Bytes, hyper::Error>>,
  proxy_request: Request<BoxBody<Bytes, hyper::Error>>,
  error_logger: &ErrorLogger,
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

  let response = ResponseData::builder_without_request()
    .response(proxy_response.map(|b| {
      b.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
        .boxed()
    }))
    .build();

  Ok(response)
}
