use std::collections::HashSet;
use std::error::Error;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty};
use hyper::{header, Request, Response, StatusCode};
use tokio::sync::RwLock;

use ferron_common::logging::ErrorLogger;
use ferron_common::util::TtlCache;
use ferron_common::{config::ServerConfiguration, util::ModuleCache};
use ferron_common::{get_entries_for_validation, get_entry, get_value};

use ferron_common::modules::{Module, ModuleHandlers, ModuleLoader, RequestData, ResponseData, SocketData};

/// A trailing slash redirection module loader
pub struct TrailingSlashRedirectsModuleLoader {
  cache: ModuleCache<TrailingSlashRedirectsModule>,
  trailing_slashes_cache: Arc<RwLock<TtlCache<String, bool>>>,
}

impl Default for TrailingSlashRedirectsModuleLoader {
  fn default() -> Self {
    Self::new()
  }
}

impl TrailingSlashRedirectsModuleLoader {
  /// Creates a new module loader
  pub fn new() -> Self {
    Self {
      cache: ModuleCache::new(vec![]),
      trailing_slashes_cache: Arc::new(RwLock::new(TtlCache::new(Duration::from_millis(100)))),
    }
  }
}

impl ModuleLoader for TrailingSlashRedirectsModuleLoader {
  fn load_module(
    &mut self,
    config: &ServerConfiguration,
    _global_config: Option<&ServerConfiguration>,
    _secondary_runtime: &tokio::runtime::Runtime,
  ) -> Result<Arc<dyn Module + Send + Sync>, Box<dyn Error + Send + Sync>> {
    Ok(
      self
        .cache
        .get_or_init::<_, Box<dyn std::error::Error + Send + Sync>>(config, |_| {
          Ok(Arc::new(TrailingSlashRedirectsModule {
            cache: self.trailing_slashes_cache.clone(),
          }))
        })?,
    )
  }

  fn get_requirements(&self) -> Vec<&'static str> {
    vec!["root"]
  }

  fn validate_configuration(
    &self,
    config: &ServerConfiguration,
    used_properties: &mut HashSet<String>,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    if let Some(entries) = get_entries_for_validation!("no_trailing_redirect", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `no_trailing_redirect` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!("Invalid trailing slashes redirect disabling option"))?
        }
      }
    };

    Ok(())
  }
}

/// A trailing slash redirection module
struct TrailingSlashRedirectsModule {
  cache: Arc<RwLock<TtlCache<String, bool>>>,
}

impl Module for TrailingSlashRedirectsModule {
  fn get_module_handlers(&self) -> Box<dyn ModuleHandlers> {
    Box::new(TrailingSlashRedirectsModuleHandlers {
      cache: self.cache.clone(),
    })
  }
}

/// Handlers for the trailing slash redirection module
struct TrailingSlashRedirectsModuleHandlers {
  cache: Arc<RwLock<TtlCache<String, bool>>>,
}

#[async_trait(?Send)]
impl ModuleHandlers for TrailingSlashRedirectsModuleHandlers {
  async fn request_handler(
    &mut self,
    request: Request<BoxBody<Bytes, std::io::Error>>,
    config: &ServerConfiguration,
    _socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    if !get_value!("no_trailing_redirect", config)
      .and_then(|v| v.as_bool())
      .unwrap_or(false)
    {
      if let Some(wwwroot) = get_entry!("root", config)
        .and_then(|e| e.values.first())
        .and_then(|v| v.as_str())
      {
        let request_path = request.uri().path();
        let mut request_path_bytes = request_path.bytes();
        if request_path_bytes.len() < 1 || request_path_bytes.nth(0) != Some(b'/') {
          return Ok(ResponseData {
            request: Some(request),
            response: None,
            response_status: Some(StatusCode::BAD_REQUEST),
            response_headers: None,
            new_remote_address: None,
          });
        }

        match request_path_bytes.last() {
          Some(b'/') | None => {
            return Ok(ResponseData {
              request: Some(request),
              response: None,
              response_status: None,
              response_headers: None,
              new_remote_address: None,
            });
          }
          _ => {
            let cache_key = format!(
              "{}{}{}",
              match &config.filters.ip {
                Some(ip) => format!("{ip}-"),
                None => String::from(""),
              },
              match &config.filters.hostname {
                Some(domain) => format!("{domain}-"),
                None => String::from(""),
              },
              request_path
            );

            let original_request_path = request
              .extensions()
              .get::<RequestData>()
              .and_then(|d| d.original_url.as_ref())
              .map_or(request_path, |u| u.path());
            let original_request_query = request
              .extensions()
              .get::<RequestData>()
              .and_then(|d| d.original_url.as_ref())
              .map_or(request.uri().query(), |u| u.query());

            let read_rwlock = self.cache.read().await;
            if let Some(is_directory) = read_rwlock.get(&cache_key) {
              drop(read_rwlock);
              if is_directory {
                let new_request_uri = format!(
                  "{}/{}",
                  original_request_path,
                  match original_request_query {
                    Some(query) => format!("?{query}"),
                    None => String::from(""),
                  }
                );
                return Ok(ResponseData {
                  request: Some(request),
                  response: Some(
                    Response::builder()
                      .status(StatusCode::MOVED_PERMANENTLY)
                      .header(header::LOCATION, new_request_uri)
                      .body(Empty::new().map_err(|e| match e {}).boxed())?,
                  ),
                  response_status: None,
                  response_headers: None,
                  new_remote_address: None,
                });
              }
            } else {
              drop(read_rwlock);

              let path = Path::new(wwwroot);
              let mut relative_path = &request_path[1..];
              while relative_path.as_bytes().first().copied() == Some(b'/') {
                relative_path = &relative_path[1..];
              }

              let decoded_relative_path = match urlencoding::decode(relative_path) {
                Ok(path) => path.to_string(),
                Err(_) => {
                  return Ok(ResponseData {
                    request: Some(request),
                    response: None,
                    response_status: Some(StatusCode::BAD_REQUEST),
                    response_headers: None,
                    new_remote_address: None,
                  });
                }
              };

              let joined_pathbuf = path.join(decoded_relative_path);

              // Monoio's `fs` doesn't expose `metadata()` on Windows, so we have to spawn a blocking task to obtain the metadata on this platform
              #[cfg(feature = "runtime-tokio")]
              let metadata = {
                use tokio::fs;
                fs::metadata(&joined_pathbuf).await
              };
              #[cfg(all(feature = "runtime-monoio", unix))]
              let metadata = {
                use monoio::fs;
                fs::metadata(&joined_pathbuf).await
              };
              #[cfg(all(feature = "runtime-monoio", windows))]
              let metadata = {
                let joined_pathbuf = joined_pathbuf.clone();
                monoio::spawn_blocking(move || std::fs::metadata(joined_pathbuf))
                  .await
                  .unwrap_or(Err(std::io::Error::other(
                    "Can't spawn a blocking task to obtain the file metadata",
                  )))
              };

              match metadata {
                Ok(metadata) => {
                  let is_directory = metadata.is_dir();
                  let mut write_rwlock = self.cache.write().await;
                  write_rwlock.cleanup();
                  write_rwlock.insert(cache_key, is_directory);
                  if is_directory {
                    let new_request_uri = format!(
                      "{}/{}",
                      original_request_path,
                      match original_request_query {
                        Some(query) => format!("?{query}"),
                        None => String::from(""),
                      }
                    );
                    return Ok(ResponseData {
                      request: Some(request),
                      response: Some(
                        Response::builder()
                          .status(StatusCode::MOVED_PERMANENTLY)
                          .header(header::LOCATION, new_request_uri)
                          .body(Empty::new().map_err(|e| match e {}).boxed())?,
                      ),
                      response_status: None,
                      response_headers: None,
                      new_remote_address: None,
                    });
                  }
                }
                Err(_) => {
                  let mut write_rwlock = self.cache.write().await;
                  write_rwlock.cleanup();
                  write_rwlock.insert(cache_key, false);
                }
              }
            }
          }
        };
      }
    }

    Ok(ResponseData {
      request: Some(request),
      response: None,
      response_status: None,
      response_headers: None,
      new_remote_address: None,
    })
  }
}
