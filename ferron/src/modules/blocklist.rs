use std::collections::HashSet;
use std::error::Error;
use std::net::IpAddr;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use hyper::{Request, StatusCode};

use crate::config::ServerConfiguration;
use crate::logging::ErrorLogger;
use crate::util::{get_entries_for_validation, get_values, IpBlockList, ModuleCache};

use super::{Module, ModuleHandlers, ModuleLoader, ResponseData, SocketData};

/// A blocklist module loader
pub struct BlocklistModuleLoader {
  cache: ModuleCache<BlocklistModule>,
}

impl BlocklistModuleLoader {
  /// Creates a new module loader
  pub fn new() -> Self {
    Self {
      cache: ModuleCache::new(vec![]),
    }
  }
}

impl ModuleLoader for BlocklistModuleLoader {
  fn load_module(
    &mut self,
    config: &ServerConfiguration,
    global_config: Option<&ServerConfiguration>,
  ) -> Result<Arc<dyn Module + Send + Sync>, Box<dyn Error + Send + Sync>> {
    Ok(
      self
        .cache
        .get_or_init::<_, Box<dyn std::error::Error + Send + Sync>>(config, move |_| {
          let mut blocklist_str_vec = Vec::new();
          for blocked_ip_config in global_config.map_or(vec![], |c| get_values!("block", c)) {
            if let Some(blocked_ip) = blocked_ip_config.as_str() {
              blocklist_str_vec.push(blocked_ip);
            }
          }

          let mut blocklist = IpBlockList::new();
          blocklist.load_from_vec(blocklist_str_vec);

          Ok(Arc::new(BlocklistModule {
            blocklist: Arc::new(blocklist),
          }))
        })?,
    )
  }

  fn get_requirements(&self) -> Vec<&'static str> {
    vec!["block"]
  }

  fn validate_configuration(
    &self,
    config: &ServerConfiguration,
    used_properties: &mut HashSet<String>,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    if let Some(entries) = get_entries_for_validation!("block", config, used_properties) {
      for entry in &entries.inner {
        for value in &entry.values {
          if !value.is_string() {
            Err(anyhow::anyhow!("Invalid blocked IP address"))?
          } else if let Some(value) = value.as_str() {
            if value.parse::<IpAddr>().is_err() {
              Err(anyhow::anyhow!("Invalid blocked IP address"))?
            }
          }
        }
      }
    }

    Ok(())
  }
}

/// A blocklist module
struct BlocklistModule {
  blocklist: Arc<IpBlockList>,
}

impl Module for BlocklistModule {
  fn get_module_handlers(&self) -> Box<dyn ModuleHandlers> {
    Box::new(BlocklistModuleHandlers {
      blocklist: self.blocklist.clone(),
    })
  }
}

/// Handlers for the blocklist module
struct BlocklistModuleHandlers {
  blocklist: Arc<IpBlockList>,
}

#[async_trait(?Send)]
impl ModuleHandlers for BlocklistModuleHandlers {
  async fn request_handler(
    &mut self,
    request: Request<BoxBody<Bytes, std::io::Error>>,
    _config: &ServerConfiguration,
    socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    Ok(ResponseData {
      request: Some(request),
      response: None,
      response_status: if self.blocklist.is_blocked(socket_data.remote_addr.ip()) {
        Some(StatusCode::FORBIDDEN)
      } else {
        None
      },
      response_headers: None,
      new_remote_address: None,
    })
  }
}
