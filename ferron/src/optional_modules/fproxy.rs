use std::error::Error;
use std::str::FromStr;

use crate::ferron_common::{
  ErrorLogger, HyperUpgraded, RequestData, ResponseData, ServerConfig, ServerModule,
  ServerModuleHandlers, SocketData,
};
use crate::ferron_common::{HyperResponse, WithRuntime};
use async_trait::async_trait;
use http_body_util::combinators::BoxBody;
use http_body_util::BodyExt;
use hyper::body::Bytes;
use hyper::{header, Request, StatusCode, Uri};
use hyper_tungstenite::HyperWebsocket;
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::runtime::Handle;

pub fn server_module_init(
  _config: &ServerConfig,
) -> Result<Box<dyn ServerModule + Send + Sync>, Box<dyn Error + Send + Sync>> {
  Ok(Box::new(ForwardProxyModule::new()))
}

struct ForwardProxyModule;

impl ForwardProxyModule {
  fn new() -> Self {
    ForwardProxyModule
  }
}

impl ServerModule for ForwardProxyModule {
  fn get_handlers(&self, handle: Handle) -> Box<dyn ServerModuleHandlers + Send> {
    Box::new(ForwardProxyModuleHandlers { handle })
  }
}

struct ForwardProxyModuleHandlers {
  handle: Handle,
}

#[async_trait]
impl ServerModuleHandlers for ForwardProxyModuleHandlers {
  async fn request_handler(
    &mut self,
    request: RequestData,
    _config: &ServerConfig,
    _socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    Ok(ResponseData::builder(request).build())
  }

  async fn proxy_request_handler(
    &mut self,
    request: RequestData,
    _config: &ServerConfig,
    _socket_data: &SocketData,
    error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    WithRuntime::new(self.handle.clone(), async move {
      // Code taken from reverse proxy module
      let (hyper_request, _auth_user, _original_url) = request.into_parts();
      let (mut hyper_request_parts, request_body) = hyper_request.into_parts();

      match hyper_request_parts.uri.scheme_str() {
        Some("http") | None => (),
        _ => {
          return Ok(
            ResponseData::builder_without_request()
              .status(StatusCode::BAD_REQUEST)
              .build(),
          );
        }
      };

      let host = match hyper_request_parts.uri.host() {
        Some(host) => host,
        None => {
          return Ok(
            ResponseData::builder_without_request()
              .status(StatusCode::BAD_REQUEST)
              .build(),
          );
        }
      };

      let port = hyper_request_parts.uri.port_u16().unwrap_or(80);

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

      let hyper_request_path = hyper_request_parts.uri.path();

      hyper_request_parts.uri = Uri::from_str(&format!(
        "{}{}",
        hyper_request_path,
        match hyper_request_parts.uri.query() {
          Some(query) => format!("?{}", query),
          None => "".to_string(),
        }
      ))?;

      // Connection header to disable HTTP/1.1 keep-alive
      hyper_request_parts
        .headers
        .insert(header::CONNECTION, "close".parse()?);

      let proxy_request = Request::from_parts(hyper_request_parts, request_body);

      http_proxy(stream, proxy_request, error_logger).await
    })
    .await
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
    upgraded_request: HyperUpgraded,
    connect_address: &str,
    _config: &ServerConfig,
    _socket_data: &SocketData,
    error_logger: &ErrorLogger,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    WithRuntime::new(self.handle.clone(), async move {
      let mut stream = match TcpStream::connect(connect_address).await {
        Ok(stream) => stream,
        Err(err) => {
          error_logger
            .log(&format!("Cannot connect to the remote server: {}", err))
            .await;
          return Ok(());
        }
      };
      match stream.set_nodelay(true) {
        Ok(_) => (),
        Err(err) => {
          error_logger
            .log(&format!(
              "Cannot disable Nagle algorithm when connecting to the remote server: {}",
              err
            ))
            .await;
          return Ok(());
        }
      };

      let mut upgraded = TokioIo::new(upgraded_request);

      tokio::io::copy_bidirectional(&mut upgraded, &mut stream)
        .await
        .unwrap_or_default();

      Ok(())
    })
    .await
  }

  fn does_connect_proxy_requests(&mut self) -> bool {
    true
  }

  async fn websocket_request_handler(
    &mut self,
    _websocket: HyperWebsocket,
    _uri: &hyper::Uri,
    _config: &ServerConfig,
    _socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    Ok(())
  }

  fn does_websocket_requests(&mut self, _config: &ServerConfig, _socket_data: &SocketData) -> bool {
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
