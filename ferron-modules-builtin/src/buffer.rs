use std::error::Error;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::StreamExt;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, BodyStream, StreamBody};
use hyper::{Request, Response};

use ferron_common::logging::ErrorLogger;
use ferron_common::{config::ServerConfiguration, util::ModuleCache};
use ferron_common::{get_entries_for_validation, get_value};

use ferron_common::modules::{Module, ModuleHandlers, ModuleLoader, ResponseData, SocketData};

/// A buffering module loader
pub struct BufferModuleLoader {
  cache: ModuleCache<BufferModule>,
}

impl Default for BufferModuleLoader {
  fn default() -> Self {
    Self::new()
  }
}

impl BufferModuleLoader {
  /// Creates a new module loader
  pub fn new() -> Self {
    Self {
      cache: ModuleCache::new(vec![]),
    }
  }
}

impl ModuleLoader for BufferModuleLoader {
  fn load_module(
    &mut self,
    config: &ServerConfiguration,
    _global_config: Option<&ServerConfiguration>,
    _secondary_runtime: &tokio::runtime::Runtime,
  ) -> Result<Arc<dyn Module + Send + Sync>, Box<dyn Error + Send + Sync>> {
    Ok(
      self
        .cache
        .get_or_init::<_, Box<dyn std::error::Error + Send + Sync>>(config, move |_| Ok(Arc::new(BufferModule)))?,
    )
  }

  fn get_requirements(&self) -> Vec<&'static str> {
    vec!["buffer_request", "buffer_response"]
  }

  fn validate_configuration(
    &self,
    config: &ServerConfiguration,
    used_properties: &mut std::collections::HashSet<String>,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    if let Some(entries) = get_entries_for_validation!("buffer_request", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `buffer_request` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_integer() && !entry.values[0].is_null() {
          Err(anyhow::anyhow!("Invalid HTTP request body buffer size"))?
        } else if let Some(value) = entry.values[0].as_i128() {
          if value < 0 {
            Err(anyhow::anyhow!("Invalid HTTP request body buffer size"))?
          }
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("buffer_response", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `buffer_response` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_integer() && !entry.values[0].is_null() {
          Err(anyhow::anyhow!("Invalid HTTP response body buffer size"))?
        } else if let Some(value) = entry.values[0].as_i128() {
          if value < 0 {
            Err(anyhow::anyhow!("Invalid HTTP response body buffer size"))?
          }
        }
      }
    }

    Ok(())
  }
}

/// A buffering module
struct BufferModule;

impl Module for BufferModule {
  fn get_module_handlers(&self) -> Box<dyn ModuleHandlers> {
    Box::new(BufferModuleHandlers {
      response_buffer_size: None,
    })
  }
}

/// Handlers for the buffering module
struct BufferModuleHandlers {
  response_buffer_size: Option<usize>,
}

#[async_trait(?Send)]
impl ModuleHandlers for BufferModuleHandlers {
  async fn request_handler(
    &mut self,
    mut request: Request<BoxBody<Bytes, std::io::Error>>,
    config: &ServerConfiguration,
    _socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    if let Some(request_buffer_size) = get_value!("buffer_request", config).and_then(|v| v.as_i128()) {
      let (request_parts, mut request_body) = request.into_parts();
      let mut request_body_buffer = Vec::new();
      let mut data_len: usize = 0;
      while let Some(frame) = request_body.frame().await {
        let frame = frame?;
        match frame.into_data() {
          Ok(data) => {
            data_len += data.len();
            request_body_buffer.push(hyper::body::Frame::data(data));
            if data_len >= request_buffer_size as usize {
              break;
            }
          }
          Err(frame) => {
            request_body_buffer.push(frame);
            break;
          }
        }
      }
      request = Request::from_parts(
        request_parts,
        BodyExt::boxed(StreamBody::new(
          futures_util::stream::iter(request_body_buffer)
            .map(Ok)
            .chain(BodyStream::new(request_body)),
        )),
      );
    }

    self.response_buffer_size = get_value!("buffer_response", config)
      .and_then(|v| v.as_i128())
      .map(|s| s as usize);

    Ok(ResponseData {
      request: Some(request),
      response: None,
      response_status: None,
      response_headers: None,
      new_remote_address: None,
    })
  }

  async fn response_modifying_handler(
    &mut self,
    mut response: Response<BoxBody<Bytes, std::io::Error>>,
  ) -> Result<Response<BoxBody<Bytes, std::io::Error>>, Box<dyn Error>> {
    if let Some(response_buffer_size) = self.response_buffer_size {
      let (response_parts, mut response_body) = response.into_parts();
      let mut response_body_buffer = Vec::new();
      let mut data_len: usize = 0;
      while let Some(frame) = response_body.frame().await {
        let frame = frame?;
        match frame.into_data() {
          Ok(data) => {
            data_len += data.len();
            response_body_buffer.push(hyper::body::Frame::data(data));
            if data_len >= response_buffer_size {
              break;
            }
          }
          Err(frame) => {
            response_body_buffer.push(frame);
            break;
          }
        }
      }
      response = Response::from_parts(
        response_parts,
        BodyExt::boxed(StreamBody::new(
          futures_util::stream::iter(response_body_buffer)
            .map(Ok)
            .chain(BodyStream::new(response_body)),
        )),
      );
    }
    Ok(response)
  }
}
