//! This module provides an example implementation of a Ferron server module.
//! It demonstrates how to create a simple HTTP handler that responds to specific paths.

use std::error::Error;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::{Request, Response};

use ferron_common::config::ServerConfiguration;
use ferron_common::get_entries_for_validation;
use ferron_common::logging::ErrorLogger;
use ferron_common::modules::{Module, ModuleHandlers, ModuleLoader, ResponseData, SocketData};
use ferron_common::util::ModuleCache;

/// An example module loader that demonstrates how to create and manage modules in Ferron.
///
/// The module loader is responsible for:
/// 1. Creating modules based on server configuration
/// 2. Caching modules to avoid recreating them for identical configurations
/// 3. Validating configuration parameters
pub struct ExampleModuleLoader {
  /// Module cache that stores instances of ExampleModule indexed by configuration parameters
  cache: ModuleCache<ExampleModule>,
}

impl ExampleModuleLoader {
  /// Creates a new module loader with an empty cache
  ///
  /// Returns a ready-to-use ExampleModuleLoader instance that can create and cache
  /// ExampleModule instances based on server configurations.
  pub fn new() -> Self {
    Self {
      // Initialize with an empty vector since this module doesn't depend on specific properties
      cache: ModuleCache::new(vec![]),
    }
  }
}

impl ModuleLoader for ExampleModuleLoader {
  /// Creates or retrieves a cached module instance based on the server configuration
  ///
  /// # Parameters
  /// * `config` - The server configuration for this specific module instance
  /// * `_global_config` - Optional global server configuration (unused in this example)
  ///
  /// # Returns
  /// A thread-safe, reference-counted module instance that implements the Module trait
  fn load_module(
    &mut self,
    config: &ServerConfiguration,
    _global_config: Option<&ServerConfiguration>,
    _secondary_runtime: &tokio::runtime::Runtime,
  ) -> Result<Arc<dyn Module + Send + Sync>, Box<dyn Error + Send + Sync>> {
    // Either get an existing module from cache or create a new one
    Ok(
      self
        .cache
        .get_or_init::<_, Box<dyn std::error::Error + Send + Sync>>(config, move |_| Ok(Arc::new(ExampleModule)))?,
    )
  }

  /// Specifies the configuration entries required by this module
  ///
  /// # Returns
  /// A vector of configuration property names that should be present
  fn get_requirements(&self) -> Vec<&'static str> {
    // This module requires the "example_handler" configuration property
    vec!["example_handler"]
  }

  /// Validates that the configuration entries for this module are correct
  ///
  /// # Parameters
  /// * `config` - The server configuration to validate
  /// * `used_properties` - A set to track which properties have been used/validated
  ///
  /// # Returns
  /// Ok if validation passes, an error with a detailed message otherwise
  fn validate_configuration(
    &self,
    config: &ServerConfiguration,
    used_properties: &mut std::collections::HashSet<String>,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    // Check if the "example" configuration entry exists and validate it
    if let Some(entries) = get_entries_for_validation!("example", config, used_properties) {
      for entry in &entries.inner {
        // Validate that each entry has exactly one value
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `example` configuration property must have exactly one value"
          ))?
        // Validate that the value is a boolean
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!("Invalid example handler enabling option"))?
        }
      }
    }

    Ok(())
  }
}

/// A simple example module that demonstrates a basic HTTP request handler
///
/// This is implemented as a zero-sized struct since it doesn't need to store any state.
/// In more complex modules, this would typically contain configuration parameters,
/// connection pools, or other state needed by the handlers.
struct ExampleModule;

impl Module for ExampleModule {
  /// Creates and returns handler instances for this module
  ///
  /// # Returns
  /// A boxed trait object implementing the ModuleHandlers trait
  fn get_module_handlers(&self) -> Box<dyn ModuleHandlers> {
    // Create a new instance of our handlers and box it
    Box::new(ExampleModuleHandlers)
  }
}

/// Handlers that process HTTP requests for the example module
///
/// This implementation demonstrates a simple path-based routing system
/// that responds with "Hello World!" for requests to "/hello".
/// For all other paths, it passes the request through without modification.
struct ExampleModuleHandlers;

#[async_trait(?Send)]
impl ModuleHandlers for ExampleModuleHandlers {
  /// Processes incoming HTTP requests and generates appropriate responses
  ///
  /// # Parameters
  /// * `request` - The incoming HTTP request with body
  /// * `_config` - Server configuration (unused in this example)
  /// * `_socket_data` - Socket connection information (unused in this example)
  /// * `_error_logger` - Logger for recording errors (unused in this example)
  ///
  /// # Returns
  /// A ResponseData struct containing either a response or the original request
  async fn request_handler(
    &mut self,
    request: Request<BoxBody<Bytes, std::io::Error>>,
    _config: &ServerConfiguration,
    _socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    // Check if the request is for the "/hello" path
    let is_hello = request.uri().path() == "/hello";

    // Return a ResponseData with appropriate fields
    Ok(ResponseData {
      // Include the original request (required for non-handled routes)
      request: Some(request),

      // If path is "/hello", create a response with "Hello World!" body
      // Otherwise, return None to let other modules handle the request
      response: if is_hello {
        Some(
          Response::builder().body(
            Full::new("Hello World!".into())
              .map_err(|e| match e {}) // Empty match because Full::new never fails
              .boxed(),
          )?,
        )
      } else {
        None
      },
      response_status: None,    // No special status code needed
      response_headers: None,   // No additional headers needed
      new_remote_address: None, // No address rewriting needed
    })
  }

  // Note: This module doesn't override response_modifying_handler
  // so it uses the default implementation from the trait
}
