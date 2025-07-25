use std::error::Error;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use http_body_util::BodyExt;
use hyper::{header, Request, Response};

use crate::config::ServerConfiguration;
use crate::logging::ErrorLogger;
use crate::modules::{Module, ModuleHandlers, ModuleLoader, ResponseData, SocketData};
use crate::util::{get_entries, get_entries_for_validation, get_value, get_values, BodyReplacer, ModuleCache};

/// A response replacement module loader
pub struct ReplaceModuleLoader {
  cache: ModuleCache<ReplaceModule>,
}

impl ReplaceModuleLoader {
  /// Creates a new module loader
  pub fn new() -> Self {
    Self {
      cache: ModuleCache::new(vec!["replace", "replace_once"]),
    }
  }
}

impl ModuleLoader for ReplaceModuleLoader {
  fn load_module(
    &mut self,
    config: &ServerConfiguration,
    _global_config: Option<&ServerConfiguration>,
    _secondary_runtime: &tokio::runtime::Runtime,
  ) -> Result<Arc<dyn Module + Send + Sync>, Box<dyn Error + Send + Sync>> {
    Ok(
      self
        .cache
        .get_or_init::<_, Box<dyn std::error::Error + Send + Sync>>(config, move |config| {
          let replacers = Arc::new(
            get_entries!("replace", config)
              .map_or(vec![].as_ref(), |e| &e.inner)
              .iter()
              .filter_map(|e| {
                if let Some(searched) = e.values.first().and_then(|v| v.as_str()) {
                  if let Some(replacement) = e.values.get(1).and_then(|v| v.as_str()) {
                    return Some(BodyReplacer::new(
                      searched.as_bytes(),
                      replacement.as_bytes(),
                      e.props.get("once").and_then(|v| v.as_bool()).unwrap_or(true),
                    ));
                  }
                }

                None
              })
              .collect::<Vec<_>>(),
          );
          Ok(Arc::new(ReplaceModule { replacers }))
        })?,
    )
  }

  fn get_requirements(&self) -> Vec<&'static str> {
    vec!["replace"]
  }

  fn validate_configuration(
    &self,
    config: &ServerConfiguration,
    used_properties: &mut std::collections::HashSet<String>,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    if let Some(entries) = get_entries_for_validation!("replace", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 2 {
          Err(anyhow::anyhow!(
            "The `replace` configuration property must have exactly two values"
          ))?
        } else if !entry.values[0].is_string() {
          Err(anyhow::anyhow!("The string to be replaced in a body must be a string"))?
        } else if !entry.values[1].is_string() {
          Err(anyhow::anyhow!("The replacement string for a body must be a string"))?
        } else if !entry.props.get("once").is_none_or(|v| v.is_bool()) {
          Err(anyhow::anyhow!("Invalid once body replacement enabling option"))?
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("replace_last_modified", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `replace_last_modified` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!(
            "Invalid \"Last-Modified\" header preserving during the body replacement enabling option"
          ))?
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("replace_filter_types", config, used_properties) {
      for entry in &entries.inner {
        for value in &entry.values {
          if !value.is_string() {
            Err(anyhow::anyhow!("Invalid body replacement enabled MIME type"))?
          }
        }
      }
    }

    Ok(())
  }
}

/// A response replacement module
struct ReplaceModule {
  replacers: Arc<Vec<BodyReplacer>>,
}

impl Module for ReplaceModule {
  fn get_module_handlers(&self) -> Box<dyn ModuleHandlers> {
    Box::new(ReplaceModuleHandlers {
      replacers: self.replacers.clone(),
      preserve_last_modified: false,
      filter_types: Vec::new(),
    })
  }
}

/// Handlers for the response replacement module
struct ReplaceModuleHandlers {
  replacers: Arc<Vec<BodyReplacer>>,
  preserve_last_modified: bool,
  filter_types: Vec<String>,
}

#[async_trait(?Send)]
impl ModuleHandlers for ReplaceModuleHandlers {
  async fn request_handler(
    &mut self,
    request: Request<BoxBody<Bytes, std::io::Error>>,
    config: &ServerConfiguration,
    _socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    self.preserve_last_modified = get_value!("replace_last_modified", config)
      .and_then(|v| v.as_bool())
      .unwrap_or(false);

    let mut filter_types = Vec::new();
    for filter_type_config in get_values!("replace_filter_types", config) {
      if let Some(filter_type) = filter_type_config.as_str() {
        filter_types.push(filter_type.to_string());
      }
    }
    if filter_types.is_empty() {
      filter_types.push("text/html".to_string());
    }
    self.filter_types = filter_types;

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
    response: Response<BoxBody<Bytes, std::io::Error>>,
  ) -> Result<Response<BoxBody<Bytes, std::io::Error>>, Box<dyn Error>> {
    let mut can_replace = !response.headers().contains_key(header::CONTENT_ENCODING); // Don't corrupt compressed data
    let response_mime_type = response
      .headers()
      .get(header::CONTENT_TYPE)
      .map(|h| String::from_utf8_lossy(h.as_bytes()));

    if can_replace {
      for filter_type in &self.filter_types {
        if filter_type == "*"
          || response_mime_type.as_deref().map(|t| {
            if let Some((mime_type, _)) = t.split_once(';') {
              mime_type.trim()
            } else {
              t.trim()
            }
          }) == Some(filter_type)
        {
          can_replace = true;
          break;
        }
      }
    }

    if can_replace {
      let (mut replaced_response_parts, mut replaced_response_body) = response.into_parts();
      if !self.preserve_last_modified {
        while replaced_response_parts.headers.remove(header::LAST_MODIFIED).is_some() {}
      }

      while replaced_response_parts.headers.remove(header::CONTENT_LENGTH).is_some() {}

      for replacer in self.replacers.iter() {
        replaced_response_body = replacer.wrap(replaced_response_body).boxed();
      }
      let response = Response::from_parts(replaced_response_parts, replaced_response_body);
      Ok(response)
    } else {
      Ok(response)
    }
  }
}
