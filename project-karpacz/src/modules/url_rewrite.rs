use std::error::Error;
use std::path::Path;
use std::sync::Arc;

use crate::project_karpacz_util::ip_match::ip_match;
use crate::project_karpacz_util::match_hostname::match_hostname;
use crate::project_karpacz_util::url_rewrite_structs::{UrlRewriteMapEntry, UrlRewriteMapWrap};

use async_trait::async_trait;
use fancy_regex::RegexBuilder;
use hyper::{header, Request, StatusCode};
use project_karpacz_common::WithRuntime;
use project_karpacz_common::{
  ErrorLogger, HyperResponse, RequestData, ResponseData, ServerConfig, ServerConfigRoot,
  ServerModule, ServerModuleHandlers, SocketData,
};
use tokio::fs;
use tokio::runtime::Handle;
use yaml_rust2::Yaml;

fn url_rewrite_config_init(rewrite_map: &[Yaml]) -> Result<Vec<UrlRewriteMapEntry>, anyhow::Error> {
  let rewrite_map_iter = rewrite_map.iter();
  let mut rewrite_map_vec = Vec::new();
  for rewrite_map_entry in rewrite_map_iter {
    let regex_str = match rewrite_map_entry["regex"].as_str() {
      Some(regex_str) => regex_str,
      None => return Err(anyhow::anyhow!("Invalid URL rewrite regular expression")),
    };
    let regex = match RegexBuilder::new(regex_str)
      .case_insensitive(cfg!(windows))
      .build()
    {
      Ok(regex) => regex,
      Err(err) => {
        return Err(anyhow::anyhow!(
          "Invalid URL rewrite regular expression: {}",
          err.to_string()
        ))
      }
    };
    let replacement = match rewrite_map_entry["replacement"].as_str() {
      Some(replacement) => String::from(replacement),
      None => return Err(anyhow::anyhow!("URL rewrite rules must have replacements")),
    };
    let is_not_file = rewrite_map_entry["isNotFile"].as_bool().unwrap_or(false);
    let is_not_directory = rewrite_map_entry["isNotDirectory"]
      .as_bool()
      .unwrap_or(false);
    let last = rewrite_map_entry["last"].as_bool().unwrap_or_default();
    let allow_double_slashes = rewrite_map_entry["allowDoubleSlashes"]
      .as_bool()
      .unwrap_or(false);
    rewrite_map_vec.push(UrlRewriteMapEntry::new(
      regex,
      replacement,
      is_not_directory,
      is_not_file,
      last,
      allow_double_slashes,
    ));
  }

  Ok(rewrite_map_vec)
}

pub fn server_module_init(
  config: &ServerConfig,
) -> Result<Box<dyn ServerModule + Send + Sync>, Box<dyn Error + Send + Sync>> {
  let mut global_url_rewrite_map = Vec::new();
  let mut host_url_rewrite_maps = Vec::new();
  if let Some(rewrite_map_yaml) = config["global"]["rewriteMap"].as_vec() {
    global_url_rewrite_map = url_rewrite_config_init(rewrite_map_yaml)?;
  }

  if let Some(hosts) = config["hosts"].as_vec() {
    for host_yaml in hosts.iter() {
      let domain = host_yaml["domain"].as_str().map(String::from);
      let ip = host_yaml["ip"].as_str().map(String::from);
      if let Some(rewrite_map_yaml) = host_yaml["rewriteMap"].as_vec() {
        host_url_rewrite_maps.push(UrlRewriteMapWrap::new(
          domain,
          ip,
          url_rewrite_config_init(rewrite_map_yaml)?,
        ));
      }
    }
  }

  Ok(Box::new(UrlRewriteModule::new(
    Arc::new(global_url_rewrite_map),
    Arc::new(host_url_rewrite_maps),
  )))
}

struct UrlRewriteModule {
  global_url_rewrite_map: Arc<Vec<UrlRewriteMapEntry>>,
  host_url_rewrite_maps: Arc<Vec<UrlRewriteMapWrap>>,
}

impl UrlRewriteModule {
  fn new(
    global_url_rewrite_map: Arc<Vec<UrlRewriteMapEntry>>,
    host_url_rewrite_maps: Arc<Vec<UrlRewriteMapWrap>>,
  ) -> Self {
    UrlRewriteModule {
      global_url_rewrite_map,
      host_url_rewrite_maps,
    }
  }
}

impl ServerModule for UrlRewriteModule {
  fn get_handlers(&self, handle: Handle) -> Box<dyn ServerModuleHandlers + Send> {
    Box::new(UrlRewriteModuleHandlers {
      global_url_rewrite_map: self.global_url_rewrite_map.clone(),
      host_url_rewrite_maps: self.host_url_rewrite_maps.clone(),
      handle,
    })
  }
}
struct UrlRewriteModuleHandlers {
  global_url_rewrite_map: Arc<Vec<UrlRewriteMapEntry>>,
  host_url_rewrite_maps: Arc<Vec<UrlRewriteMapWrap>>,
  handle: Handle,
}

#[async_trait]
impl ServerModuleHandlers for UrlRewriteModuleHandlers {
  async fn request_handler(
    &mut self,
    request: RequestData,
    config: &ServerConfigRoot,
    socket_data: &SocketData,
    error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    WithRuntime::new(self.handle.clone(), async move {
      let hyper_request = request.get_hyper_request();
      let global_url_rewrite_map = self.global_url_rewrite_map.iter();
      let empty_vector = Vec::new();
      let mut host_url_rewrite_map = empty_vector.iter();

      // Should have used a HashMap instead of iterating over an array for better performance...
      for host_url_rewrite_map_wrap in self.host_url_rewrite_maps.iter() {
        if match_hostname(
          match &host_url_rewrite_map_wrap.domain {
            Some(value) => Some(value as &str),
            None => None,
          },
          match hyper_request.headers().get(header::HOST) {
            Some(value) => match value.to_str() {
              Ok(value) => Some(value),
              Err(_) => None,
            },
            None => None,
          },
        ) || match &host_url_rewrite_map_wrap.ip {
          Some(value) => ip_match(value as &str, socket_data.remote_addr.ip()),
          None => true,
        } {
          host_url_rewrite_map = host_url_rewrite_map_wrap.rewrite_map.iter();
        }
      }

      let combined_url_rewrite_map = global_url_rewrite_map.chain(host_url_rewrite_map);

      let original_url = format!(
        "{}{}",
        hyper_request.uri().path(),
        match hyper_request.uri().query() {
          Some(query) => format!("?{}", query),
          None => String::from(""),
        }
      );
      let mut rewritten_url = original_url.clone();

      let mut rewritten_url_bytes = rewritten_url.bytes();
      if rewritten_url_bytes.len() < 1 || rewritten_url_bytes.nth(0) != Some(b'/') {
        return Ok(
          ResponseData::builder(request)
            .status(StatusCode::BAD_REQUEST)
            .build(),
        );
      }

      for url_rewrite_map_entry in combined_url_rewrite_map {
        // Check if it's a file or a directory according to the rewrite map configuration
        if url_rewrite_map_entry.is_not_directory || url_rewrite_map_entry.is_not_file {
          if let Some(wwwroot) = config.get("wwwroot").as_str() {
            let path = Path::new(wwwroot);
            let mut relative_path = &rewritten_url[1..];
            while relative_path.as_bytes().first().copied() == Some(b'/') {
              relative_path = &relative_path[1..];
            }
            let relative_path_split: Vec<&str> = relative_path.split("?").collect();
            if !relative_path_split.is_empty() {
              relative_path = relative_path_split[0];
            }
            let joined_pathbuf = path.join(relative_path);
            if let Ok(metadata) = fs::metadata(joined_pathbuf).await {
              if (url_rewrite_map_entry.is_not_file && metadata.is_file())
                || (url_rewrite_map_entry.is_not_directory && metadata.is_dir())
              {
                continue;
              }
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
          return Ok(
            ResponseData::builder(request)
              .status(StatusCode::BAD_REQUEST)
              .build(),
          );
        }

        if url_rewrite_map_entry.last && old_rewritten_url != rewritten_url {
          break;
        }
      }

      if rewritten_url == original_url {
        Ok(ResponseData::builder(request).build())
      } else {
        if config.get("enableRewriteLogging").as_bool() == Some(true) {
          error_logger
            .log(&format!(
              "URL rewritten from \"{}\" to \"{}\"",
              original_url, rewritten_url
            ))
            .await;
        }
        let (hyper_request, auth_user) = request.into_parts();
        let (mut parts, body) = hyper_request.into_parts();
        let mut url_parts = parts.uri.into_parts();
        url_parts.path_and_query = Some(rewritten_url.parse()?);
        parts.uri = hyper::Uri::from_parts(url_parts)?;
        let hyper_request = Request::from_parts(parts, body);
        let request = RequestData::new(hyper_request, auth_user);
        Ok(ResponseData::builder(request).build())
      }
    })
    .await
  }

  async fn proxy_request_handler(
    &mut self,
    request: RequestData,
    _config: &ServerConfigRoot,
    _socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    Ok(ResponseData::builder(request).build())
  }

  async fn response_modifying_handler(
    &mut self,
    response: HyperResponse,
  ) -> Result<HyperResponse, Box<dyn Error + Send + Sync>> {
    Ok(response)
  }

  async fn proxy_response_modifying_handler(
    &mut self,
    response: HyperResponse,
  ) -> Result<HyperResponse, Box<dyn Error + Send + Sync>> {
    Ok(response)
  }
}
