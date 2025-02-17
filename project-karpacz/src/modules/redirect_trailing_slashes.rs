use std::error::Error;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use http_body_util::{BodyExt, Empty};
use hyper::{header, Response, StatusCode};
use project_karpacz_common::WithRuntime;
use project_karpacz_common::{
  ErrorLogger, HyperResponse, RequestData, ResponseData, ServerConfigRoot, ServerModule,
  ServerModuleHandlers, SocketData,
};
use tokio::fs;
use tokio::runtime::Handle;
use tokio::sync::RwLock;

use crate::project_karpacz_util::ttl_cache::TtlCache;

pub fn server_module_init(
) -> Result<Box<dyn ServerModule + Send + Sync>, Box<dyn Error + Send + Sync>> {
  let cache = Arc::new(RwLock::new(TtlCache::new(Duration::new(1, 0))));
  Ok(Box::new(RedirectTrailingSlashesModule::new(cache)))
}

struct RedirectTrailingSlashesModule {
  cache: Arc<RwLock<TtlCache<String, bool>>>,
}

impl RedirectTrailingSlashesModule {
  fn new(cache: Arc<RwLock<TtlCache<String, bool>>>) -> Self {
    RedirectTrailingSlashesModule { cache }
  }
}

impl ServerModule for RedirectTrailingSlashesModule {
  fn get_handlers(&self, handle: Handle) -> Box<dyn ServerModuleHandlers + Send> {
    Box::new(RedirectTrailingSlashesModuleHandlers {
      cache: self.cache.clone(),
      handle,
    })
  }
}

struct RedirectTrailingSlashesModuleHandlers {
  cache: Arc<RwLock<TtlCache<String, bool>>>,
  handle: Handle,
}

#[async_trait]
impl ServerModuleHandlers for RedirectTrailingSlashesModuleHandlers {
  async fn request_handler(
    &mut self,
    request: RequestData,
    config: &ServerConfigRoot,
    _socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    WithRuntime::new(self.handle.clone(), async move {
      if config.get("disableTrailingSlashRedirects").as_bool() != Some(true) {
        if let Some(wwwroot) = config.get("wwwroot").as_str() {
          let hyper_request = request.get_hyper_request();

          let request_path = hyper_request.uri().path();
          let request_query = hyper_request.uri().query();
          let mut request_path_bytes = request_path.bytes();
          if request_path_bytes.len() < 1 || request_path_bytes.nth(0) != Some(b'/') {
            return Ok(
              ResponseData::builder(request)
                .status(StatusCode::BAD_REQUEST)
                .build(),
            );
          }

          match request_path_bytes.last() {
            Some(b'/') | None => {
              return Ok(ResponseData::builder(request).build());
            }
            _ => {
              let cache_key = format!(
                "{}{}{}",
                match config.get("ip").as_str() {
                  Some(ip) => format!("{}-", ip),
                  None => String::from(""),
                },
                match config.get("domain").as_str() {
                  Some(domain) => format!("{}-", domain),
                  None => String::from(""),
                },
                request_path
              );

              let read_rwlock = self.cache.read().await;
              if let Some(is_directory) = read_rwlock.get(&cache_key) {
                drop(read_rwlock);
                if is_directory {
                  let new_request_uri = format!(
                    "{}/{}",
                    request_path,
                    match request_query {
                      Some(query) => format!("?{}", query),
                      None => String::from(""),
                    }
                  );
                  return Ok(
                    ResponseData::builder(request)
                      .response(
                        Response::builder()
                          .status(StatusCode::MOVED_PERMANENTLY)
                          .header(header::LOCATION, new_request_uri)
                          .body(Empty::new().map_err(|e| match e {}).boxed())?,
                      )
                      .build(),
                  );
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
                    return Ok(
                      ResponseData::builder(request)
                        .status(StatusCode::BAD_REQUEST)
                        .build(),
                    );
                  }
                };

                let joined_pathbuf = path.join(decoded_relative_path);

                match fs::metadata(joined_pathbuf).await {
                  Ok(metadata) => {
                    let is_directory = metadata.is_dir();
                    let mut write_rwlock = self.cache.write().await;
                    write_rwlock.cleanup();
                    write_rwlock.insert(cache_key, is_directory);
                    if is_directory {
                      let new_request_uri = format!(
                        "{}/{}",
                        request_path,
                        match request_query {
                          Some(query) => format!("?{}", query),
                          None => String::from(""),
                        }
                      );
                      return Ok(
                        ResponseData::builder(request)
                          .response(
                            Response::builder()
                              .status(StatusCode::MOVED_PERMANENTLY)
                              .header(header::LOCATION, new_request_uri)
                              .body(Empty::new().map_err(|e| match e {}).boxed())?,
                          )
                          .build(),
                      );
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
      Ok(ResponseData::builder(request).build())
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
