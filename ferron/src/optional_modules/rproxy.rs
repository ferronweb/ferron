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
use futures_util::{SinkExt, StreamExt};
use http::uri::{PathAndQuery, Scheme};
use http_body_util::combinators::BoxBody;
use http_body_util::BodyExt;
use hyper::body::Bytes;
use hyper::client::conn::http1::SendRequest;
use hyper::{header, Request, StatusCode, Uri};
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
use tokio_tungstenite::Connector;

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

#[allow(clippy::type_complexity)]
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

      // When the array is supplied with non-string values, the reverse proxy may have undesirable behavior
      // The "proxyTo" and "secureProxyTo" are validated though.

      if socket_data.encrypted {
        let secure_proxy_to_yaml = config.get("secureProxyTo");
        if let Some(secure_proxy_to_vector) = secure_proxy_to_yaml.as_vec() {
          if !secure_proxy_to_vector.is_empty() {
            if let Some(secure_proxy_to) =
              secure_proxy_to_vector[rand::random_range(..secure_proxy_to_vector.len())].as_str()
            {
              proxy_to = Some(secure_proxy_to.to_string());
            }
          }
        } else if let Some(secure_proxy_to) = secure_proxy_to_yaml.as_str() {
          proxy_to = Some(secure_proxy_to.to_string());
        }
      }

      if proxy_to.is_none() {
        let proxy_to_yaml = config.get("proxyTo");
        if let Some(proxy_to_vector) = proxy_to_yaml.as_vec() {
          if !proxy_to_vector.is_empty() {
            if let Some(proxy_to_str) =
              proxy_to_vector[rand::random_range(..proxy_to_vector.len())].as_str()
            {
              proxy_to = Some(proxy_to_str.to_string());
            }
          }
        } else if let Some(proxy_to_str) = proxy_to_yaml.as_str() {
          proxy_to = Some(proxy_to_str.to_string());
        }
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

        let original_host = hyper_request_parts.headers.get(header::HOST).cloned();

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

        // X-Forwarded-* headers to send the client's data to a server that's behind the reverse proxy
        hyper_request_parts.headers.insert(
          "x-forwarded-for",
          socket_data
            .remote_addr
            .ip()
            .to_canonical()
            .to_string()
            .parse()?,
        );

        if socket_data.encrypted {
          hyper_request_parts
            .headers
            .insert("x-forwarded-proto", "https".parse()?);
        } else {
          hyper_request_parts
            .headers
            .insert("x-forwarded-proto", "http".parse()?);
        }

        if let Some(original_host) = original_host {
          hyper_request_parts
            .headers
            .insert("x-forwarded-host", original_host);
        }

        let proxy_request = Request::from_parts(hyper_request_parts, request_body);

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

  async fn websocket_request_handler(
    &mut self,
    websocket: HyperWebsocket,
    uri: &hyper::Uri,
    config: &ServerConfigRoot,
    socket_data: &SocketData,
    error_logger: &ErrorLogger,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    WithRuntime::new(self.handle.clone(), async move {
      let mut proxy_to = None;

      // When the array is supplied with non-string values, the reverse proxy may have undesirable behavior
      // The "proxyTo" and "secureProxyTo" are validated though.

      if socket_data.encrypted {
        let secure_proxy_to_yaml = config.get("secureProxyTo");
        if let Some(secure_proxy_to_vector) = secure_proxy_to_yaml.as_vec() {
          if !secure_proxy_to_vector.is_empty() {
            if let Some(secure_proxy_to) =
              secure_proxy_to_vector[rand::random_range(..secure_proxy_to_vector.len())].as_str()
            {
              proxy_to = Some(secure_proxy_to.to_string());
            }
          }
        } else if let Some(secure_proxy_to) = secure_proxy_to_yaml.as_str() {
          proxy_to = Some(secure_proxy_to.to_string());
        }
      }

      if proxy_to.is_none() {
        let proxy_to_yaml = config.get("proxyTo");
        if let Some(proxy_to_vector) = proxy_to_yaml.as_vec() {
          if !proxy_to_vector.is_empty() {
            if let Some(proxy_to_str) =
              proxy_to_vector[rand::random_range(..proxy_to_vector.len())].as_str()
            {
              proxy_to = Some(proxy_to_str.to_string());
            }
          }
        } else if let Some(proxy_to_str) = proxy_to_yaml.as_str() {
          proxy_to = Some(proxy_to_str.to_string());
        }
      }

      if let Some(proxy_to) = proxy_to {
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

        let request_path = uri.path();

        let path = match request_path.as_bytes().first() {
          Some(b'/') => {
            let mut proxy_request_path = proxy_request_url.path();
            while proxy_request_path.as_bytes().last().copied() == Some(b'/') {
              proxy_request_path = &proxy_request_path[..(proxy_request_path.len() - 1)];
            }
            format!("{}{}", proxy_request_path, request_path)
          }
          _ => request_path.to_string(),
        };

        let mut proxy_request_url_parts = proxy_request_url.into_parts();
        proxy_request_url_parts.scheme = if encrypted {
          Some(Scheme::from_str("wss")?)
        } else {
          Some(Scheme::from_str("ws")?)
        };
        match proxy_request_url_parts.path_and_query {
          Some(path_and_query) => {
            let path_and_query_string = match path_and_query.query() {
              Some(query) => {
                format!("{}?{}", path, query)
              }
              None => path,
            };
            proxy_request_url_parts.path_and_query =
              Some(PathAndQuery::from_str(&path_and_query_string)?);
          }
          None => {
            proxy_request_url_parts.path_and_query = Some(PathAndQuery::from_str(&path)?);
          }
        };

        let proxy_request_url = hyper::Uri::from_parts(proxy_request_url_parts)?;

        let connector = if !encrypted {
          Connector::Plain
        } else {
          Connector::Rustls(Arc::new(
            rustls::ClientConfig::builder()
              .with_root_certificates(self.roots.clone())
              .with_no_client_auth(),
          ))
        };

        let client_bi_stream = websocket.await?;

        let (proxy_bi_stream, _) = match tokio_tungstenite::connect_async_tls_with_config(
          proxy_request_url,
          None,
          true,
          Some(connector),
        )
        .await
        {
          Ok(data) => data,
          Err(err) => {
            error_logger
              .log(&format!("Cannot connect to WebSocket server: {}", err))
              .await;
            return Ok(());
          }
        };

        let (mut client_sink, mut client_stream) = client_bi_stream.split();
        let (mut proxy_sink, mut proxy_stream) = proxy_bi_stream.split();

        let client_to_proxy = async {
          while let Some(Ok(value)) = client_stream.next().await {
            if let Err(_) = proxy_sink.send(value).await {
              break;
            }
          }
        };

        let proxy_to_client = async {
          while let Some(Ok(value)) = proxy_stream.next().await {
            if let Err(_) = client_sink.send(value).await {
              break;
            }
          }
        };

        tokio::pin!(client_to_proxy);
        tokio::pin!(proxy_to_client);

        let client_to_proxy_first;
        tokio::select! {
          _ = &mut client_to_proxy => {
            client_to_proxy_first = true;
          }
          _ = &mut proxy_to_client => {
            client_to_proxy_first = false;
          }
        }

        if client_to_proxy_first {
          proxy_to_client.await;
        } else {
          client_to_proxy.await;
        }
      }

      Ok(())
    })
    .await
  }

  fn does_websocket_requests(
    &mut self,
    config: &ServerConfigRoot,
    socket_data: &SocketData,
  ) -> bool {
    if socket_data.encrypted {
      let secure_proxy_to = config.get("secureProxyTo");
      if secure_proxy_to.as_vec().is_some() || secure_proxy_to.as_str().is_some() {
        return true;
      }
    }

    let proxy_to = config.get("proxyTo");
    proxy_to.as_vec().is_some() || proxy_to.as_str().is_some()
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
