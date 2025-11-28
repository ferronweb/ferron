use std::collections::HashSet;
use std::error::Error;
use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use hyper::{HeaderMap, Request, Response, StatusCode, Uri};

use crate::config::ServerConfiguration;
use crate::logging::ErrorLogger;
use crate::observability::MetricsMultiSender;

/// A trait that defines a module loader
pub trait ModuleLoader {
  /// Loads a module according to specific configuration
  fn load_module(
    &mut self,
    config: &ServerConfiguration,
    global_config: Option<&ServerConfiguration>,
    secondary_runtime: &tokio::runtime::Runtime,
  ) -> Result<Arc<dyn Module + Send + Sync>, Box<dyn Error + Send + Sync>>;

  /// Determines configuration properties required to load a module
  fn get_requirements(&self) -> Vec<&'static str> {
    vec![]
  }

  /// Validates the server configuration
  #[allow(unused_variables)]
  fn validate_configuration(
    &self,
    config: &ServerConfiguration,
    used_properties: &mut HashSet<String>,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    Ok(())
  }
}

/// A trait that defines a module
pub trait Module {
  /// Obtains the module handlers
  fn get_module_handlers(&self) -> Box<dyn ModuleHandlers>;
}

/// A trait that defines handlers for a module
#[async_trait(?Send)]
pub trait ModuleHandlers {
  /// Handles the incoming request
  async fn request_handler(
    &mut self,
    request: Request<BoxBody<Bytes, std::io::Error>>,
    config: &ServerConfiguration,
    socket_data: &SocketData,
    error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>>;

  /// Modifies the outgoing response
  async fn response_modifying_handler(
    &mut self,
    response: Response<BoxBody<Bytes, std::io::Error>>,
  ) -> Result<Response<BoxBody<Bytes, std::io::Error>>, Box<dyn Error>> {
    Ok(response)
  }

  /// Sends metric data before handling the request
  #[allow(unused_variables)]
  async fn metric_data_before_handler(
    &mut self,
    request: &Request<BoxBody<Bytes, std::io::Error>>,
    socket_data: &SocketData,
    metrics_sender: &MetricsMultiSender,
  ) {
  }

  /// Sends metric data after modifying the response
  #[allow(unused_variables)]
  async fn metric_data_after_handler(&mut self, metrics_sender: &MetricsMultiSender) {}
}

/// Contains information about a network socket, including remote and local addresses,
/// and whether the connection is encrypted.
pub struct SocketData {
  /// The remote address of the socket.
  pub remote_addr: SocketAddr,

  /// The local address of the socket.
  pub local_addr: SocketAddr,

  /// Indicates if the connection is encrypted.
  pub encrypted: bool,
}

/// Data related to an HTTP request
#[derive(Clone)]
pub struct RequestData {
  /// The authenticated username
  pub auth_user: Option<String>,

  /// The original URL (before URL rewriting)
  pub original_url: Option<Uri>,

  /// The error status code, when the error handler is executed
  #[allow(dead_code)]
  pub error_status_code: Option<StatusCode>,
}

/// Data related to an HTTP response
pub struct ResponseData {
  /// The passed HTTP request
  pub request: Option<Request<BoxBody<Bytes, std::io::Error>>>,

  /// The HTTP response with a body
  pub response: Option<Response<BoxBody<Bytes, std::io::Error>>>,

  /// The HTTP response status code (when the response with a body isn't set)
  pub response_status: Option<StatusCode>,

  /// The HTTP response headers
  pub response_headers: Option<HeaderMap>,

  /// The new client address
  pub new_remote_address: Option<SocketAddr>,
}
