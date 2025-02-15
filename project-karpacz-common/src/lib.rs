use std::{error::Error, net::SocketAddr};

use async_trait::async_trait;
use http_body_util::combinators::BoxBody;
use hyper::{
  body::{Bytes, Incoming},
  HeaderMap, Request, Response, StatusCode,
};
use tokio::{runtime::Handle, sync::mpsc::Sender};
use yaml_rust2::Yaml;

mod log;
mod with_runtime;

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

impl SocketData {
  /// Creates a new `SocketData` instance.
  ///
  /// # Parameters
  ///
  /// - `remote_addr`: The remote address of the socket.
  /// - `local_addr`: The local address of the socket.
  /// - `encrypted`: A boolean indicating if the connection is encrypted.
  ///
  /// # Returns
  ///
  /// A new `SocketData` instance with the provided parameters.
  pub fn new(remote_addr: SocketAddr, local_addr: SocketAddr, encrypted: bool) -> Self {
    SocketData {
      remote_addr,
      local_addr,
      encrypted,
    }
  }
}

/// Represents a log message. This is a type alias for `crate::log::LogMessage`.
pub type LogMessage = crate::log::LogMessage;

/// Represents the server configuration. This is a type alias for `Yaml` from the `yaml_rust2` crate.
pub type ServerConfig = Yaml;

/// Represents the HTTP request from Hyper.
pub type HyperRequest = Request<Incoming>;

/// Represents the HTTP response from Hyper.
pub type HyperResponse = Response<BoxBody<Bytes, std::io::Error>>;

/// A wrapper that ensures a function is executed within a specific runtime context.
/// This is a type alias for `crate::with_runtime::WithRuntime<F>`.
pub type WithRuntime<F> = crate::with_runtime::WithRuntime<F>;

/// Contains data related to an HTTP request, including the original Hyper request
/// and optional authentication user information.
pub struct RequestData {
  hyper_request: Request<Incoming>,
  auth_user: Option<String>,
}

impl RequestData {
  /// Creates a new `RequestData` instance.
  ///
  /// # Parameters
  ///
  /// - `hyper_request`: The original Hyper `Request` object.
  /// - `auth_user`: An optional string representing the authenticated user.
  ///
  /// # Returns
  ///
  /// A new `RequestData` instance with the provided parameters.
  pub fn new(hyper_request: Request<Incoming>, auth_user: Option<String>) -> Self {
    RequestData {
      hyper_request,
      auth_user,
    }
  }

  /// Sets the authenticated user for the request.
  ///
  /// # Parameters
  ///
  /// - `auth_user`: A string representing the authenticated user.
  pub fn set_auth_user(&mut self, auth_user: String) {
    self.auth_user = Some(auth_user);
  }

  /// Retrieves the authenticated user associated with the request, if any.
  ///
  /// # Returns
  ///
  /// An `Option` containing a reference to the authenticated user's string, or `None` if not set.
  pub fn get_auth_user(&self) -> Option<&str> {
    match &self.auth_user {
      Some(auth_user) => Some(auth_user),
      None => None,
    }
  }

  /// Provides a reference to the underlying Hyper `Request` object.
  ///
  /// # Returns
  ///
  /// A reference to the Hyper `Request<Incoming>` object.
  pub fn get_hyper_request(&self) -> &Request<Incoming> {
    &self.hyper_request
  }

  /// Provides a mutable reference to the underlying Hyper `Request` object.
  ///
  /// # Returns
  ///
  /// A mutable reference to the Hyper `Request<Incoming>` object.
  pub fn get_mut_hyper_request(&mut self) -> &mut Request<Incoming> {
    &mut self.hyper_request
  }

  /// Consumes the `RequestData` instance and returns its components.
  ///
  /// # Returns
  ///
  /// A tuple containing the Hyper `Request<Incoming>` object and an optional authenticated user string.
  pub fn into_parts(self) -> (Request<Incoming>, Option<String>) {
    (self.hyper_request, self.auth_user)
  }
}

/// Facilitates logging of error messages through a provided logger sender.
pub struct ErrorLogger<'a> {
  logger: Option<&'a Sender<LogMessage>>,
}

impl<'a> ErrorLogger<'a> {
  /// Creates a new `ErrorLogger` instance.
  ///
  /// # Parameters
  ///
  /// - `logger`: A reference to a `Sender<LogMessage>` used for sending log messages.
  ///
  /// # Returns
  ///
  /// A new `ErrorLogger` instance associated with the provided logger.
  pub fn new(logger: &'a Sender<LogMessage>) -> Self {
    ErrorLogger {
      logger: Some(logger),
    }
  }

  /// Creates a new `ErrorLogger` instance without any underlying logger.
  ///
  /// # Returns
  ///
  /// A new `ErrorLogger` instance not associated with any logger.
  pub fn without_logger() -> Self {
    ErrorLogger { logger: None }
  }

  /// Logs an error message asynchronously.
  ///
  /// # Parameters
  ///
  /// - `message`: A string slice containing the error message to be logged.
  ///
  /// # Examples
  ///
  /// ```
  /// # use tokio::sync::mpsc::channel;
  /// # use project_karpacz_common::ErrorLogger;
  /// # #[tokio::main]
  /// # async fn main() {
  /// let (tx, mut rx) = channel(100);
  /// let logger = ErrorLogger::new(&tx);
  /// logger.log("An error occurred").await;
  /// # }
  /// ```
  pub async fn log(&self, message: &str) {
    if let Some(logger) = self.logger {
      logger
        .send(LogMessage::new(String::from(message), true))
        .await
        .unwrap_or_default();
    }
  }
}

/// Holds data related to an HTTP response, including the original request,
/// optional authentication user information, and the response details.
pub struct ResponseData {
  request: Option<Request<Incoming>>,
  auth_user: Option<String>,
  response: Option<Response<BoxBody<Bytes, std::io::Error>>>,
  response_status: Option<StatusCode>,
  response_headers: Option<HeaderMap>,
  new_remote_address: Option<SocketAddr>,
}

impl ResponseData {
  /// Initiates the building process for a `ResponseData` instance using a `RequestData` object.
  ///
  /// # Parameters
  ///
  /// - `request`: A `RequestData` instance containing the original request and authentication information.
  ///
  /// # Returns
  ///
  /// A `ResponseDataBuilder` initialized with the provided request data.
  pub fn builder(request: RequestData) -> ResponseDataBuilder {
    let (request, auth_user) = request.into_parts();
        
    ResponseDataBuilder {
      request: Some(request),
      auth_user,
      response: None,
      response_status: None,
      response_headers: None,
      new_remote_address: None,
    }
  }

  /// Initiates the building process for a `ResponseData` instance without a `RequestData` object.
  ///
  /// # Returns
  ///
  /// A `ResponseDataBuilder` initialized without any request data.
  pub fn builder_without_request() -> ResponseDataBuilder {
    ResponseDataBuilder {
      request: None,
      auth_user: None,
      response: None,
      response_status: None,
      response_headers: None,
      new_remote_address: None,
    }
  }

  /// Consumes the `ResponseData` instance and returns its components.
  ///
  /// # Returns
  ///
  /// A tuple containing:
  /// - The optional original Hyper `Request<Incoming>` object.
  /// - An optional authenticated user string.
  /// - An optional `Response` object encapsulated in a `BoxBody` with `Bytes` and `std::io::Error`.
  /// - An optional HTTP `StatusCode`.
  /// - An optional `HeaderMap` containing the HTTP headers.
  /// - An optional `SocketAddr` containing the client's new IP address and port.
  #[allow(clippy::type_complexity)]
  pub fn into_parts(
    self,
  ) -> (
    Option<Request<Incoming>>,
    Option<String>,
    Option<Response<BoxBody<Bytes, std::io::Error>>>,
    Option<StatusCode>,
    Option<HeaderMap>,
    Option<SocketAddr>,
  ) {
    (
      self.request,
      self.auth_user,
      self.response,
      self.response_status,
      self.response_headers,
      self.new_remote_address,
    )
  }
}

pub struct ResponseDataBuilder {
  request: Option<Request<Incoming>>,
  auth_user: Option<String>,
  response: Option<Response<BoxBody<Bytes, std::io::Error>>>,
  response_status: Option<StatusCode>,
  response_headers: Option<HeaderMap>,
  new_remote_address: Option<SocketAddr>,
}

impl ResponseDataBuilder {
  /// Sets the response for the `ResponseData`.
  ///
  /// # Parameters
  ///
  /// - `response`: A `Response` object encapsulated in a `BoxBody` with `Bytes` and `std::io::Error`.
  ///
  /// # Returns
  ///
  /// The updated `ResponseDataBuilder` instance with the specified response.
  pub fn response(mut self, response: Response<BoxBody<Bytes, std::io::Error>>) -> Self {
    self.response = Some(response);
    self
  }

  /// Sets the status code for the `ResponseData`.
  ///
  /// # Parameters
  ///
  /// - `status`: A `StatusCode` representing the HTTP status code.
  ///
  /// # Returns
  ///
  /// The updated `ResponseDataBuilder` instance with the specified status code.
  pub fn status(mut self, status: StatusCode) -> Self {
    self.response_status = Some(status);
    self
  }

  /// Sets the headers for the `ResponseData`.
  ///
  /// # Parameters
  ///
  /// - `headers`: A `HeaderMap` containing the HTTP headers.
  ///
  /// # Returns
  ///
  /// The updated `ResponseDataBuilder` instance with the specified headers.
  pub fn headers(mut self, headers: HeaderMap) -> Self {
    self.response_headers = Some(headers);
    self
  }

  /// Sets the new client address for the `ResponseData`.
  ///
  /// # Parameters
  ///
  /// - `new_remote_address`: A `SocketAddr` containing the new client's IP address and port.
  ///
  /// # Returns
  ///
  /// The updated `ResponseDataBuilder` instance with the specified headers.
  pub fn new_remote_address(mut self, new_remote_address: SocketAddr) -> Self {
    self.new_remote_address = Some(new_remote_address);
    self
  }

  /// Builds the `ResponseData` instance.
  ///
  /// # Returns
  ///
  /// A `ResponseData` object containing the accumulated data from the builder.
  pub fn build(self) -> ResponseData {
    ResponseData {
      request: self.request,
      auth_user: self.auth_user,
      response: self.response,
      response_status: self.response_status,
      response_headers: self.response_headers,
      new_remote_address: self.new_remote_address,
    }
  }
}

/// Defines the interface for server module handlers, specifying how requests should be processed.
#[async_trait]
pub trait ServerModuleHandlers {
  /// Handles an incoming request.
  ///
  /// # Parameters
  ///
  /// - `request`: A `RequestData` object containing the incoming request and associated data.
  /// - `config`: A reference to the combined server configuration (`ServerConfig`). The combined configuration has properties in its root.
  /// - `socket_data`: A reference to the `SocketData` containing socket-related information.
  /// - `error_logger`: A reference to an `ErrorLogger` for logging errors.
  ///
  /// # Returns
  ///
  /// A `Result` containing a `ResponseData` object upon success, or a boxed `dyn Error` if an error occurs.
  async fn request_handler(
    &mut self,
    request: RequestData,
    config: &ServerConfig,
    socket_data: &SocketData,
    error_logger: &ErrorLogger<'_>,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>>;

  /// Handles an incoming forward proxy request.
  ///
  /// # Parameters
  ///
  /// - `request`: A `RequestData` object containing the incoming request and associated data.
  /// - `config`: A reference to the combined server configuration (`ServerConfig`). The combined configuration has properties in its root.
  /// - `socket_data`: A reference to the `SocketData` containing socket-related information.
  /// - `error_logger`: A reference to an `ErrorLogger` for logging errors.
  ///
  /// # Returns
  ///
  /// A `Result` containing a `ResponseData` object upon success, or a boxed `dyn Error` if an error occurs.
  async fn proxy_request_handler(
    &mut self,
    request: RequestData,
    config: &ServerConfig,
    socket_data: &SocketData,
    error_logger: &ErrorLogger<'_>,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>>;

  /// Modifies an outgoing response before it is sent to the client.
  ///
  /// This function allows for inspection and modification of the response generated by the server
  /// or other handlers. Implementers can use this to add, remove, or alter headers, change the
  /// status code, or modify the body of the response as needed.
  ///
  /// # Parameters
  ///
  /// - `response`: A `HyperResponse` object representing the outgoing HTTP response.
  ///
  /// # Returns
  ///
  /// A `Result` containing the potentially modified `HyperResponse` object upon success, or a boxed
  /// `dyn Error` if an error occurs during processing.
  async fn response_modifying_handler(
    &mut self,
    response: HyperResponse,
  ) -> Result<HyperResponse, Box<dyn Error + Send + Sync>>;

  /// Modifies an outgoing response for forward proxy requests before it is sent to the client.
  ///
  /// This function allows for inspection and modification of the response generated by the server
  /// or other handlers. Implementers can use this to add, remove, or alter headers, change the
  /// status code, or modify the body of the response as needed.
  ///
  /// # Parameters
  ///
  /// - `response`: A `HyperResponse` object representing the outgoing HTTP response.
  ///
  /// # Returns
  ///
  /// A `Result` containing the potentially modified `HyperResponse` object upon success, or a boxed
  /// `dyn Error` if an error occurs during processing.
  async fn proxy_response_modifying_handler(
    &mut self,
    response: HyperResponse,
  ) -> Result<HyperResponse, Box<dyn Error + Send + Sync>>;
}

/// Represents a server module that can provide handlers for processing requests.
pub trait ServerModule {
  /// Retrieves the handlers associated with the server module.
  ///
  /// # Parameters
  ///
  /// - `handle`: A `Handle` to the Tokio runtime.
  ///
  /// # Returns
  ///
  /// A boxed object implementing `ServerModuleHandlers` that can be sent across threads.
  fn get_handlers(&self, handle: Handle) -> Box<dyn ServerModuleHandlers + Send>;
}
