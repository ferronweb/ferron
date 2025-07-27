use std::collections::HashSet;
use std::error::Error;
use std::net::IpAddr;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use cidr::IpCidr;
use http_body_util::combinators::BoxBody;
use hyper::{Request, StatusCode};

use ferron_common::config::ServerConfiguration;
use ferron_common::logging::ErrorLogger;
use ferron_common::util::{IpBlockList, ModuleCache};
use ferron_common::{get_entries_for_validation, get_values};

use ferron_common::modules::{Module, ModuleHandlers, ModuleLoader, ResponseData, SocketData};

/// A blocklist module loader
pub struct BlocklistModuleLoader {
  cache: ModuleCache<BlocklistModule>,
}

impl Default for BlocklistModuleLoader {
  fn default() -> Self {
    Self::new()
  }
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
    _secondary_runtime: &tokio::runtime::Runtime,
  ) -> Result<Arc<dyn Module + Send + Sync>, Box<dyn Error + Send + Sync>> {
    Ok(
      self
        .cache
        .get_or_init::<_, Box<dyn std::error::Error + Send + Sync>>(config, move |_| {
          let blocklist_value_vec = global_config.map_or(vec![], |c| get_values!("block", c));
          let blocklist = if !blocklist_value_vec.is_empty() {
            let mut blocklist_str_vec = Vec::new();
            for blocked_ip_config in blocklist_value_vec {
              if let Some(blocked_ip) = blocked_ip_config.as_str() {
                blocklist_str_vec.push(blocked_ip);
              }
            }

            let mut blocklist = IpBlockList::new();
            blocklist.load_from_vec(blocklist_str_vec);
            Some(Arc::new(blocklist))
          } else {
            None
          };

          let allowlist_value_vec = global_config.map_or(vec![], |c| get_values!("allow", c));
          let allowlist = if !allowlist_value_vec.is_empty() {
            let mut allowlist_str_vec = Vec::new();
            for allowed_ip_config in allowlist_value_vec {
              if let Some(allowed_ip) = allowed_ip_config.as_str() {
                allowlist_str_vec.push(allowed_ip);
              }
            }

            let mut allowlist = IpBlockList::new();
            allowlist.load_from_vec(allowlist_str_vec);
            Some(Arc::new(allowlist))
          } else {
            None
          };

          Ok(Arc::new(BlocklistModule { blocklist, allowlist }))
        })?,
    )
  }

  fn get_requirements(&self) -> Vec<&'static str> {
    vec!["block", "allow"]
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
            if value.parse::<IpAddr>().is_err() && value.parse::<IpCidr>().is_err() {
              Err(anyhow::anyhow!("Invalid blocked IP address"))?
            }
          }
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("allow", config, used_properties) {
      for entry in &entries.inner {
        for value in &entry.values {
          if !value.is_string() {
            Err(anyhow::anyhow!("Invalid allowed IP address"))?
          } else if let Some(value) = value.as_str() {
            if value.parse::<IpAddr>().is_err() && value.parse::<IpCidr>().is_err() {
              Err(anyhow::anyhow!("Invalid allowed IP address"))?
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
  allowlist: Option<Arc<IpBlockList>>,
  blocklist: Option<Arc<IpBlockList>>,
}

impl Module for BlocklistModule {
  fn get_module_handlers(&self) -> Box<dyn ModuleHandlers> {
    Box::new(BlocklistModuleHandlers {
      allowlist: self.allowlist.clone(),
      blocklist: self.blocklist.clone(),
    })
  }
}

/// Handlers for the blocklist module
struct BlocklistModuleHandlers {
  allowlist: Option<Arc<IpBlockList>>,
  blocklist: Option<Arc<IpBlockList>>,
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
    let blocked = self
      .blocklist
      .as_ref()
      .map_or(false, |blocklist| blocklist.is_blocked(socket_data.remote_addr.ip()))
      || !self
        .allowlist
        .as_ref()
        .map_or(true, |allowlist| allowlist.is_blocked(socket_data.remote_addr.ip()));
    Ok(ResponseData {
      request: Some(request),
      response: None,
      response_status: if blocked { Some(StatusCode::FORBIDDEN) } else { None },
      response_headers: None,
      new_remote_address: None,
    })
  }
}
