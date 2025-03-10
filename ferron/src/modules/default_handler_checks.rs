use std::error::Error;

use async_trait::async_trait;
use ferron_common::{
  ErrorLogger, HyperResponse, RequestData, ResponseData, ServerConfigRoot, ServerModule,
  ServerModuleHandlers, SocketData,
};
use ferron_common::{HyperUpgraded, WithRuntime};
use http_body_util::{BodyExt, Empty};
use hyper::header::HeaderValue;
use hyper::{header, HeaderMap, Method, Response, StatusCode};
use hyper_tungstenite::HyperWebsocket;
use tokio::runtime::Handle;

struct DefaultHandlerChecksModule;

pub fn server_module_init(
) -> Result<Box<dyn ServerModule + Send + Sync>, Box<dyn Error + Send + Sync>> {
  Ok(Box::new(DefaultHandlerChecksModule::new()))
}

impl DefaultHandlerChecksModule {
  fn new() -> Self {
    DefaultHandlerChecksModule
  }
}

impl ServerModule for DefaultHandlerChecksModule {
  fn get_handlers(&self, handle: Handle) -> Box<dyn ServerModuleHandlers + Send> {
    Box::new(DefaultHandlerChecksModuleHandlers { handle })
  }
}
struct DefaultHandlerChecksModuleHandlers {
  handle: Handle,
}

#[async_trait]
impl ServerModuleHandlers for DefaultHandlerChecksModuleHandlers {
  async fn request_handler(
    &mut self,
    request: RequestData,
    _config: &ServerConfigRoot,
    _socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    WithRuntime::new(self.handle.clone(), async move {
      match request.get_hyper_request().method() {
        &Method::OPTIONS => Ok(
          ResponseData::builder(request)
            .response(
              Response::builder()
                .status(StatusCode::NO_CONTENT)
                .header(header::ALLOW, "GET, POST, HEAD, OPTIONS")
                .body(Empty::new().map_err(|e| match e {}).boxed())
                .unwrap_or_default(),
            )
            .build(),
        ),
        &Method::GET | &Method::POST | &Method::HEAD => Ok(ResponseData::builder(request).build()),
        _ => {
          let mut header_map = HeaderMap::new();
          if let Ok(header_value) = HeaderValue::from_str("GET, POST, HEAD, OPTIONS") {
            header_map.insert(header::ALLOW, header_value);
          };
          Ok(
            ResponseData::builder(request)
              .status(StatusCode::METHOD_NOT_ALLOWED)
              .headers(header_map)
              .build(),
          )
        }
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
    Ok(
      ResponseData::builder(request)
        .status(StatusCode::NOT_IMPLEMENTED)
        .build(),
    )
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

  fn does_websocket_requests(
    &mut self,
    _config: &ServerConfigRoot,
    _socket_data: &SocketData,
  ) -> bool {
    false
  }
}
