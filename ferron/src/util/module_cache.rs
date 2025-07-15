use std::hash::Hasher;
use std::sync::Arc;
use std::{collections::HashMap, error::Error};

use crate::config::{ServerConfiguration, ServerConfigurationEntries};

/// A highly optimized cache that stores modules according to server configuration
pub struct ModuleCache<T> {
  // Use a single HashMap for O(1) average-case lookups
  inner: HashMap<CacheKey, Arc<T>>,
  properties: Box<[&'static str]>, // Box<[T]> is more memory efficient than Vec<T>
}

// Optimized cache key that implements fast hashing and comparison
#[derive(Clone, PartialEq, Eq)]
struct CacheKey {
  // Pre-sorted entries for consistent hashing
  entries: Box<[(String, Option<ServerConfigurationEntries>)]>,
}

impl std::hash::Hash for CacheKey {
  fn hash<H: Hasher>(&self, state: &mut H) {
    self.entries.hash(state);
  }
}

impl CacheKey {
  fn new(config: &ServerConfiguration, properties: &[&'static str]) -> Self {
    let mut entries: Vec<_> = properties
      .iter()
      .map(|&prop| (prop.to_string(), config.entries.get(prop).cloned()))
      .collect();

    // Sort for consistent cache keys regardless of property order
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    Self {
      entries: entries.into_boxed_slice(),
    }
  }
}

#[allow(dead_code)]
impl<T> ModuleCache<T> {
  /// Creates a cache that stores modules per specific properties
  pub fn new(properties: Vec<&'static str>) -> Self {
    Self {
      inner: HashMap::with_capacity(16), // Pre-allocate reasonable capacity
      properties: properties.into_boxed_slice(),
    }
  }

  /// Creates a cache with custom initial capacity
  pub fn with_capacity(properties: Vec<&'static str>, capacity: usize) -> Self {
    Self {
      inner: HashMap::with_capacity(capacity),
      properties: properties.into_boxed_slice(),
    }
  }

  /// Obtains a module from cache, initializing if not present
  /// This is now O(1) average case instead of O(n)
  pub fn get_or_init<F, E>(&mut self, config: &ServerConfiguration, init_fn: F) -> Result<Arc<T>, E>
  where
    F: FnOnce(&ServerConfiguration) -> Result<Arc<T>, E>,
    E: From<Box<dyn Error + Send + Sync>>,
  {
    let cache_key = CacheKey::new(config, &self.properties);

    // Fast path: check if already cached
    if let Some(cached_value) = self.inner.get(&cache_key) {
      return Ok(cached_value.clone());
    }

    // Slow path: initialize and cache
    let new_value = init_fn(config)?;
    self.inner.insert(cache_key, new_value.clone());
    Ok(new_value)
  }

  /// Non-mutable variant that only retrieves from cache
  pub fn get(&self, config: &ServerConfiguration) -> Option<Arc<T>> {
    let cache_key = CacheKey::new(config, &self.properties);
    self.inner.get(&cache_key).cloned()
  }

  /// Clear the cache
  pub fn clear(&mut self) {
    self.inner.clear();
  }

  /// Get current cache size
  pub fn len(&self) -> usize {
    self.inner.len()
  }

  /// Check if cache is empty
  pub fn is_empty(&self) -> bool {
    self.inner.is_empty()
  }

  /// Reserve capacity for additional entries
  pub fn reserve(&mut self, additional: usize) {
    self.inner.reserve(additional);
  }

  /// Gets a module from cache, or creates one with the fallback function without caching
  pub fn get_or<F, E>(&self, config: &ServerConfiguration, fallback_fn: F) -> Result<Arc<T>, E>
  where
    F: FnOnce(&ServerConfiguration) -> Result<Arc<T>, E>,
  {
    let cache_key = CacheKey::new(config, &self.properties);

    // Check if already cached
    if let Some(cached_value) = self.inner.get(&cache_key) {
      return Ok(cached_value.clone());
    }

    // Not cached, use fallback function (but don't cache the result)
    fallback_fn(config)
  }
}

// Implement Default for convenience
impl<T> Default for ModuleCache<T> {
  fn default() -> Self {
    Self::new(Vec::new())
  }
}

#[cfg(test)]
mod test {
  use crate::config::{ServerConfigurationEntry, ServerConfigurationFilters, ServerConfigurationValue};

  use super::*;

  #[test]
  fn module_loading_test() {
    let module = 1;

    let cache = ModuleCache::new(vec!["property"]);

    let mut config_entries = HashMap::new();
    config_entries.insert(
      "property".to_string(),
      ServerConfigurationEntries {
        inner: vec![ServerConfigurationEntry {
          values: vec![ServerConfigurationValue::String("something".to_string())],
          props: HashMap::new(),
        }],
      },
    );
    let config = ServerConfiguration {
      entries: config_entries,
      filters: ServerConfigurationFilters {
        is_host: true,
        hostname: None,
        ip: None,
        port: None,
        location_prefix: None,
        error_handler_status: None,
      },
      modules: vec![],
    };

    let mut config2_entries = HashMap::new();
    config2_entries.insert(
      "property".to_string(),
      ServerConfigurationEntries {
        inner: vec![ServerConfigurationEntry {
          values: vec![ServerConfigurationValue::String("something".to_string())],
          props: HashMap::new(),
        }],
      },
    );
    config2_entries.insert(
      "ignore".to_string(),
      ServerConfigurationEntries {
        inner: vec![ServerConfigurationEntry {
          values: vec![ServerConfigurationValue::String("something else".to_string())],
          props: HashMap::new(),
        }],
      },
    );
    let config2 = ServerConfiguration {
      entries: config2_entries,
      filters: ServerConfigurationFilters {
        is_host: true,
        hostname: None,
        ip: None,
        port: Some(80),
        location_prefix: None,
        error_handler_status: None,
      },
      modules: vec![],
    };

    assert_eq!(
      cache
        .get_or::<_, Box<dyn std::error::Error + Send + Sync>>(&config, |_config| Ok(Arc::new(module)))
        .unwrap(),
      Arc::new(module)
    );

    assert_eq!(
      cache
        .get_or::<_, Box<dyn std::error::Error + Send + Sync>>(&config2, |_config| Ok(Arc::new(module)))
        .unwrap(),
      Arc::new(module)
    );
  }

  #[test]
  fn should_cache_the_module() {
    let module = 1;
    let module2 = 2;

    let mut cache = ModuleCache::new(vec!["property"]);

    let mut config_entries = HashMap::new();
    config_entries.insert(
      "property".to_string(),
      ServerConfigurationEntries {
        inner: vec![ServerConfigurationEntry {
          values: vec![ServerConfigurationValue::String("something".to_string())],
          props: HashMap::new(),
        }],
      },
    );
    let config = ServerConfiguration {
      entries: config_entries,
      filters: ServerConfigurationFilters {
        is_host: true,
        hostname: None,
        ip: None,
        port: None,
        location_prefix: None,
        error_handler_status: None,
      },
      modules: vec![],
    };

    let mut config2_entries = HashMap::new();
    config2_entries.insert(
      "property".to_string(),
      ServerConfigurationEntries {
        inner: vec![ServerConfigurationEntry {
          values: vec![ServerConfigurationValue::String("something".to_string())],
          props: HashMap::new(),
        }],
      },
    );
    config2_entries.insert(
      "ignore".to_string(),
      ServerConfigurationEntries {
        inner: vec![ServerConfigurationEntry {
          values: vec![ServerConfigurationValue::String("something else".to_string())],
          props: HashMap::new(),
        }],
      },
    );
    let config2 = ServerConfiguration {
      entries: config2_entries,
      filters: ServerConfigurationFilters {
        is_host: true,
        hostname: None,
        ip: None,
        port: Some(80),
        location_prefix: None,
        error_handler_status: None,
      },
      modules: vec![],
    };

    // Should initialize a module
    assert_eq!(
      cache
        .get_or_init::<_, Box<dyn std::error::Error + Send + Sync>>(&config, |_config| Ok(Arc::new(module)))
        .unwrap(),
      Arc::new(module)
    );

    // Should obtain cached module (not initialize module2)
    assert_eq!(
      cache
        .get_or_init::<_, Box<dyn std::error::Error + Send + Sync>>(&config2, |_config| Ok(Arc::new(module2)))
        .unwrap(),
      Arc::new(module)
    );
  }

  #[test]
  fn test_cache_operations() {
    let mut cache = ModuleCache::with_capacity(vec!["test_prop"], 10);

    let config = ServerConfiguration {
      entries: HashMap::new(),
      filters: ServerConfigurationFilters {
        is_host: true,
        hostname: None,
        ip: None,
        port: None,
        location_prefix: None,
        error_handler_status: None,
      },
      modules: vec![],
    };

    assert!(cache.is_empty());
    assert_eq!(cache.len(), 0);

    // Use Box<dyn Error + Send + Sync> directly
    let value = cache
      .get_or_init::<_, Box<dyn std::error::Error + Send + Sync>>(&config, |_| Ok(Arc::new(42)))
      .unwrap();
    assert_eq!(*value, 42);
    assert_eq!(cache.len(), 1);

    // Test direct get
    let cached = cache.get(&config).unwrap();
    assert_eq!(*cached, 42);

    cache.clear();
    assert!(cache.is_empty());
  }
}
