use std::error::Error;
use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use http_body_util::combinators::BoxBody;
use http_body_util::BodyExt;
use hyper::body::Bytes;
use hyper::{header, Request, StatusCode, Uri};
use hyper_util::rt::TokioIo;
use mimalloc::MiMalloc;
use project_karpacz_common::{
  ErrorLogger, HyperUpgraded, RequestData, ResponseData, ServerConfig, ServerConfigRoot,
  ServerModule, ServerModuleHandlers, SocketData,
};
use project_karpacz_common::{HyperResponse, WithRuntime};
use rustls::crypto::aws_lc_rs::default_provider;
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, RootCertStore};
use rustls_native_certs::load_native_certs;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::runtime::Handle;
use tokio_rustls::TlsConnector;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[no_mangle]
pub fn server_module_validate_config(
  config: &ServerConfigRoot,
  _is_global: bool,
) -> Result<(), Box<dyn Error + Send + Sync>> {
  if !config.get("proxyTo").is_badvalue() && config.get("proxyTo").as_str().is_none() {
    Err(anyhow::anyhow!("Invalid reverse proxy target URL value"))?
  }

  if !config.get("secureProxyTo").is_badvalue() && config.get("secureProxyTo").as_str().is_none() {
    Err(anyhow::anyhow!(
      "Invalid secure reverse proxy target URL value"
    ))?
  }

  Ok(())
}

#[no_mangle]
pub fn server_module_init(
  _config: &ServerConfig,
) -> Result<Box<dyn ServerModule + Send + Sync>, Box<dyn Error + Send + Sync>> {
  if default_provider().install_default().is_err() {
    Err(anyhow::anyhow!("Cannot install crypto provider"))?
  }

  let mut roots = RootCertStore::empty();
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

  let tls_client_config = rustls::ClientConfig::builder()
    .with_root_certificates(roots)
    .with_no_client_auth();

  Ok(Box::new(ReverseProxyModule::new(Arc::new(
    tls_client_config,
  ))))
}

struct ReverseProxyModule {
  tls_client_config: Arc<ClientConfig>,
}

impl ReverseProxyModule {
  fn new(tls_client_config: Arc<ClientConfig>) -> Self {
    ReverseProxyModule { tls_client_config }
  }
}

impl ServerModule for ReverseProxyModule {
  fn get_handlers(&self, handle: Handle) -> Box<dyn ServerModuleHandlers + Send> {
    Box::new(ReverseProxyModuleHandlers {
      tls_client_config: self.tls_client_config.clone(),
      handle,
    })
  }
}

struct ReverseProxyModuleHandlers {
  handle: Handle,
  tls_client_config: Arc<ClientConfig>,
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
        let stream = match TcpStream::connect(addr).await {
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

        // Connection header to disable HTTP/1.1 keep-alive
        hyper_request_parts
          .headers
          .insert(header::CONNECTION, "close".parse()?);

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

        if !encrypted {
          http_proxy(stream, proxy_request, error_logger).await
        } else {
          let connector = TlsConnector::from(self.tls_client_config.clone());
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

          http_proxy(tls_stream, proxy_request, error_logger).await
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

  Ok(response)
}
