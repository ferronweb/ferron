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

// Define a struct for the module implementation
struct ExampleModule;

/// Initializes the server module and returns an instance of `ExampleModule`.
pub fn server_module_init(
  _config: &ServerConfig, // This is YAML configuration parsed as-is.
) -> Result<Box<dyn ServerModule + Send + Sync>, Box<dyn Error + Send + Sync>> {
  Ok(Box::new(ExampleModule::new()))
}

impl ExampleModule {
  /// Creates a new instance of `ExampleModule`.
  fn new() -> Self {
    ExampleModule
  }
}

/// Implements the `ServerModule` trait for `ExampleModule`.
impl ServerModule for ExampleModule {
  /// Returns an instance of `ExampleModuleHandlers` to handle HTTP requests.
  fn get_handlers(&self, handle: Handle) -> Box<dyn ServerModuleHandlers + Send> {
    Box::new(ExampleModuleHandlers { handle })
  }
}

// Define a struct to handle HTTP requests
struct ExampleModuleHandlers {
  handle: Handle,
}

/// Implements the `ServerModuleHandlers` trait for `ExampleModuleHandlers`.
#[async_trait]
impl ServerModuleHandlers for ExampleModuleHandlers {
  /// Handles incoming HTTP requests.
  /// If the request path is `/hello`, it responds with "Hello World!".
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

  /// Handles non-CONNECT proxy requests (not used in this module).
  async fn proxy_request_handler(
    &mut self,
    request: RequestData,
    _config: &ServerConfig,
    _socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    // No proxy request handling needed.
    Ok(ResponseData::builder(request).build())
  }

  /// Modifies outgoing responses (not used in this module).
  async fn response_modifying_handler(
    &mut self,
    response: HyperResponse,
  ) -> Result<HyperResponse, Box<dyn Error + Send + Sync>> {
    // No response modification needed.
    Ok(response)
  }

  /// Modifies outgoing proxy responses (not used in this module).
  async fn proxy_response_modifying_handler(
    &mut self,
    response: HyperResponse,
  ) -> Result<HyperResponse, Box<dyn Error + Send + Sync>> {
    // No proxy response modification needed.
    Ok(response)
  }

  /// Handles CONNECT proxy requests (not used in this module).
  async fn connect_proxy_request_handler(
    &mut self,
    _upgraded_request: HyperUpgraded,
    _connect_address: &str,
    _config: &ServerConfig,
    _socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    // No proxy request handling needed.
    Ok(())
  }

  /// Checks if the module is a forward proxy module utilizing CONNECT method.
  fn does_connect_proxy_requests(&mut self) -> bool {
    // This is not a forward proxy module utilizing CONNECT method
    false
  }

  /// Handles WebSocket requests (not used in this module).
  async fn websocket_request_handler(
    &mut self,
    _websocket: HyperWebsocket,
    _uri: &hyper::Uri,
    _headers: &hyper::HeaderMap,
    _config: &ServerConfig,
    _socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    // No proxy request handling needed.
    Ok(())
  }

  /// Checks if the module supports WebSocket connections.
  fn does_websocket_requests(&mut self, _config: &ServerConfig, _socket_data: &SocketData) -> bool {
    // This module doesn't support WebSocket connections.
    false
  }
}
