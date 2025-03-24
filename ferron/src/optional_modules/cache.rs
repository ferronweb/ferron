use std::collections::HashMap;
use std::error::Error;
use std::hash::RandomState;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::ferron_common::{
  ErrorLogger, HyperUpgraded, RequestData, ResponseData, ServerConfig, ServerModule,
  ServerModuleHandlers, SocketData,
};
use crate::ferron_common::{HyperResponse, WithRuntime};
use async_trait::async_trait;
use cache_control::{Cachability, CacheControl};
use futures_util::{StreamExt, TryStreamExt};
use hashlink::LinkedHashMap;
use http_body_util::{BodyExt, Full, StreamBody};
use hyper::body::{Bytes, Frame};
use hyper::header::HeaderValue;
use hyper::{header, HeaderMap, Method, Response, StatusCode};
use hyper_tungstenite::HyperWebsocket;
use itertools::Itertools;
use tokio::runtime::Handle;
use tokio::sync::RwLock;

const CACHE_HEADER_NAME: &str = "X-Ferron-Cache";
const DEFAULT_MAX_AGE: u64 = 300;

pub fn server_module_init(
  config: &ServerConfig,
) -> Result<Box<dyn ServerModule + Send + Sync>, Box<dyn Error + Send + Sync>> {
  let maximum_cache_entries = config["global"]["maximumCacheEntries"]
    .as_i64()
    .map(|v| v as usize);

  Ok(Box::new(CacheModule::new(
    Arc::new(RwLock::new(LinkedHashMap::with_hasher(RandomState::new()))),
    Arc::new(RwLock::new(HashMap::new())),
    maximum_cache_entries,
  )))
}

#[allow(clippy::type_complexity)]
struct CacheModule {
  cache: Arc<
    RwLock<
      LinkedHashMap<
        String,
        (
          StatusCode,
          HeaderMap,
          Vec<u8>,
          Instant,
          Option<CacheControl>,
        ),
        RandomState,
      >,
    >,
  >,
  vary_cache: Arc<RwLock<HashMap<String, Vec<String>>>>,
  maximum_cache_entries: Option<usize>,
}

impl CacheModule {
  #[allow(clippy::type_complexity)]
  fn new(
    cache: Arc<
      RwLock<
        LinkedHashMap<
          String,
          (
            StatusCode,
            HeaderMap,
            Vec<u8>,
            Instant,
            Option<CacheControl>,
          ),
          RandomState,
        >,
      >,
    >,
    vary_cache: Arc<RwLock<HashMap<String, Vec<String>>>>,
    maximum_cache_entries: Option<usize>,
  ) -> Self {
    CacheModule {
      cache,
      vary_cache,
      maximum_cache_entries,
    }
  }
}

impl ServerModule for CacheModule {
  fn get_handlers(&self, handle: Handle) -> Box<dyn ServerModuleHandlers + Send> {
    Box::new(CacheModuleHandlers {
      cache: self.cache.clone(),
      vary_cache: self.vary_cache.clone(),
      maximum_cache_entries: self.maximum_cache_entries,
      cache_vary_headers_configured: Vec::new(),
      cache_ignore_headers_configured: Vec::new(),
      maximum_cached_response_size: None,
      cache_key: None,
      request_headers: HeaderMap::new(),
      has_authorization: false,
      cached: false,
      no_store: false,
      handle,
    })
  }
}

#[allow(clippy::type_complexity)]
struct CacheModuleHandlers {
  handle: Handle,
  cache: Arc<
    RwLock<
      LinkedHashMap<
        String,
        (
          StatusCode,
          HeaderMap,
          Vec<u8>,
          Instant,
          Option<CacheControl>,
        ),
        RandomState,
      >,
    >,
  >,
  vary_cache: Arc<RwLock<HashMap<String, Vec<String>>>>,
  maximum_cache_entries: Option<usize>,
  cache_vary_headers_configured: Vec<String>,
  cache_ignore_headers_configured: Vec<String>,
  maximum_cached_response_size: Option<u64>,
  cache_key: Option<String>,
  request_headers: HeaderMap<HeaderValue>,
  has_authorization: bool,
  cached: bool,
  no_store: bool,
}

#[async_trait]
impl ServerModuleHandlers for CacheModuleHandlers {
  async fn request_handler(
    &mut self,
    request: RequestData,
    config: &ServerConfig,
    socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    WithRuntime::new(self.handle.clone(), async move {
      self.cache_vary_headers_configured = match config["cacheVaryHeaders"].as_vec() {
        Some(vector) => {
          let mut new_vector = Vec::new();
          for yaml_value in vector.iter() {
            if let Some(str_value) = yaml_value.as_str() {
              new_vector.push(str_value.to_string());
            }
          }
          new_vector
        }
        None => Vec::new(),
      };
      self.cache_ignore_headers_configured = match config["cacheIgnoreHeaders"].as_vec() {
        Some(vector) => {
          let mut new_vector = Vec::new();
          for yaml_value in vector.iter() {
            if let Some(str_value) = yaml_value.as_str() {
              new_vector.push(str_value.to_string());
            }
          }
          new_vector
        }
        None => Vec::new(),
      };
      self.maximum_cached_response_size = config["maximumCachedResponseSize"]
        .as_i64()
        .map(|f| f as u64);

      let hyper_request = request.get_hyper_request();
      let cache_key = format!(
        "{} {}{}{}{}",
        hyper_request.method().as_str(),
        match socket_data.encrypted {
          false => "http://",
          true => "https://",
        },
        match hyper_request.headers().get(header::HOST) {
          Some(host) => String::from_utf8_lossy(host.as_bytes()).into_owned(),
          None => "".to_string(),
        },
        hyper_request.uri().path(),
        match hyper_request.uri().query() {
          Some(query) => format!("?{}", query),
          None => "".to_string(),
        }
      );

      let request_cache_control = match hyper_request.headers().get(header::CACHE_CONTROL) {
        Some(value) => CacheControl::from_value(&String::from_utf8_lossy(value.as_bytes())),
        None => None,
      };

      let mut no_store = false;
      let mut no_cache = false;

      if let Some(request_cache_control) = request_cache_control {
        no_store = request_cache_control.no_store;
        if let Some(cachability) = request_cache_control.cachability {
          if cachability == Cachability::NoCache {
            no_cache = true;
          }
        }
      }

      match hyper_request.method() {
        &Method::GET | &Method::HEAD => (),
        _ => {
          no_store = true;
        }
      };

      if no_store {
        self.no_store = true;
        return Ok(ResponseData::builder(request).build());
      }

      if !no_cache {
        let rwlock_read = self.vary_cache.read().await;
        let processed_vary = rwlock_read.get(&cache_key);
        if let Some(processed_vary) = processed_vary {
          let cache_key_with_vary = format!(
            "{}\n{}",
            &cache_key,
            processed_vary
              .iter()
              .map(|header_name| {
                match hyper_request.headers().get(header_name) {
                  Some(header_value) => format!(
                    "{}: {}",
                    header_name,
                    String::from_utf8_lossy(header_value.as_bytes()).into_owned()
                  ),
                  None => "".to_string(),
                }
              })
              .collect::<Vec<String>>()
              .join("\n")
          );

          drop(rwlock_read);

          let rwlock_read = self.cache.read().await;
          let cached_entry_option = rwlock_read.get(&cache_key_with_vary);

          if let Some((status_code, headers, body, timestamp, response_cache_control)) =
            cached_entry_option
          {
            let max_age = match response_cache_control {
              Some(response_cache_control) => match response_cache_control.s_max_age {
                Some(s_max_age) => Some(s_max_age),
                None => response_cache_control.max_age,
              },
              None => None,
            };

            let mut cached = true;

            if timestamp.elapsed() > max_age.unwrap_or(Duration::from_secs(DEFAULT_MAX_AGE)) {
              cached = false;
            }

            if cached {
              self.cached = true;
              let mut hyper_response_builder = Response::builder().status(status_code);
              for (header_name, header_value) in headers.iter() {
                hyper_response_builder = hyper_response_builder.header(header_name, header_value);
              }
              let hyper_response = hyper_response_builder.body(
                Full::new(Bytes::from(body.clone()))
                  .map_err(|e| match e {})
                  .boxed(),
              )?;
              return Ok(
                ResponseData::builder(request)
                  .response(hyper_response)
                  .build(),
              );
            } else {
              drop(rwlock_read);
            }
          }
        } else {
          drop(rwlock_read);
        }
      }

      self.request_headers = hyper_request.headers().clone();
      self.cache_key = Some(cache_key);
      self.has_authorization = hyper_request.headers().contains_key(header::AUTHORIZATION);

      Ok(ResponseData::builder(request).build())
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
    mut response: HyperResponse,
  ) -> Result<HyperResponse, Box<dyn Error + Send + Sync>> {
    WithRuntime::new(self.handle.clone(), async move {
      if self.no_store {
        response
          .headers_mut()
          .insert(CACHE_HEADER_NAME, HeaderValue::from_str("BYPASS")?);
        Ok(response)
      } else if self.cached {
        response
          .headers_mut()
          .insert(CACHE_HEADER_NAME, HeaderValue::from_str("HIT")?);
        Ok(response)
      } else if let Some(cache_key) = &self.cache_key {
        let (mut response_parts, mut response_body) = response.into_parts();
        let response_cache_control = match response_parts.headers.get(header::CACHE_CONTROL) {
          Some(value) => CacheControl::from_value(&String::from_utf8_lossy(value.as_bytes())),
          None => None,
        };

        let should_cache_response = match &response_cache_control {
          Some(response_cache_control) => {
            let is_private = response_cache_control.cachability == Some(Cachability::Private);
            let is_public = response_cache_control.cachability == Some(Cachability::Public);

            !response_cache_control.no_store
              && !is_private
              && (is_public
                || (!self.has_authorization
                  && (response_cache_control.max_age.is_some()
                    || response_cache_control.s_max_age.is_some())))
          }
          None => false,
        };

        if should_cache_response {
          let mut response_body_buffer = Vec::new();
          let mut maximum_cached_response_size_exceeded = false;

          while let Some(frame) = response_body.frame().await {
            let frame_unwrapped = frame?;
            if frame_unwrapped.is_data() {
              if let Some(bytes) = frame_unwrapped.data_ref() {
                response_body_buffer.extend_from_slice(bytes);
                if let Some(maximum_cached_response_size) = self.maximum_cached_response_size {
                  if response_body_buffer.len() as u64 > maximum_cached_response_size {
                    maximum_cached_response_size_exceeded = true;
                    break;
                  }
                }
              }
            }
          }

          if maximum_cached_response_size_exceeded {
            let cached_stream =
              futures_util::stream::once(async move { Ok(Bytes::from(response_body_buffer)) });
            let response_stream = response_body.into_data_stream();
            let chained_stream = cached_stream.chain(response_stream);
            let stream_body = StreamBody::new(chained_stream.map_ok(Frame::data));
            let response_body = BodyExt::boxed(stream_body);
            response_parts
              .headers
              .insert(CACHE_HEADER_NAME, HeaderValue::from_str("MISS")?);
            let response = Response::from_parts(response_parts, response_body);
            Ok(response)
          } else {
            let mut response_vary = match response_parts.headers.get(header::VARY) {
              Some(value) => String::from_utf8_lossy(value.as_bytes())
                .split(",")
                .map(|s| s.trim().to_owned())
                .collect(),
              None => Vec::new(),
            };

            let mut processed_vary_orig = self.cache_vary_headers_configured.clone();
            processed_vary_orig.append(&mut response_vary);

            let processed_vary = processed_vary_orig
              .iter()
              .unique()
              .map(|s| s.to_owned())
              .collect::<Vec<String>>();

            if !processed_vary.contains(&"*".to_string()) {
              let cache_key_with_vary = format!(
                "{}\n{}",
                &cache_key,
                processed_vary
                  .iter()
                  .map(|header_name| {
                    match self.request_headers.get(header_name) {
                      Some(header_value) => format!(
                        "{}: {}",
                        header_name,
                        String::from_utf8_lossy(header_value.as_bytes()).into_owned()
                      ),
                      None => "".to_string(),
                    }
                  })
                  .collect::<Vec<String>>()
                  .join("\n")
              );

              let mut rwlock_write = self.vary_cache.write().await;
              rwlock_write.insert(cache_key.clone(), processed_vary);
              drop(rwlock_write);

              let mut written_headers = response_parts.headers.clone();
              for header in self.cache_ignore_headers_configured.iter() {
                while written_headers.remove(header).is_some() {}
              }

              let mut rwlock_write = self.cache.write().await;
              rwlock_write.retain(|_, (_, _, _, timestamp, response_cache_control)| {
                let max_age = match response_cache_control {
                  Some(response_cache_control) => match response_cache_control.s_max_age {
                    Some(s_max_age) => Some(s_max_age),
                    None => response_cache_control.max_age,
                  },
                  None => None,
                };

                timestamp.elapsed() <= max_age.unwrap_or(Duration::from_secs(DEFAULT_MAX_AGE))
              });

              if let Some(maximum_cache_entries) = self.maximum_cache_entries {
                // Remove a value at the front of the list
                while rwlock_write.len() > 0 && rwlock_write.len() >= maximum_cache_entries {
                  rwlock_write.pop_front();
                }
              }

              // This inserts a value at the back of the list
              rwlock_write.insert(
                cache_key_with_vary,
                (
                  response_parts.status,
                  written_headers,
                  response_body_buffer.clone(),
                  Instant::now(),
                  response_cache_control,
                ),
              );
              drop(rwlock_write);
            }

            let cached_stream =
              futures_util::stream::once(async move { Ok(Bytes::from(response_body_buffer)) });
            let stream_body = StreamBody::new(cached_stream.map_ok(Frame::data));
            let response_body = BodyExt::boxed(stream_body);
            response_parts
              .headers
              .insert(CACHE_HEADER_NAME, HeaderValue::from_str("MISS")?);
            let response = Response::from_parts(response_parts, response_body);
            Ok(response)
          }
        } else {
          response_parts
            .headers
            .insert(CACHE_HEADER_NAME, HeaderValue::from_str("MISS")?);
          let response = Response::from_parts(response_parts, response_body);
          Ok(response)
        }
      } else {
        Ok(response)
      }
    })
    .await
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
