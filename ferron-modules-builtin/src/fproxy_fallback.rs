use std::error::Error;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use hyper::{Request, StatusCode};

use ferron_common::logging::ErrorLogger;
use ferron_common::{config::ServerConfiguration, util::ModuleCache};

use ferron_common::modules::{Module, ModuleHandlers, ModuleLoader, ResponseData, SocketData};

/// A forward proxy fallback module loader
pub struct ForwardProxyFallbackModuleLoader {
  cache: ModuleCache<ForwardProxyFallbackModule>,
}

impl Default for ForwardProxyFallbackModuleLoader {
  fn default() -> Self {
    Self::new()
  }
}

impl ForwardProxyFallbackModuleLoader {
  /// Creates a new module loader
  pub fn new() -> Self {
    Self {
      cache: ModuleCache::new(vec![]),
    }
  }
}

impl ModuleLoader for ForwardProxyFallbackModuleLoader {
  fn load_module(
    &mut self,
    config: &ServerConfiguration,
    _global_config: Option<&ServerConfiguration>,
    _secondary_runtime: &tokio::runtime::Runtime,
  ) -> Result<Arc<dyn Module + Send + Sync>, Box<dyn Error + Send + Sync>> {
    Ok(
      self
        .cache
        .get_or_init::<_, Box<dyn std::error::Error + Send + Sync>>(config, move |_| {
          Ok(Arc::new(ForwardProxyFallbackModule))
        })?,
    )
  }
}

/// A forward proxy fallback module
struct ForwardProxyFallbackModule;

impl Module for ForwardProxyFallbackModule {
  fn get_module_handlers(&self) -> Box<dyn ModuleHandlers> {
    Box::new(ForwardProxyFallbackModuleHandlers)
  }
}

/// Handlers for the forward proxy fallback module
struct ForwardProxyFallbackModuleHandlers;

#[async_trait(?Send)]
impl ModuleHandlers for ForwardProxyFallbackModuleHandlers {
  async fn request_handler(
    &mut self,
    request: Request<BoxBody<Bytes, std::io::Error>>,
    _config: &ServerConfiguration,
    _socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    // Determine if the request is a forward proxy request
    let is_proxy_request = match request.version() {
      hyper::Version::HTTP_2 | hyper::Version::HTTP_3 => {
        request.method() == hyper::Method::CONNECT && request.uri().host().is_some()
      }
      _ => request.uri().host().is_some(),
    };

    Ok(ResponseData {
      request: Some(request),
      response: None,
      response_status: if is_proxy_request {
        Some(StatusCode::NOT_IMPLEMENTED)
      } else {
        None
      },
      response_headers: None,
      new_remote_address: None,
    })
  }
}
