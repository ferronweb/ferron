use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use fancy_regex::{Regex, RegexBuilder};
use http_body_util::combinators::BoxBody;
use hyper::{Request, StatusCode};
use tokio::sync::RwLock;

use crate::logging::ErrorLogger;
use crate::util::{get_entries, get_entries_for_validation, get_entry, get_value, TtlCache};
use crate::{config::ServerConfiguration, util::ModuleCache};

use super::{Module, ModuleHandlers, ModuleLoader, RequestData, ResponseData, SocketData};

/// A URL rewrite rule
struct UrlRewriteRule {
  regex: Regex,
  replacement: String,
  is_not_directory: bool,
  is_not_file: bool,
  last: bool,
  allow_double_slashes: bool,
}

/// A URL rewriting module loader
pub struct RewriteModuleLoader {
  cache: ModuleCache<RewriteModule>,
  metadata_cache: Arc<RwLock<TtlCache<PathBuf, (bool, bool)>>>,
}

impl RewriteModuleLoader {
  /// Creates a new module loader
  pub fn new() -> Self {
    Self {
      cache: ModuleCache::new(vec!["rewrite"]),
      metadata_cache: Arc::new(RwLock::new(TtlCache::new(Duration::from_millis(100)))),
    }
  }
}

impl ModuleLoader for RewriteModuleLoader {
  fn load_module(
    &mut self,
    config: &ServerConfiguration,
    _global_config: Option<&ServerConfiguration>,
  ) -> Result<Arc<dyn Module + Send + Sync>, Box<dyn Error + Send + Sync>> {
    Ok(
      self
        .cache
        .get_or_init::<_, Box<dyn std::error::Error + Send + Sync>>(config, |_| {
          let mut rewrite_rules = Vec::new();
          if let Some(rewrite_config_entries) = get_entries!("rewrite", config) {
            for rewrite_config_entry in &rewrite_config_entries.inner {
              let regex_str = match rewrite_config_entry.values.first().and_then(|v| v.as_str()) {
                Some(regex_str) => regex_str,
                None => Err(anyhow::anyhow!("Invalid URL rewrite regular expression"))?,
              };
              let regex = match RegexBuilder::new(regex_str)
                .case_insensitive(cfg!(windows))
                .build()
              {
                Ok(regex) => regex,
                Err(err) => Err(anyhow::anyhow!(
                  "Invalid URL rewrite regular expression: {}",
                  err.to_string()
                ))?,
              };
              let replacement = match rewrite_config_entry.values.get(1).and_then(|v| v.as_str()) {
                Some(replacement) => String::from(replacement),
                None => Err(anyhow::anyhow!("Invalid URL rewrite replacement"))?,
              };
              let is_not_directory = !rewrite_config_entry
                .props
                .pin_owned()
                .get("directory")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
              let is_not_file = !rewrite_config_entry
                .props
                .pin_owned()
                .get("file")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
              let last = rewrite_config_entry
                .props
                .pin_owned()
                .get("last")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
              let allow_double_slashes = rewrite_config_entry
                .props
                .pin_owned()
                .get("allow_double_slashes")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
              rewrite_rules.push(UrlRewriteRule {
                regex,
                replacement,
                is_not_directory,
                is_not_file,
                last,
                allow_double_slashes,
              });
            }
          }
          Ok(Arc::new(RewriteModule {
            rewrite_rules: Arc::new(rewrite_rules),
            metadata_cache: self.metadata_cache.clone(),
          }))
        })?,
    )
  }

  fn validate_configuration(
    &self,
    config: &ServerConfiguration,
    used_properties: &mut std::collections::HashSet<String>,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    if let Some(entries) = get_entries_for_validation!("rewrite", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 2 {
          Err(anyhow::anyhow!(
            "The `rewrite` configuration property must have exactly two values"
          ))?
        } else if !entry.values[0].is_string() {
          Err(anyhow::anyhow!(
            "The URL rewrite regular expression must be a string"
          ))?
        } else if !entry.values[1].is_string() {
          Err(anyhow::anyhow!(
            "The URL rewrite replacement must be a string"
          ))?
        } else if !entry
          .props
          .pin_owned()
          .get("directory")
          .is_none_or(|v| v.is_bool())
        {
          Err(anyhow::anyhow!(
            "The URL rewrite disabling when it's a directory option must be boolean"
          ))?
        } else if !entry
          .props
          .pin_owned()
          .get("file")
          .is_none_or(|v| v.is_bool())
        {
          Err(anyhow::anyhow!(
            "The URL rewrite disabling when it's a file option must be boolean"
          ))?
        } else if !entry
          .props
          .pin_owned()
          .get("last")
          .is_none_or(|v| v.is_bool())
        {
          Err(anyhow::anyhow!(
            "The URL rewrite last rule option must be boolean"
          ))?
        } else if !entry
          .props
          .pin_owned()
          .get("allow_double_slashes")
          .is_none_or(|v| v.is_bool())
        {
          Err(anyhow::anyhow!(
            "The URL rewrite double slashes allowing option must be boolean"
          ))?
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("rewrite_log", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `rewrite_log` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!("Invalid URL rewrite log enabling option"))?
        }
      }
    }

    Ok(())
  }
}

/// A URL rewriting module
struct RewriteModule {
  rewrite_rules: Arc<Vec<UrlRewriteRule>>,
  metadata_cache: Arc<RwLock<TtlCache<PathBuf, (bool, bool)>>>,
}

impl Module for RewriteModule {
  fn get_module_handlers(&self) -> Box<dyn ModuleHandlers> {
    Box::new(RewriteModuleHandlers {
      rewrite_rules: self.rewrite_rules.clone(),
      metadata_cache: self.metadata_cache.clone(),
    })
  }
}

/// Handlers for the URL rewriting module
struct RewriteModuleHandlers {
  rewrite_rules: Arc<Vec<UrlRewriteRule>>,
  metadata_cache: Arc<RwLock<TtlCache<PathBuf, (bool, bool)>>>,
}

#[async_trait(?Send)]
impl ModuleHandlers for RewriteModuleHandlers {
  async fn request_handler(
    &mut self,
    mut request: Request<BoxBody<Bytes, std::io::Error>>,
    config: &ServerConfiguration,
    _socket_data: &SocketData,
    error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    let original_url = format!(
      "{}{}",
      request.uri().path(),
      match request.uri().query() {
        Some(query) => format!("?{}", query),
        None => String::from(""),
      }
    );
    let mut rewritten_url = original_url.clone();

    let mut rewritten_url_bytes = rewritten_url.bytes();
    if rewritten_url_bytes.len() < 1 || rewritten_url_bytes.nth(0) != Some(b'/') {
      return Ok(ResponseData {
        request: Some(request),
        response: None,
        response_status: Some(StatusCode::BAD_REQUEST),
        response_headers: None,
        new_remote_address: None,
      });
    }

    for url_rewrite_map_entry in self.rewrite_rules.iter() {
      // Check if it's a file or a directory according to the rewrite map configuration
      if url_rewrite_map_entry.is_not_directory || url_rewrite_map_entry.is_not_file {
        if let Some(wwwroot) = get_entry!("root", config)
          .and_then(|e| {
            let first = e.values.first();
            first.cloned()
          })
          .and_then(|v| v.to_string())
        {
          let path = Path::new(&wwwroot);
          let mut relative_path = &rewritten_url[1..];
          while relative_path.as_bytes().first().copied() == Some(b'/') {
            relative_path = &relative_path[1..];
          }
          let relative_path_split: Vec<&str> = relative_path.split("?").collect();
          if !relative_path_split.is_empty() {
            relative_path = relative_path_split[0];
          }
          let joined_pathbuf = path.join(relative_path);

          let metadata_cache = self.metadata_cache.read().await;
          let (is_file, is_directory) = if let Some(data) = metadata_cache.get(&joined_pathbuf) {
            drop(metadata_cache);
            data
          } else {
            drop(metadata_cache);

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

            let data = if let Ok(metadata) = metadata {
              (metadata.is_file(), metadata.is_dir())
            } else {
              (false, false)
            };

            let mut metadata_cache = self.metadata_cache.write().await;
            metadata_cache.cleanup();
            metadata_cache.insert(joined_pathbuf, data);

            data
          };

          if (url_rewrite_map_entry.is_not_file && is_file)
            || (url_rewrite_map_entry.is_not_directory && is_directory)
          {
            continue;
          }
        }
      }

      if !url_rewrite_map_entry.allow_double_slashes {
        while rewritten_url.contains("//") {
          rewritten_url = rewritten_url.replace("//", "/");
        }
      }

      // Actual URL rewriting
      let old_rewritten_url = rewritten_url;
      rewritten_url = url_rewrite_map_entry
        .regex
        .replace(&old_rewritten_url, &url_rewrite_map_entry.replacement)
        .to_string();

      let mut rewritten_url_bytes = rewritten_url.bytes();
      if rewritten_url_bytes.len() < 1 || rewritten_url_bytes.nth(0) != Some(b'/') {
        return Ok(ResponseData {
          request: Some(request),
          response: None,
          response_status: Some(StatusCode::BAD_REQUEST),
          response_headers: None,
          new_remote_address: None,
        });
      }

      if url_rewrite_map_entry.last && old_rewritten_url != rewritten_url {
        break;
      }
    }

    if rewritten_url != original_url {
      if get_value!("rewrite_log", config)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
      {
        error_logger
          .log(&format!(
            "URL rewritten from \"{}\" to \"{}\"",
            original_url, rewritten_url
          ))
          .await;
      }
      let request_data = request.extensions_mut().get_mut::<RequestData>();
      if let Some(request_data) = request_data {
        if request_data.original_url.is_none() {
          let mut url_parts = request.uri().to_owned().into_parts();
          url_parts.path_and_query = Some(rewritten_url.parse()?);
          *request.uri_mut() = hyper::Uri::from_parts(url_parts)?;
        }
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
