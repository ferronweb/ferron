use std::collections::HashSet;
use std::error::Error;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use ahash::RandomState;
use async_trait::async_trait;
use bytes::Bytes;
use cache_control::{Cachability, CacheControl};
use futures_util::stream::{StreamExt, TryStreamExt};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full, StreamBody};
use hyper::body::Frame;
use hyper::header::{self, HeaderName, HeaderValue};
use hyper::{HeaderMap, Method, Request, Response, StatusCode};
use smallvec::SmallVec;

#[cfg(feature = "runtime-monoio")]
use monoio::time::Instant;
#[cfg(feature = "runtime-tokio")]
use tokio::time::Instant;

use crate::logging::ErrorLogger;
use crate::modules::{Module, ModuleHandlers, ModuleLoader, ResponseData, SocketData};
use crate::util::get_entries_for_validation;
use crate::util::AtomicGenericCache;
use crate::{config::ServerConfiguration, util::ModuleCache};
use crate::{get_entry, get_value, get_values};

// Constants for optimization
const CACHE_HEADER_NAME: &str = "X-Ferron-Cache";
const DEFAULT_MAX_AGE: u64 = 300;
const INITIAL_CACHE_KEY_CAPACITY: usize = 256;
const INITIAL_RESPONSE_BUFFER_CAPACITY: usize = 16384; // Increased for better chunking
const MAX_SMALL_HEADER_COUNT: usize = 16;

// Pre-computed header names for faster lookups
static CACHE_CONTROL_HEADER: LazyLock<HeaderName> = LazyLock::new(|| header::CACHE_CONTROL);
static HOST_HEADER: LazyLock<HeaderName> = LazyLock::new(|| header::HOST);
static AUTHORIZATION_HEADER: LazyLock<HeaderName> = LazyLock::new(|| header::AUTHORIZATION);
static UPGRADE_HEADER: LazyLock<HeaderName> = LazyLock::new(|| header::UPGRADE);
static VARY_HEADER: LazyLock<HeaderName> = LazyLock::new(|| header::VARY);

// Cache header values to avoid allocations
static CACHE_HIT: LazyLock<HeaderValue> = LazyLock::new(|| HeaderValue::from_static("HIT"));
static CACHE_MISS: LazyLock<HeaderValue> = LazyLock::new(|| HeaderValue::from_static("MISS"));
static CACHE_BYPASS: LazyLock<HeaderValue> = LazyLock::new(|| HeaderValue::from_static("BYPASS"));

// Protocol prefixes
const HTTP_PREFIX: &str = "http://";
const HTTPS_PREFIX: &str = "https://";

type HeaderList = SmallVec<[String; MAX_SMALL_HEADER_COUNT]>;
type CacheEntry = (
  StatusCode,
  HeaderMap,
  Vec<u8>,
  Instant,
  Option<Arc<CacheControl>>,
);

/// Optimized cache decision with bitflags for faster comparisons
#[derive(Debug, Clone, Copy)]
struct CacheDecision {
  flags: u8,
}

impl CacheDecision {
  const NO_STORE: u8 = 1;
  const NO_CACHE: u8 = 2;
  const CACHEABLE_METHOD: u8 = 4;

  fn new() -> Self {
    Self { flags: 0 }
  }

  fn set_no_store(&mut self) {
    self.flags |= Self::NO_STORE;
  }

  fn set_no_cache(&mut self) {
    self.flags |= Self::NO_CACHE;
  }

  fn set_cacheable_method(&mut self) {
    self.flags |= Self::CACHEABLE_METHOD;
  }

  fn no_store(&self) -> bool {
    self.flags & Self::NO_STORE != 0
  }

  fn no_cache(&self) -> bool {
    self.flags & Self::NO_CACHE != 0
  }

  fn from_request(request: &Request<BoxBody<Bytes, std::io::Error>>) -> Self {
    let mut decision = Self::new();

    // Check method first (most likely to eliminate caching)
    match request.method() {
      &Method::GET | &Method::HEAD => decision.set_cacheable_method(),
      _ => decision.set_no_store(),
    }

    // Early return if method is not cacheable
    if decision.no_store() {
      return decision;
    }

    // Check for upgrade header (common case)
    if request.headers().contains_key(&*UPGRADE_HEADER) {
      decision.set_no_store();
      return decision;
    }

    // Parse cache control only if needed
    if let Some(cache_control_header) = request.headers().get(&*CACHE_CONTROL_HEADER) {
      if let Ok(header_str) = cache_control_header.to_str() {
        if let Some(cache_control) = CacheControl::from_value(header_str) {
          if cache_control.no_store {
            decision.set_no_store();
          }
          if let Some(Cachability::NoCache) = cache_control.cachability {
            decision.set_no_cache();
          }
        }
      }
    }

    decision
  }
}

/// High-performance response body handler with memory pooling
struct ResponseBodyHandler {
  max_size: Option<u64>,
  buffer: Vec<u8>,
  total_size: usize,
}

impl ResponseBodyHandler {
  fn new(max_size: Option<u64>) -> Self {
    Self {
      max_size,
      buffer: Vec::with_capacity(INITIAL_RESPONSE_BUFFER_CAPACITY),
      total_size: 0,
    }
  }

  #[inline]
  async fn collect_body(
    &mut self,
    body: &mut BoxBody<Bytes, std::io::Error>,
  ) -> Result<bool, Box<dyn Error>> {
    while let Some(frame) = body.frame().await {
      let frame = frame?;
      if let Some(bytes) = frame.data_ref() {
        let chunk_size = bytes.len();

        // Check size limit before allocation
        if let Some(max_size) = self.max_size {
          if self.total_size + chunk_size > max_size as usize {
            return Ok(false);
          }
        }

        // Reserve capacity in larger chunks to reduce allocations
        if self.buffer.capacity() < self.total_size + chunk_size {
          let new_capacity = (self.total_size + chunk_size).next_power_of_two();
          self.buffer.reserve(new_capacity - self.buffer.capacity());
        }

        self.buffer.extend_from_slice(bytes);
        self.total_size += chunk_size;
      }
    }
    Ok(true)
  }

  fn into_bytes(mut self) -> Vec<u8> {
    self.buffer.shrink_to_fit(); // Reclaim unused capacity
    self.buffer
  }
}

/// Optimized vary key builder with pre-allocated buffer
struct VaryKeyBuilder {
  buffer: String,
}

impl VaryKeyBuilder {
  fn new() -> Self {
    Self {
      buffer: String::with_capacity(INITIAL_CACHE_KEY_CAPACITY * 2),
    }
  }

  fn build(
    &mut self,
    base_key: &str,
    vary_headers: &[String],
    request_headers: &HeaderMap,
  ) -> &str {
    self.buffer.clear();
    self.buffer.push_str(base_key);
    self.buffer.push('\n');

    for (i, header_name) in vary_headers.iter().enumerate() {
      if i > 0 {
        self.buffer.push('\n');
      }

      self.buffer.push_str(header_name);
      self.buffer.push_str(": ");

      if let Some(header_value) = request_headers.get(header_name) {
        if let Ok(str_val) = header_value.to_str() {
          self.buffer.push_str(str_val);
        } else {
          // Fallback to lossy conversion
          self
            .buffer
            .push_str(&String::from_utf8_lossy(header_value.as_bytes()));
        }
      }
    }

    &self.buffer
  }
}

thread_local! {
    /// Thread-local storage for frequently used objects
    static VARY_KEY_BUILDER: std::cell::RefCell<VaryKeyBuilder> =
        std::cell::RefCell::new(VaryKeyBuilder::new());
}

/// Fast cache key generation with minimal allocations
#[inline]
fn extract_uri_parts(
  request: &Request<BoxBody<Bytes, std::io::Error>>,
) -> (String, String, Option<String>) {
  let host = request
    .headers()
    .get(&*HOST_HEADER)
    .and_then(|h| h.to_str().ok())
    .unwrap_or("")
    .to_string();

  let path = request.uri().path().to_string();
  let query = request.uri().query().map(String::from);

  (host, path, query)
}

/// Optimized cache key building
#[inline]
fn build_cache_key(
  method: &Method,
  encrypted: bool,
  host: &str,
  path: &str,
  query: Option<&str>,
) -> String {
  let estimated_size = method.as_str().len()
    + if encrypted {
      HTTPS_PREFIX.len()
    } else {
      HTTP_PREFIX.len()
    }
    + host.len()
    + path.len()
    + query.map_or(0, |q| q.len() + 1);

  let mut cache_key = String::with_capacity(estimated_size.max(INITIAL_CACHE_KEY_CAPACITY));

  cache_key.push_str(method.as_str());
  cache_key.push(' ');
  cache_key.push_str(if encrypted { HTTPS_PREFIX } else { HTTP_PREFIX });
  cache_key.push_str(host);
  cache_key.push_str(path);

  if let Some(query) = query {
    cache_key.push('?');
    cache_key.push_str(query);
  }

  cache_key
}

/// A cache module loader with optimized initialization
#[allow(clippy::type_complexity)]
pub struct CacheModuleLoader {
  module_cache: ModuleCache<MemCacheModule>,
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
    Ok(self.module_cache.get_or::<_, anyhow::Error>(config, |_| {
      let maximum_cache_entries = global_config
        .and_then(|c| get_entry!("cache_max_entries", c))
        .and_then(|e| e.values.first())
        .and_then(|v| v.as_i128())
        .map(|v| v as usize);

      // Use optimized cache size calculation
      let cache_size = maximum_cache_entries.map_or(2048, |e| e.min(8192).max(512));
      let max_entries = maximum_cache_entries.unwrap_or(0);

      Ok(Arc::new(MemCacheModule {
        cache: AtomicGenericCache::new(cache_size, max_entries),
        vary_cache: Arc::new(papaya::HashMap::with_capacity_and_hasher(
          cache_size / 4,
          RandomState::new(),
        )),
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
    // Fixed validation - back to the original approach for clarity
    if let Some(entries) = get_entries_for_validation!("cache", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          return Err(
            anyhow::anyhow!("The `cache` configuration property must have exactly one value")
              .into(),
          );
        } else if !entry.values[0].is_bool() {
          return Err(anyhow::anyhow!("Invalid cache enabling option").into());
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("cache_max_entries", config, used_properties)
    {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          return Err(
            anyhow::anyhow!(
              "The `cache_max_entries` configuration property must have exactly one value"
            )
            .into(),
          );
        } else if (!entry.values[0].is_integer() && !entry.values[0].is_null())
          || entry.values[0].as_i128().is_some_and(|v| v < 0)
        {
          return Err(anyhow::anyhow!("Invalid maximum cache entries configuration").into());
        }
      }
    }

    if let Some(entries) =
      get_entries_for_validation!("cache_max_response_size", config, used_properties)
    {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          return Err(
            anyhow::anyhow!(
              "The `cache_max_response_size` configuration property must have exactly one value"
            )
            .into(),
          );
        } else if !entry.values[0].is_integer() || entry.values[0].as_i128().is_some_and(|v| v < 0)
        {
          return Err(anyhow::anyhow!("Invalid maximum cache response size configuration").into());
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("cache_vary", config, used_properties) {
      for entry in &entries.inner {
        for value in &entry.values {
          if !value.is_string() {
            return Err(anyhow::anyhow!("Invalid varying request headers configuration").into());
          }
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("cache_ignore", config, used_properties) {
      for entry in &entries.inner {
        for value in &entry.values {
          if !value.is_string() {
            return Err(
              anyhow::anyhow!("Invalid ignored cache response headers configuration").into(),
            );
          }
        }
      }
    }

    Ok(())
  }
}

/// A cache module with optimized data structures
#[allow(clippy::type_complexity)]
struct MemCacheModule {
  cache: Arc<AtomicGenericCache<String, Option<CacheEntry>>>,
  vary_cache: Arc<papaya::HashMap<String, Vec<String>, RandomState>>,
}

impl Module for MemCacheModule {
  fn get_module_handlers(&self) -> Box<dyn ModuleHandlers> {
    Box::new(CacheModuleHandlers {
      cache: self.cache.clone(),
      vary_cache: self.vary_cache.clone(),
      cache_vary_headers_configured: HeaderList::new(),
      cache_ignore_headers_configured: HeaderList::new(),
      maximum_cached_response_size: None,
      cache_key: None,
      request_headers: HeaderMap::new(),
      has_authorization: false,
      cached: false,
      no_store: false,
    })
  }
}

/// Optimized handlers for the cache module
#[allow(clippy::type_complexity)]
struct CacheModuleHandlers {
  cache: Arc<AtomicGenericCache<String, Option<CacheEntry>>>,
  vary_cache: Arc<papaya::HashMap<String, Vec<String>, RandomState>>,
  cache_vary_headers_configured: HeaderList,
  cache_ignore_headers_configured: HeaderList,
  maximum_cached_response_size: Option<u64>,
  cache_key: Option<String>,
  request_headers: HeaderMap<HeaderValue>,
  has_authorization: bool,
  cached: bool,
  no_store: bool,
}

impl CacheModuleHandlers {
  /// Extract cache configuration with minimal allocations
  #[inline]
  fn extract_cache_config(&mut self, config: &ServerConfiguration) {
    self.cache_vary_headers_configured.clear();
    self.cache_ignore_headers_configured.clear();

    // Use extend for better performance
    self.cache_vary_headers_configured.extend(
      get_values!("cache_vary", config)
        .into_iter()
        .filter_map(|v| v.as_str().map(String::from)),
    );

    self.cache_ignore_headers_configured.extend(
      get_values!("cache_ignore", config)
        .into_iter()
        .filter_map(|v| v.as_str().map(String::from)),
    );

    self.maximum_cached_response_size =
      get_value!("cache_max_response_size", config).and_then(|v| v.as_i128().map(|f| f as u64));
  }

  /// Optimized cache cleanup with batching
  fn cleanup_expired_entries(&self) {
    let default_max_age = Duration::from_secs(DEFAULT_MAX_AGE);
    let now = Instant::now();

    self.cache.retain(|value, _, _| {
      if let Some((_, _, _, timestamp, cache_control)) = value {
        let max_age = cache_control
          .as_ref()
          .and_then(|cc| cc.s_max_age.or(cc.max_age))
          .unwrap_or(default_max_age);

        now.duration_since(*timestamp) <= max_age
      } else {
        false
      }
    });
  }

  /// Fast cache control evaluation
  #[inline]
  fn should_cache_response(
    &self,
    response_cache_control: &Option<CacheControl>,
    has_authorization: bool,
  ) -> bool {
    match response_cache_control {
      Some(cache_control) => {
        if cache_control.no_store {
          return false;
        }

        match cache_control.cachability {
          Some(Cachability::Private) => false,
          Some(Cachability::Public) => true,
          _ => {
            !has_authorization
              && (cache_control.max_age.is_some() || cache_control.s_max_age.is_some())
          }
        }
      }
      None => false,
    }
  }
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
    // Extract configuration once per request
    self.extract_cache_config(config);

    // Fast cache decision
    let cache_decision = CacheDecision::from_request(&request);

    if cache_decision.no_store() {
      self.no_store = true;
      return Ok(ResponseData {
        request: Some(request),
        response: None,
        response_status: None,
        response_headers: None,
        new_remote_address: None,
      });
    }

    // Extract URI parts efficiently
    let (host, path, query) = extract_uri_parts(&request);

    // Build cache key
    let cache_key = build_cache_key(
      request.method(),
      socket_data.encrypted,
      &host,
      &path,
      query.as_deref(),
    );

    // Check cache only if not no-cache
    if !cache_decision.no_cache() {
      let vary_cache_guard = self.vary_cache.pin_owned();
      if let Some(processed_vary) = vary_cache_guard.get(&cache_key) {
        // Use thread-local builder for vary key
        let cache_key_with_vary = VARY_KEY_BUILDER.with(|builder| {
          builder
            .borrow_mut()
            .build(&cache_key, processed_vary, request.headers())
            .to_string()
        });

        if let Some(Some((status_code, headers, body, timestamp, response_cache_control))) =
          self.cache.get(&cache_key_with_vary)
        {
          let max_age = response_cache_control
            .as_ref()
            .and_then(|cc| cc.s_max_age.or(cc.max_age))
            .unwrap_or(Duration::from_secs(DEFAULT_MAX_AGE));

          if timestamp.elapsed() <= max_age {
            self.cached = true;

            let mut hyper_response_builder = Response::builder().status(status_code);
            for (header_name, header_value) in headers.iter() {
              hyper_response_builder = hyper_response_builder.header(header_name, header_value);
            }

            let hyper_response = hyper_response_builder
              .body(Full::new(Bytes::from(body)).map_err(|e| match e {}).boxed())?;

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

    // Store request data for response processing
    self.request_headers = request.headers().clone();
    self.cache_key = Some(cache_key);
    self.has_authorization = request.headers().contains_key(&*AUTHORIZATION_HEADER);

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
    // Fast path for common cases
    if self.no_store {
      response
        .headers_mut()
        .insert(CACHE_HEADER_NAME, CACHE_BYPASS.clone());
      return Ok(response);
    }

    if self.cached {
      response
        .headers_mut()
        .insert(CACHE_HEADER_NAME, CACHE_HIT.clone());
      return Ok(response);
    }

    let Some(cache_key) = &self.cache_key else {
      return Ok(response);
    };

    let (mut response_parts, mut response_body) = response.into_parts();

    // Fast cache control parsing
    let response_cache_control = response_parts
      .headers
      .get(&*CACHE_CONTROL_HEADER)
      .and_then(|value| value.to_str().ok())
      .and_then(CacheControl::from_value);

    let should_cache = self.should_cache_response(&response_cache_control, self.has_authorization);

    if should_cache {
      let mut body_handler = ResponseBodyHandler::new(self.maximum_cached_response_size);
      let body_collected = body_handler.collect_body(&mut response_body).await?;

      if !body_collected {
        // Size exceeded, stream response without caching
        let cached_stream =
          futures_util::stream::once(async move { Ok(Bytes::from(body_handler.into_bytes())) });
        let response_stream = response_body.into_data_stream();
        let chained_stream = cached_stream.chain(response_stream);
        let stream_body = StreamBody::new(chained_stream.map_ok(Frame::data));
        let response_body = BodyExt::boxed(stream_body);

        response_parts
          .headers
          .insert(CACHE_HEADER_NAME, CACHE_MISS.clone());

        return Ok(Response::from_parts(response_parts, response_body));
      }

      let response_body_buffer = body_handler.into_bytes();

      // Optimized vary header processing
      let mut processed_vary = self.cache_vary_headers_configured.clone();

      if let Some(vary_header) = response_parts.headers.get(&*VARY_HEADER) {
        if let Ok(vary_str) = vary_header.to_str() {
          processed_vary.extend(vary_str.split(',').map(|s| s.trim().to_string()));
        }
      }

      // Remove duplicates efficiently
      processed_vary.sort_unstable();
      processed_vary.dedup();

      if !processed_vary.iter().any(|h| h == "*") {
        // Use thread-local builder for vary key
        let cache_key_with_vary = VARY_KEY_BUILDER.with(|builder| {
          builder
            .borrow_mut()
            .build(cache_key, &processed_vary, &self.request_headers)
            .to_string()
        });

        // Update vary cache
        let vary_cache_guard = self.vary_cache.pin_owned();
        vary_cache_guard.insert(cache_key.clone(), processed_vary.into_vec());

        // Prepare headers for caching (remove ignored headers)
        let mut written_headers = response_parts.headers.clone();
        for header in &self.cache_ignore_headers_configured {
          written_headers.remove(header);
        }

        // Periodic cleanup
        self.cleanup_expired_entries();

        // Store in cache
        self
          .cache
          .force_insert(
            cache_key_with_vary,
            Some((
              response_parts.status,
              written_headers,
              response_body_buffer.clone(),
              Instant::now(),
              response_cache_control.map(Arc::new),
            )),
          )
          .unwrap_or_default();
      }

      // Create response stream efficiently
      let cached_stream =
        futures_util::stream::once(async move { Ok(Bytes::from(response_body_buffer)) });
      let stream_body = StreamBody::new(cached_stream.map_ok(Frame::data));
      let response_body = BodyExt::boxed(stream_body);

      response_parts
        .headers
        .insert(CACHE_HEADER_NAME, CACHE_MISS.clone());

      Ok(Response::from_parts(response_parts, response_body))
    } else {
      response_parts
        .headers
        .insert(CACHE_HEADER_NAME, CACHE_MISS.clone());

      Ok(Response::from_parts(response_parts, response_body))
    }
  }
}
