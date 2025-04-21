#[cfg(not(unix))]
compile_error!("This module is supported only on Unix and Unix-like systems.");

use std::error::Error;

use crate::ferron_common::{
  ErrorLogger, HyperUpgraded, RequestData, ResponseData, ServerConfig, ServerModule,
  ServerModuleHandlers, SocketData,
};
use crate::ferron_common::{HyperResponse, WithRuntime};
use async_trait::async_trait;
use http_body_util::{BodyExt, Full};
use hyper::Response;
use hyper_tungstenite::HyperWebsocket;
use tokio::runtime::Handle;

pub fn server_module_init(
  _config: &ServerConfig,
) -> Result<Box<dyn ServerModule + Send + Sync>, Box<dyn Error + Send + Sync>> {
  Err(anyhow::anyhow!("This module is just a stub."))?;
  Ok(Box::new(WsgidModule::new()))
}

struct WsgidModule;

impl WsgidModule {
  fn new() -> Self {
    WsgidModule
  }
}

impl ServerModule for WsgidModule {
  fn get_handlers(&self, handle: Handle) -> Box<dyn ServerModuleHandlers + Send> {
    Box::new(WsgidModuleHandlers { handle })
  }
}

struct WsgidModuleHandlers {
  handle: Handle,
}

#[async_trait]
impl ServerModuleHandlers for WsgidModuleHandlers {
  async fn request_handler(
    &mut self,
    request: RequestData,
    _config: &ServerConfig,
    _socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    WithRuntime::new(self.handle.clone(), async move {
      if request.get_hyper_request().uri().path() == "/hello" {
        Ok(
          ResponseData::builder(request)
            .response(
              Response::builder().body(
                Full::new("Hello World!".into())
                  .map_err(|e| match e {})
                  .boxed(),
              )?,
            )
            .build(),
        )
      } else {
        Ok(ResponseData::builder(request).build())
      }
    })
    .await
  }

  async fn proxy_request_handler(
    &mut self,
    request: RequestData,
    _config: &ServerConfig,
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
    _config: &ServerConfig,
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
