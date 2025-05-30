use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use cache_control::{Cachability, CacheControl};
use futures_util::stream::{StreamExt, TryStreamExt};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full, StreamBody};
use hyper::body::Frame;
use hyper::header::{self, HeaderValue};
use hyper::{HeaderMap, Method, Request, Response, StatusCode};
use itertools::Itertools;
use monoio::time::Instant;
use tokio::sync::RwLock;

use crate::logging::ErrorLogger;
use crate::modules::{Module, ModuleHandlers, ModuleLoader, ResponseData, SocketData};
use crate::util::get_entries_for_validation;
use crate::util::AtomicGenericCache;
use crate::{config::ServerConfiguration, util::ModuleCache};
use crate::{get_entry, get_value, get_values};

const CACHE_HEADER_NAME: &str = "X-Ferron-Cache";
const DEFAULT_MAX_AGE: u64 = 300;
const DEFAULT_MAX_CACHE_ENTRIES: usize = 128;

/// A cache module loader
#[allow(clippy::type_complexity)]
pub struct CacheModuleLoader {
  module_cache: ModuleCache<CacheModule>,
}

impl CacheModuleLoader {
  /// Creates a new module loader
  pub fn new() -> Self {
    Self {
      module_cache: ModuleCache::new(vec![]),
    }
  }
}

impl ModuleLoader for CacheModuleLoader {
  fn load_module(
    &mut self,
    config: &ServerConfiguration,
    global_config: Option<&ServerConfiguration>,
  ) -> Result<Arc<dyn Module + Send + Sync>, Box<dyn Error + Send + Sync>> {
    Ok(self.module_cache.get_or(config, |_| {
      Ok(Arc::new(CacheModule {
        cache: AtomicGenericCache::new(
          1024,
          global_config
            .and_then(|c| get_entry!("cache_max_entries", c))
            .and_then(|e| e.values.first())
            .and_then(|v| v.as_i128())
            .map(|v| v as usize)
            .unwrap_or(DEFAULT_MAX_CACHE_ENTRIES),
        ),
        vary_cache: Arc::new(RwLock::new(HashMap::new())),
      }))
    })?)
  }

  fn get_requirements(&self) -> Vec<&'static str> {
    vec!["cache"]
  }

  fn validate_configuration(
    &self,
    config: &ServerConfiguration,
    used_properties: &mut HashSet<String>,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    if let Some(entries) = get_entries_for_validation!("cache", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `cache` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!("Invalid cache enabling option"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("cache_max_entries", config, used_properties)
    {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `cache_max_entries` configuration property must have exactly one value"
          ))?
        } else if (!entry.values[0].is_integer())
          || entry.values[0].as_i128().is_some_and(|v| v < 0)
        {
          Err(anyhow::anyhow!(
            "Invalid maximum cache entries configuration"
          ))?
        }
      }
    };

    if let Some(entries) =
      get_entries_for_validation!("cache_max_response_size", config, used_properties)
    {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `cache_max_response_size` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_integer() || entry.values[0].as_i128().is_some_and(|v| v < 0)
        {
          Err(anyhow::anyhow!(
            "Invalid maximum cache response size configuration"
          ))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("cache_vary", config, used_properties) {
      for entry in &entries.inner {
        for value in &entry.values {
          if !value.is_string() {
            Err(anyhow::anyhow!(
              "Invalid varying request headers configuration"
            ))?
          }
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("cache_ignore", config, used_properties) {
      for entry in &entries.inner {
        for value in &entry.values {
          if !value.is_string() {
            Err(anyhow::anyhow!(
              "Invalid ignored cache response headers configuration"
            ))?
          }
        }
      }
    }

    Ok(())
  }
}

/// A cache module
#[allow(clippy::type_complexity)]
struct CacheModule {
  cache: Arc<
    AtomicGenericCache<
      String,
      Option<(
        StatusCode,
        HeaderMap,
        Vec<u8>,
        Instant,
        Option<Arc<CacheControl>>,
      )>,
    >,
  >,
  vary_cache: Arc<RwLock<HashMap<String, Vec<String>>>>,
}

impl Module for CacheModule {
  fn get_module_handlers(&self) -> Box<dyn ModuleHandlers> {
    Box::new(CacheModuleHandlers {
      cache: self.cache.clone(),
      vary_cache: self.vary_cache.clone(),
      cache_vary_headers_configured: Vec::new(),
      cache_ignore_headers_configured: Vec::new(),
      maximum_cached_response_size: None,
      cache_key: None,
      request_headers: HeaderMap::new(),
      has_authorization: false,
      cached: false,
      no_store: false,
    })
  }
}

/// Handlers for the cache module
#[allow(clippy::type_complexity)]
struct CacheModuleHandlers {
  cache: Arc<
    AtomicGenericCache<
      String,
      Option<(
        StatusCode,
        HeaderMap,
        Vec<u8>,
        Instant,
        Option<Arc<CacheControl>>,
      )>,
    >,
  >,
  vary_cache: Arc<RwLock<HashMap<String, Vec<String>>>>,
  cache_vary_headers_configured: Vec<String>,
  cache_ignore_headers_configured: Vec<String>,
  maximum_cached_response_size: Option<u64>,
  cache_key: Option<String>,
  request_headers: HeaderMap<HeaderValue>,
  has_authorization: bool,
  cached: bool,
  no_store: bool,
}

#[async_trait(?Send)]
impl ModuleHandlers for CacheModuleHandlers {
  async fn request_handler(
    &mut self,
    request: Request<BoxBody<Bytes, std::io::Error>>,
    config: &ServerConfiguration,
    socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    self.cache_vary_headers_configured = get_values!("cache_vary", config)
      .into_iter()
      .filter_map(|v| v.as_str().map(|v| v.to_string()))
      .collect::<Vec<_>>();
    self.cache_ignore_headers_configured = get_values!("cache_ignore", config)
      .into_iter()
      .filter_map(|v| v.as_str().map(|v| v.to_string()))
      .collect::<Vec<_>>();
    self.maximum_cached_response_size =
      get_value!("cache_max_response_size", config).and_then(|v| v.as_i128().map(|f| f as u64));

    let cache_key = format!(
      "{} {}{}{}{}",
      request.method().as_str(),
      match socket_data.encrypted {
        false => "http://",
        true => "https://",
      },
      match request.headers().get(header::HOST) {
        Some(host) => String::from_utf8_lossy(host.as_bytes()).into_owned(),
        None => "".to_string(),
      },
      request.uri().path(),
      match request.uri().query() {
        Some(query) => format!("?{}", query),
        None => "".to_string(),
      }
    );

    let request_cache_control = match request.headers().get(header::CACHE_CONTROL) {
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

    match request.method() {
      &Method::GET | &Method::HEAD => (),
      _ => {
        no_store = true;
      }
    };

    // If the request is an upgrade request, mark it as no-store
    if request.headers().get(header::UPGRADE).is_some() {
      no_store = true;
    }

    if no_store {
      self.no_store = true;
      return Ok(ResponseData {
        request: Some(request),
        response: None,
        response_status: None,
        response_headers: None,
        new_remote_address: None,
      });
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
              match request.headers().get(header_name) {
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

        let cached_entry_option = self.cache.get(&cache_key_with_vary);

        if let Some(Some((status_code, headers, body, timestamp, response_cache_control))) =
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
            return Ok(ResponseData {
              request: Some(request),
              response: Some(hyper_response),
              response_status: None,
              response_headers: None,
              new_remote_address: None,
            });
          }
        }
      }
    }

    self.request_headers = request.headers().clone();
    self.cache_key = Some(cache_key);
    self.has_authorization = request.headers().contains_key(header::AUTHORIZATION);

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
    mut response: Response<BoxBody<Bytes, std::io::Error>>,
  ) -> Result<Response<BoxBody<Bytes, std::io::Error>>, Box<dyn Error>> {
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

            self.cache.retain(|value, _, _| {
              if let Some((_, _, _, timestamp, response_cache_control)) = value {
                let max_age = match response_cache_control {
                  Some(response_cache_control) => match response_cache_control.s_max_age {
                    Some(s_max_age) => Some(s_max_age),
                    None => response_cache_control.max_age,
                  },
                  None => None,
                };

                timestamp.elapsed() <= max_age.unwrap_or(Duration::from_secs(DEFAULT_MAX_AGE))
              } else {
                false
              }
            });

            // This inserts a value at the back of the list
            _ = self.cache.insert(
              cache_key_with_vary,
              Some((
                response_parts.status,
                written_headers,
                response_body_buffer.clone(),
                Instant::now(),
                response_cache_control.map(Arc::new),
              )),
            );
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
  }
}
