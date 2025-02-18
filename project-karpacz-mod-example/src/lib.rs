use std::error::Error;

use async_trait::async_trait;
use http_body_util::{BodyExt, Full};
use hyper::Response;
use mimalloc::MiMalloc;
use project_karpacz_common::{
  ErrorLogger, HyperUpgraded, RequestData, ResponseData, ServerConfig, ServerConfigRoot,
  ServerModule, ServerModuleHandlers, SocketData,
};
use project_karpacz_common::{HyperResponse, WithRuntime};
use tokio::runtime::Handle;

// It's very important to not remove these two lines below, otherwise a HTTP request will trigger a segmentation fault!
#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

// Define a struct for the module implementation
struct ExampleModule;

/// Validates the server configuration.
/// Since this module has no configurable properties, it always returns Ok(()).
#[no_mangle]
pub fn server_module_validate_config(
  _config: &ServerConfigRoot, // This is a configuration root created from YAML configuration
  _is_global: bool,
) -> Result<(), Box<dyn Error + Send + Sync>> {
  Ok(())
}

/// Initializes the server module and returns an instance of `ExampleModule`.
#[no_mangle]
pub fn server_module_init(
  _config: &ServerConfig, // This is YAML configuration parsed as-is. If used, you would have to clone it, otherwise every configuration property would be a BadValue.
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
    _config: &ServerConfigRoot,
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
    _config: &ServerConfigRoot,
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
    _config: &ServerConfigRoot,
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
}
