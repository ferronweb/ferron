use std::collections::HashMap;
use std::error::Error;
use std::net::IpAddr;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use hyper::{Request, StatusCode};
use tokenbucket::TokenBucket;
use tokio::sync::{Mutex, RwLock};

use crate::config::ServerConfiguration;
use crate::get_entry;
use crate::logging::ErrorLogger;
use crate::modules::{Module, ModuleHandlers, ModuleLoader, ResponseData, SocketData};
use crate::util::{get_entries_for_validation, ModuleCache};

/// A rate limiting module loader
pub struct LimitModuleLoader {
  cache: ModuleCache<LimitModule>,
}

impl LimitModuleLoader {
  /// Creates a new module loader
  pub fn new() -> Self {
    Self {
      cache: ModuleCache::new(vec!["limit"]),
    }
  }
}

impl ModuleLoader for LimitModuleLoader {
  fn load_module(
    &mut self,
    config: &ServerConfiguration,
    _global_config: Option<&ServerConfiguration>,
  ) -> Result<Arc<dyn Module + Send + Sync>, Box<dyn Error + Send + Sync>> {
    Ok(
      self
        .cache
        .get_or_init::<_, Box<dyn std::error::Error + Send + Sync>>(config, |config| {
          let limit_entry = get_entry!("limit", config);
          let r = limit_entry
            .and_then(|e| e.props.get("rate"))
            .and_then(|v| {
              if v.is_float() {
                v.as_f64()
              } else if v.is_integer() {
                v.as_i128().map(|v| v as f64)
              } else {
                None
              }
            })
            .unwrap_or(25.0);
          let b = limit_entry
            .and_then(|e| e.props.get("burst"))
            .and_then(|v| {
              if v.is_float() {
                v.as_f64()
              } else if v.is_integer() {
                v.as_i128().map(|v| v as f64)
              } else {
                None
              }
            })
            .unwrap_or(r * 5.0);
          Ok(Arc::new(LimitModule {
            token_buckets: Arc::new(RwLock::new(HashMap::new())),
            r,
            b,
          }))
        })?,
    )
  }

  fn get_requirements(&self) -> Vec<&'static str> {
    vec!["limit"]
  }

  fn validate_configuration(
    &self,
    config: &ServerConfiguration,
    used_properties: &mut std::collections::HashSet<String>,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    if let Some(entries) = get_entries_for_validation!("limit", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `limit` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!("Invalid rate limit enabling option"))?
        } else if !entry
          .props
          .get("rate")
          .is_none_or(|v| v.is_integer() || v.is_float())
        {
          Err(anyhow::anyhow!(
            "The maximum average rate per second must be an integer or a float"
          ))?
        } else if !entry
          .props
          .get("burst")
          .is_none_or(|v| v.is_integer() || v.is_float())
        {
          Err(anyhow::anyhow!(
            "The maximum peak rate per second must be an integer or a float"
          ))?
        }
      }
    }

    Ok(())
  }
}

/// A rate limiting module
struct LimitModule {
  token_buckets: Arc<RwLock<HashMap<IpAddr, Arc<Mutex<TokenBucket>>>>>,
  r: f64,
  b: f64,
}

impl Module for LimitModule {
  fn get_module_handlers(&self) -> Box<dyn ModuleHandlers> {
    Box::new(LimitModuleHandlers {
      token_buckets: self.token_buckets.clone(),
      r: self.r,
      b: self.b,
    })
  }
}

/// Handlers for the rate limiting module
struct LimitModuleHandlers {
  token_buckets: Arc<RwLock<HashMap<IpAddr, Arc<Mutex<TokenBucket>>>>>,
  r: f64,
  b: f64,
}

#[async_trait(?Send)]
impl ModuleHandlers for LimitModuleHandlers {
  async fn request_handler(
    &mut self,
    request: Request<BoxBody<Bytes, std::io::Error>>,
    _config: &ServerConfiguration,
    socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    let token_buckets_read_locked = self.token_buckets.read().await;
    let token_bucket_mutex =
      if let Some(token_bucket) = token_buckets_read_locked.get(&socket_data.remote_addr.ip()) {
        let token_bucket = token_bucket.clone();
        drop(token_buckets_read_locked);
        token_bucket
      } else {
        drop(token_buckets_read_locked);
        let new_token_bucket = Arc::new(Mutex::new(TokenBucket::new(self.r, self.b)));
        self
          .token_buckets
          .write()
          .await
          .insert(socket_data.remote_addr.ip(), new_token_bucket.clone());
        new_token_bucket
      };
    let mut token_bucket = token_bucket_mutex.lock().await;

    if token_bucket.acquire(1.0).is_err() {
      Ok(ResponseData {
        request: Some(request),
        response: None,
        response_status: Some(StatusCode::TOO_MANY_REQUESTS),
        response_headers: None,
        new_remote_address: None,
      })
    } else {
      Ok(ResponseData {
        request: Some(request),
        response: None,
        response_status: None,
        response_headers: None,
        new_remote_address: None,
      })
    }
  }
}
