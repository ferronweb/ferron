use std::sync::Arc;
use std::{collections::HashMap, error::Error};

use crate::config::{ServerConfiguration, ServerConfigurationEntries};

/// The cache that stores modules according to the server configuration
#[allow(clippy::type_complexity)]
pub struct ModuleCache<T> {
  // Cannot use either HashMap or BTreeMap for the inner cache, so a regular vector is used, which may result in slower lookups
  inner: Vec<(HashMap<String, Option<ServerConfigurationEntries>>, Arc<T>)>,
  properties: Vec<&'static str>,
}

impl<T> ModuleCache<T> {
  /// Creates a cache that stores modules per specific properties
  pub fn new(properties: Vec<&'static str>) -> Self {
    ModuleCache {
      inner: Vec::new(),
      properties,
    }
  }

  /// Obtains a module from a cache, and if not present, initializes a new one
  pub fn get_or(
    &mut self,
    config: &ServerConfiguration,
    mut init_fn: impl FnMut(&ServerConfiguration) -> Result<Arc<T>, Box<dyn Error + Send + Sync>>,
  ) -> Result<Arc<T>, Box<dyn Error + Send + Sync>> {
    let mut cache_key = HashMap::new();
    for property_name in &self.properties {
      cache_key.insert(
        (*property_name).to_string(),
        config.entries.get(*property_name).cloned(),
      );
    }
    for (cache_key_obtained, cache_value) in &self.inner {
      if cache_key_obtained == &cache_key {
        return Ok(cache_value.clone());
      }
    }
    let value_to_cache = init_fn(config)?;
    self.inner.push((cache_key, value_to_cache.clone()));
    Ok(value_to_cache)
  }
}

#[cfg(test)]
mod test {
  use crate::config::{
    ServerConfigurationEntry, ServerConfigurationFilters, ServerConfigurationValue,
  };

  use super::*;

  #[test]
  fn should_cache_the_module() {
    let module = 1; // A fake "module"
    let module2 = 2; // Another fake "module"

    // Initialize cache
    let mut cache = ModuleCache::new(vec!["property"]);

    // Initialize two configuration structs
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
          values: vec![ServerConfigurationValue::String(
            "something else".to_string(),
          )],
          props: HashMap::new(),
        }],
      },
    );
    let config2 = ServerConfiguration {
      entries: config2_entries,
      filters: ServerConfigurationFilters {
        hostname: None,
        ip: None,
        port: Some(80), // Something different
        location_prefix: None,
        error_handler_status: None,
      },
      modules: vec![],
    };

    // Should initialize a "module"
    assert_eq!(
      cache
        .get_or(&config, |_config| { Ok(Arc::new(module)) })
        .unwrap(),
      Arc::new(module)
    );

    // Should obtain a cached "module"
    assert_eq!(
      cache
        .get_or(&config2, |_config| { Ok(Arc::new(module2)) })
        .unwrap(),
      Arc::new(module)
    );
  }
}
