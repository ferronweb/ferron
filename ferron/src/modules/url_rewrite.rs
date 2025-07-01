use std::error::Error;
use std::path::Path;

use crate::ferron_util::obtain_config_struct_vec::ObtainConfigStructVec;

use crate::ferron_common::{
  ErrorLogger, HyperResponse, RequestData, ResponseData, ServerConfig, ServerModule,
  ServerModuleHandlers, SocketData,
};
use crate::ferron_common::{HyperUpgraded, WithRuntime};
use async_trait::async_trait;
use fancy_regex::{Regex, RegexBuilder};
use hyper::{header, Request, StatusCode};
use hyper_tungstenite::HyperWebsocket;
use tokio::fs;
use tokio::runtime::Handle;
use yaml_rust2::Yaml;

struct UrlRewriteMapEntry {
  regex: Regex,
  replacement: String,
  is_not_directory: bool,
  is_not_file: bool,
  last: bool,
  allow_double_slashes: bool,
}

impl UrlRewriteMapEntry {
  fn new(
    regex: Regex,
    replacement: String,
    is_not_directory: bool,
    is_not_file: bool,
    last: bool,
    allow_double_slashes: bool,
  ) -> Self {
    Self {
      regex,
      replacement,
      is_not_directory,
      is_not_file,
      last,
      allow_double_slashes,
    }
  }
}

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
  Ok(Box::new(UrlRewriteModule::new(ObtainConfigStructVec::new(
    config,
    |config| {
      if let Some(rewrite_map_yaml) = config["rewriteMap"].as_vec() {
        Ok(url_rewrite_config_init(rewrite_map_yaml)?)
      } else {
        Ok(vec![])
      }
    },
  )?)))
}

struct UrlRewriteModule {
  url_rewrite_maps: ObtainConfigStructVec<UrlRewriteMapEntry>,
}

impl UrlRewriteModule {
  fn new(url_rewrite_maps: ObtainConfigStructVec<UrlRewriteMapEntry>) -> Self {
    Self { url_rewrite_maps }
  }
}

impl ServerModule for UrlRewriteModule {
  fn get_handlers(&self, handle: Handle) -> Box<dyn ServerModuleHandlers + Send> {
    Box::new(UrlRewriteModuleHandlers {
      url_rewrite_maps: self.url_rewrite_maps.clone(),
      handle,
    })
  }
}
struct UrlRewriteModuleHandlers {
  url_rewrite_maps: ObtainConfigStructVec<UrlRewriteMapEntry>,
  handle: Handle,
}

#[async_trait]
impl ServerModuleHandlers for UrlRewriteModuleHandlers {
  async fn request_handler(
    &mut self,
    request: RequestData,
    config: &ServerConfig,
    socket_data: &SocketData,
    error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    WithRuntime::new(self.handle.clone(), async move {
      let hyper_request = request.get_hyper_request();
      let combined_url_rewrite_map = self.url_rewrite_maps.obtain(
        match hyper_request.headers().get(header::HOST) {
          Some(value) => value.to_str().ok(),
          None => None,
        },
        socket_data.remote_addr.ip(),
        request
          .get_original_url()
          .unwrap_or(request.get_hyper_request().uri())
          .path(),
        request.get_error_status_code().map(|x| x.as_u16()),
      );

      let original_url = format!(
        "{}{}",
        hyper_request.uri().path(),
        match hyper_request.uri().query() {
          Some(query) => format!("?{query}"),
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
          if let Some(wwwroot) = config["wwwroot"].as_str() {
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
        if config["enableRewriteLogging"].as_bool() == Some(true) {
          error_logger
            .log(&format!(
              "URL rewritten from \"{original_url}\" to \"{rewritten_url}\""
            ))
            .await;
        }
        let (hyper_request, auth_user, _, error_status_code) = request.into_parts();
        let (mut parts, body) = hyper_request.into_parts();
        let original_url = parts.uri.clone();
        let mut url_parts = parts.uri.into_parts();
        url_parts.path_and_query = Some(rewritten_url.parse()?);
        parts.uri = hyper::Uri::from_parts(url_parts)?;
        let hyper_request = Request::from_parts(parts, body);
        let request = RequestData::new(
          hyper_request,
          auth_user,
          Some(original_url),
          error_status_code,
        );
        Ok(ResponseData::builder(request).build())
      }
    })
    .await
  }

  async fn proxy_request_handler(
    &mut self,
    request: RequestData,
    _config: &ServerConfig,
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

  async fn connect_proxy_request_handler(
    &mut self,
    _upgraded_request: HyperUpgraded,
    _connect_address: &str,
    _config: &ServerConfig,
    _socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    Ok(())
  }

  fn does_connect_proxy_requests(&mut self) -> bool {
    false
  }

  async fn websocket_request_handler(
    &mut self,
    _websocket: HyperWebsocket,
    _uri: &hyper::Uri,
    _headers: &hyper::HeaderMap,
    _config: &ServerConfig,
    _socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    Ok(())
  }

  fn does_websocket_requests(&mut self, _config: &ServerConfig, _socket_data: &SocketData) -> bool {
    false
  }
}
