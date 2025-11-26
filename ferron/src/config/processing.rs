use std::{
  collections::{BTreeMap, HashMap, HashSet, VecDeque},
  error::Error,
  net::IpAddr,
};

use ferron_common::{
  config::{Conditional, ErrorHandlerStatus},
  modules::ModuleLoader,
};

use super::{ServerConfiguration, ServerConfigurationFilters};

/// Merges configurations with same filters
/// Combines server configurations with identical filters by merging their entries.
///
/// This function takes a vector of server configurations and combines those that have matching
/// filter criteria (hostname, IP, port, location prefix, and error handler status).
/// For configurations with identical filters, their entries are merged.
pub fn merge_duplicates(mut server_configurations: Vec<ServerConfiguration>) -> Vec<ServerConfiguration> {
  // Sort configurations by filter criteria
  server_configurations.sort_by(|a, b| {
    (
      &a.filters.is_host,
      &a.filters.port,
      &a.filters.ip,
      &a.filters.hostname,
      &a.filters
        .condition
        .as_ref()
        .map(|s| (&s.location_prefix, &s.conditionals)),
      &a.filters.error_handler_status,
    )
      .cmp(&(
        &b.filters.is_host,
        &b.filters.port,
        &b.filters.ip,
        &b.filters.hostname,
        &b.filters
          .condition
          .as_ref()
          .map(|s| (&s.location_prefix, &s.conditionals)),
        &b.filters.error_handler_status,
      ))
  });

  // Convert server configurations to a double-ended queue
  let mut server_configurations = VecDeque::from(server_configurations);

  let mut result = Vec::new();
  while !server_configurations.is_empty() {
    if let Some(mut current) = server_configurations.pop_front() {
      // Merge all adjacent configurations with matching filters
      while !server_configurations.is_empty()
        && server_configurations[0].filters.is_host == current.filters.is_host
        && server_configurations[0].filters.hostname == current.filters.hostname
        && server_configurations[0].filters.ip == current.filters.ip
        && server_configurations[0].filters.port == current.filters.port
        && server_configurations[0].filters.condition == current.filters.condition
        && server_configurations[0].filters.error_handler_status == current.filters.error_handler_status
      {
        if let Some(server_configuration) = server_configurations.pop_front() {
          // Merge entries
          for (k, v) in server_configuration.entries {
            current.entries.entry(k).or_default().inner.extend(v.inner);
          }
        }
      }
      result.push(current);
    }
  }

  result
}

/// Removes empty Ferron configurations and add an empty global configuration, if not present
/// Ensures there is a global configuration in the server configurations.
///
/// This function filters out empty configurations, checks if a global configuration exists,
/// and adds one if it doesn't.
pub fn remove_and_add_global_configuration(
  server_configurations: Vec<ServerConfiguration>,
) -> Vec<ServerConfiguration> {
  // The resulting list of server configurations
  let mut new_server_configurations = Vec::new();
  // Flag to track if a global non-host configuration exists
  let mut has_global_non_host = false;

  // Process each server configuration
  for server_configuration in server_configurations {
    // Only keep non-empty configurations
    if !server_configuration.entries.is_empty() {
      // Check if this is a global non-host configuration
      if server_configuration.filters.is_global_non_host() {
        has_global_non_host = true;
      }
      // Add the configuration to the result list
      new_server_configurations.push(server_configuration);
    }
  }

  // If no global non-host configuration exists, add a default one at the beginning
  if !has_global_non_host {
    new_server_configurations.insert(
      0,
      ServerConfiguration {
        entries: HashMap::new(),
        filters: ServerConfigurationFilters {
          is_host: false,
          hostname: None,
          ip: None,
          port: None,
          condition: None,
          error_handler_status: None,
        },
        modules: vec![],
      },
    );
  }

  // Return the processed configurations
  new_server_configurations
}

/// Configuration filter enum for a trie
#[derive(Clone, PartialEq, PartialOrd, Eq, Ord)]
enum ServerConfigurationFilter {
  /// Whether the configuration represents a host block
  IsHost(bool),

  /// The port
  Port(Option<u16>),

  /// The IP address
  Ip(Option<IpAddr>),

  /// The hostname
  Hostname(Option<String>),

  /// The conditions
  Condition(Option<(String, Vec<Conditional>)>),

  /// The error handler status code
  ErrorHandlerStatus(Option<ErrorHandlerStatus>),
}

/// Configuration filter trie
struct ServerConfigurationFilterTrie {
  children: BTreeMap<ServerConfigurationFilter, ServerConfigurationFilterTrie>,
  index: Option<usize>,
}

impl ServerConfigurationFilterTrie {
  /// Creates an empty ConfigurationFilterTrie.
  pub fn new() -> Self {
    Self {
      children: BTreeMap::new(),
      index: None,
    }
  }

  /// Inserts new filters with index into the trie.
  pub fn insert(&mut self, filters: ServerConfigurationFilters, filters_index: usize) {
    let no_host = !filters.is_host;
    let no_port = filters.port.is_none();
    let no_ip = filters.ip.is_none();
    let no_hostname = filters.hostname.is_none();
    let no_condition = filters.condition.is_none();
    let no_error_handler_status = filters.error_handler_status.is_none();

    let filter_vec = vec![
      ServerConfigurationFilter::IsHost(filters.is_host),
      ServerConfigurationFilter::Port(filters.port),
      ServerConfigurationFilter::Ip(filters.ip),
      ServerConfigurationFilter::Hostname(filters.hostname),
      ServerConfigurationFilter::Condition(filters.condition.map(|s| (s.location_prefix, s.conditionals))),
      ServerConfigurationFilter::ErrorHandlerStatus(filters.error_handler_status),
    ];

    let mut current_node = self;
    for filter in filter_vec {
      if match &filter {
        ServerConfigurationFilter::IsHost(_) => {
          no_host && no_port && no_ip && no_hostname && no_condition && no_error_handler_status
        }
        ServerConfigurationFilter::Port(_) => {
          no_port && no_ip && no_hostname && no_condition && no_error_handler_status
        }
        ServerConfigurationFilter::Ip(_) => no_ip && no_hostname && no_condition && no_error_handler_status,
        ServerConfigurationFilter::Hostname(_) => no_hostname && no_condition && no_error_handler_status,
        ServerConfigurationFilter::Condition(_) => no_condition && no_error_handler_status,
        ServerConfigurationFilter::ErrorHandlerStatus(_) => no_error_handler_status,
      } && current_node.index.is_none()
      {
        current_node.index = Some(filters_index);
      }
      if !current_node.children.contains_key(&filter) {
        current_node.children.insert(filter.clone(), Self::new());
      }
      match current_node.children.get_mut(&filter) {
        Some(node) => current_node = node,
        None => unreachable!(),
      }
    }
  }

  /// Finds indices by the filters in the trie.
  pub fn find_indices(&self, filters: ServerConfigurationFilters) -> Vec<usize> {
    let filter_vec = vec![
      ServerConfigurationFilter::IsHost(filters.is_host),
      ServerConfigurationFilter::Port(filters.port),
      ServerConfigurationFilter::Ip(filters.ip),
      ServerConfigurationFilter::Hostname(filters.hostname),
      ServerConfigurationFilter::Condition(filters.condition.map(|s| (s.location_prefix, s.conditionals))),
      ServerConfigurationFilter::ErrorHandlerStatus(filters.error_handler_status),
    ];

    let mut current_node = self;
    let mut indices = Vec::new();
    for filter in filter_vec {
      if indices.last() != current_node.index.as_ref() {
        if let Some(index) = current_node.index {
          indices.push(index);
        }
      }
      let child = current_node.children.get(&filter);
      match child {
        Some(child) => {
          current_node = child;
        }
        None => break,
      }
    }
    indices.reverse();
    indices
  }
}

/// Pre-merges Ferron configurations
/// Merges server configurations based on a hierarchical inheritance model.
///
/// This function implements a layered configuration system where more specific configurations
/// inherit and override properties from less specific ones. It handles matching logic based
/// on specificity of filters (error handlers, location prefixes, hostnames, IPs, ports).
pub fn premerge_configuration(mut server_configurations: Vec<ServerConfiguration>) -> Vec<ServerConfiguration> {
  // Sort server configurations vector, based on the ascending specifity, to make the algorithm easier to implement
  server_configurations.sort_by(|a, b| a.filters.cmp(&b.filters));

  // Initialize a trie to store server configurations based on their filters
  let mut server_configuration_filter_trie = ServerConfigurationFilterTrie::new();
  for (index, server_configuration) in server_configurations.iter().enumerate() {
    server_configuration_filter_trie.insert(server_configuration.filters.clone(), index);
  }

  // Initialize a vector to store the new server configurations
  let mut new_server_configurations = Vec::with_capacity(server_configurations.len());

  // Pre-merge server configurations
  while let Some(mut server_configuration) = server_configurations.pop() {
    // Get the layers indexes
    let layers_indexes = server_configuration_filter_trie.find_indices(server_configuration.filters.clone());

    // Start with current configuration's entries
    let mut configuration_entries = server_configuration.entries;

    // Process all parent configurations that this one should inherit from
    for layer_index in layers_indexes {
      // If layer index is out of bounds, skip it
      if layer_index >= server_configurations.len() {
        continue;
      }

      // Track which properties have been processed in this layer
      let mut properties_in_layer = HashSet::new();
      // Clone parent configuration's entries
      let mut cloned_hashmap = server_configurations[layer_index].entries.clone();
      // Iterate through child configuration's entries
      let moved_hashmap_iterator = configuration_entries.into_iter();
      // Merge child entries with parent entries
      for (property_name, mut property) in moved_hashmap_iterator {
        match cloned_hashmap.get_mut(&property_name) {
          Some(obtained_property) => {
            if properties_in_layer.contains(&property_name) {
              // If property was already processed in this layer, append values
              obtained_property.inner.append(&mut property.inner);
            } else {
              // If property appears for the first time, replace values
              obtained_property.inner = property.inner;
            }
          }
          None => {
            // If property doesn't exist in parent, add it
            cloned_hashmap.insert(property_name.clone(), property);
          }
        }
        // Mark this property as processed in this layer
        properties_in_layer.insert(property_name);
      }
      // Update entries with merged result
      configuration_entries = cloned_hashmap;
    }
    // Assign the merged entries back to the configuration
    server_configuration.entries = configuration_entries;

    // Add the processed configuration to the result list
    new_server_configurations.push(server_configuration);
  }

  // Reverse the result to restore original specificity order
  new_server_configurations.reverse();
  new_server_configurations
}

/// Loads Ferron modules into its configurations
/// Loads and validates modules for each server configuration.
///
/// This function processes each server configuration, validates it against available modules,
/// and loads modules that meet their requirements. It tracks unused properties and any errors
/// that occur during module loading.
pub fn load_modules(
  server_configurations: Vec<ServerConfiguration>,
  server_modules: &mut [Box<dyn ModuleLoader + Send + Sync>],
  secondary_runtime: &tokio::runtime::Runtime,
) -> (
  Vec<ServerConfiguration>,
  Option<Box<dyn Error + Send + Sync>>,
  Vec<String>,
) {
  // The resulting list of server configurations with loaded modules
  let mut new_server_configurations = Vec::new();
  // The first error encountered during module loading (if any)
  let mut first_server_module_error = None;
  // Properties that weren't used by any module
  let mut unused_properties = HashSet::new();

  // Find the global configuration to pass to modules
  let global_configuration = find_global_configuration(&server_configurations);

  // Process each server configuration
  for mut server_configuration in server_configurations {
    // Track which properties are used by modules
    let mut used_properties = HashSet::new();

    // Process each available server module
    for server_module in server_modules.iter_mut() {
      // Get module requirements
      let requirements = server_module.get_requirements();
      // Check if this module's requirements are satisfied by this configuration
      let mut requirements_met = true;
      for requirement in requirements {
        requirements_met = false;
        // Check if the required property exists and has a non-null value
        if server_configuration
          .entries
          .get(requirement)
          .and_then(|e| e.get_value())
          .is_some_and(|v| !v.is_null() && v.as_bool().unwrap_or(true))
        {
          requirements_met = true;
          break;
        }
      }
      // Validate the configuration against this module
      match server_module.validate_configuration(&server_configuration, &mut used_properties) {
        Ok(_) => (),
        Err(error) => {
          // Store the first error encountered
          if first_server_module_error.is_none() {
            first_server_module_error
              .replace(anyhow::anyhow!("{error} (at {})", server_configuration.filters).into_boxed_dyn_error());
          }
          // Skip remaining modules for this configuration if validation fails
          break;
        }
      }
      // Only load module if its requirements are met
      if requirements_met {
        // Load the module with current configuration and global configuration
        match server_module.load_module(&server_configuration, global_configuration.as_ref(), secondary_runtime) {
          Ok(loaded_module) => server_configuration.modules.push(loaded_module),
          Err(error) => {
            // Store the first error encountered
            if first_server_module_error.is_none() {
              first_server_module_error
                .replace(anyhow::anyhow!("{error} (at {})", server_configuration.filters).into_boxed_dyn_error());
            }
            // Skip remaining modules for this configuration if loading fails
            break;
          }
        }
      }
    }

    // Track unused properties (except for undocumented ones)
    for property in server_configuration.entries.keys() {
      if !property.starts_with("UNDOCUMENTED_") && !used_properties.contains(property) {
        unused_properties.insert(property.to_string());
      }
    }

    // Add the configuration with loaded modules to the result list
    new_server_configurations.push(server_configuration);
  }
  // Return:
  // 1. Server configurations with modules loaded
  // 2. First error encountered (if any)
  // 3. List of unused properties
  (
    new_server_configurations,
    first_server_module_error,
    unused_properties.into_iter().collect(),
  )
}

/// Finds the global server configuration (host or non-host) from the given list of server configurations.
fn find_global_configuration(server_configurations: &[ServerConfiguration]) -> Option<ServerConfiguration> {
  // The server configurations are pre-merged, so we can simply return the found global configuration
  let mut iterator = server_configurations.iter();
  let first_found = iterator.find(|server_configuration| {
    server_configuration.filters.is_global() || server_configuration.filters.is_global_non_host()
  });
  if let Some(first_found) = first_found {
    if first_found.filters.is_global() {
      return Some(first_found.clone());
    }
    for server_configuration in iterator {
      if server_configuration.filters.is_global() {
        return Some(server_configuration.clone());
      } else if !server_configuration.filters.is_global_non_host() {
        return Some(first_found.clone());
      }
    }
  }
  None
}

#[cfg(test)]
mod tests {
  use crate::config::*;

  use super::*;
  use std::collections::HashMap;
  use std::net::{IpAddr, Ipv4Addr};

  fn make_filters(
    is_host: bool,
    hostname: Option<&str>,
    ip: Option<IpAddr>,
    port: Option<u16>,
    location_prefix: Option<&str>,
    error_handler_status: Option<ErrorHandlerStatus>,
  ) -> ServerConfigurationFilters {
    ServerConfigurationFilters {
      is_host,
      hostname: hostname.map(String::from),
      ip,
      port,
      condition: location_prefix.map(|prefix| Conditions {
        location_prefix: prefix.to_string(),
        conditionals: vec![],
      }),
      error_handler_status,
    }
  }

  fn make_entry(values: Vec<ServerConfigurationValue>) -> ServerConfigurationEntries {
    ServerConfigurationEntries {
      inner: vec![ServerConfigurationEntry {
        values,
        props: HashMap::new(),
      }],
    }
  }

  fn make_entry_premerge(key: &str, value: ServerConfigurationValue) -> (String, ServerConfigurationEntries) {
    let entry = ServerConfigurationEntry {
      values: vec![value],
      props: HashMap::new(),
    };
    (key.to_string(), ServerConfigurationEntries { inner: vec![entry] })
  }

  fn config_with_filters(
    is_host: bool,
    hostname: Option<&str>,
    ip: Option<IpAddr>,
    port: Option<u16>,
    location_prefix: Option<&str>,
    error_handler_status: Option<ErrorHandlerStatus>,
    entries: Vec<(String, ServerConfigurationEntries)>,
  ) -> ServerConfiguration {
    ServerConfiguration {
      filters: ServerConfigurationFilters {
        is_host,
        hostname: hostname.map(|s| s.to_string()),
        ip,
        port,
        condition: location_prefix.map(|prefix| Conditions {
          location_prefix: prefix.to_string(),
          conditionals: vec![],
        }),
        error_handler_status,
      },
      entries: entries.into_iter().collect(),
      modules: vec![],
    }
  }

  #[test]
  fn merges_identical_filters_and_combines_entries() {
    let filters = make_filters(
      true,
      Some("example.com"),
      Some(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))),
      Some(8080),
      Some("/api"),
      Some(ErrorHandlerStatus::Status(404)),
    );

    let filters_2 = make_filters(
      true,
      Some("example.com"),
      Some(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))),
      Some(8080),
      Some("/api"),
      Some(ErrorHandlerStatus::Status(404)),
    );

    let mut config1_entries = HashMap::new();
    config1_entries.insert(
      "route".to_string(),
      make_entry(vec![ServerConfigurationValue::String("v1".to_string())]),
    );

    let mut config2_entries = HashMap::new();
    config2_entries.insert(
      "route".to_string(),
      make_entry(vec![ServerConfigurationValue::String("v2".to_string())]),
    );

    let config1 = ServerConfiguration {
      filters: filters_2,
      entries: config1_entries,
      modules: vec![],
    };

    let config2 = ServerConfiguration {
      filters,
      entries: config2_entries,
      modules: vec![],
    };

    let merged = merge_duplicates(vec![config1, config2]);
    assert_eq!(merged.len(), 1);

    let merged_entries = &merged[0].entries;
    assert!(merged_entries.contains_key("route"));
    let route_entry = merged_entries.get("route").unwrap();
    let values: Vec<_> = route_entry.inner.iter().flat_map(|e| e.values.iter()).collect();
    assert_eq!(values.len(), 2);
    assert!(values.contains(&&ServerConfigurationValue::String("v1".into())));
    assert!(values.contains(&&ServerConfigurationValue::String("v2".into())));
  }

  #[test]
  fn does_not_merge_different_filters() {
    let filters1 = make_filters(
      true,
      Some("example.com"),
      Some(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))),
      Some(8080),
      Some("/api"),
      None,
    );

    let filters2 = make_filters(
      true,
      Some("example.org"),
      Some(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))),
      Some(8080),
      Some("/api"),
      None,
    );

    let mut config1_entries = HashMap::new();
    config1_entries.insert(
      "route".to_string(),
      make_entry(vec![ServerConfigurationValue::String("v1".to_string())]),
    );

    let mut config2_entries = HashMap::new();
    config2_entries.insert(
      "route".to_string(),
      make_entry(vec![ServerConfigurationValue::String("v2".to_string())]),
    );

    let config1 = ServerConfiguration {
      filters: filters1,
      entries: config1_entries,
      modules: vec![],
    };

    let config2 = ServerConfiguration {
      filters: filters2,
      entries: config2_entries,
      modules: vec![],
    };

    let merged = merge_duplicates(vec![config1, config2]);
    assert_eq!(merged.len(), 2);
  }

  #[test]
  fn handles_filters_then_unique_then_duplicate() {
    let filters1 = make_filters(
      true,
      Some("example.com"),
      Some(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))),
      Some(8080),
      Some("/api"),
      None,
    );

    let filters2 = make_filters(
      true,
      Some("example.org"),
      Some(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))),
      Some(8080),
      Some("/api"),
      None,
    );

    let mut config1_entries = HashMap::new();
    config1_entries.insert(
      "route".to_string(),
      make_entry(vec![ServerConfigurationValue::String("v1".to_string())]),
    );

    let mut config2_entries = HashMap::new();
    config2_entries.insert(
      "route".to_string(),
      make_entry(vec![ServerConfigurationValue::String("v2".to_string())]),
    );

    let mut config3_entries = HashMap::new();
    config3_entries.insert(
      "route".to_string(),
      make_entry(vec![ServerConfigurationValue::String("v3".to_string())]),
    );

    let config1 = ServerConfiguration {
      filters: filters1.clone(),
      entries: config1_entries,
      modules: vec![],
    };

    let config2 = ServerConfiguration {
      filters: filters2,
      entries: config2_entries,
      modules: vec![],
    };

    let config3 = ServerConfiguration {
      filters: filters1,
      entries: config3_entries,
      modules: vec![],
    };

    let merged = merge_duplicates(vec![config1, config2, config3]);
    assert_eq!(merged.len(), 2);
  }

  #[test]
  fn merges_entries_with_non_overlapping_keys() {
    let filters = make_filters(
      true,
      Some("example.com"),
      Some(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))),
      Some(8080),
      None,
      None,
    );

    let filters_2 = make_filters(
      true,
      Some("example.com"),
      Some(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))),
      Some(8080),
      None,
      None,
    );

    let mut config1_entries = HashMap::new();
    config1_entries.insert(
      "route1".to_string(),
      make_entry(vec![ServerConfigurationValue::String("r1".to_string())]),
    );

    let mut config2_entries = HashMap::new();
    config2_entries.insert(
      "route2".to_string(),
      make_entry(vec![ServerConfigurationValue::String("r2".to_string())]),
    );

    let config1 = ServerConfiguration {
      filters: filters_2,
      entries: config1_entries,
      modules: vec![],
    };

    let config2 = ServerConfiguration {
      filters,
      entries: config2_entries,
      modules: vec![],
    };

    let merged = merge_duplicates(vec![config1, config2]);
    assert_eq!(merged.len(), 1);

    let merged_entries = &merged[0].entries;
    assert_eq!(merged_entries.len(), 2);
    assert!(merged_entries.contains_key("route1"));
    assert!(merged_entries.contains_key("route2"));
  }

  #[test]
  fn test_no_merge_returns_all() {
    let config1 = config_with_filters(
      true,
      Some("example.com"),
      Some(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))),
      Some(80),
      None,
      None,
      vec![make_entry_premerge(
        "key1",
        ServerConfigurationValue::String("val1".into()),
      )],
    );

    let config2 = config_with_filters(
      true,
      Some("example.org"),
      Some(IpAddr::V4(Ipv4Addr::new(192, 168, 0, 1))),
      Some(8080),
      None,
      None,
      vec![make_entry_premerge(
        "key2",
        ServerConfigurationValue::String("val2".into()),
      )],
    );

    let merged = premerge_configuration(vec![config1, config2]);

    assert_eq!(merged.len(), 2);
    assert!(merged.iter().any(|c| c.entries.contains_key("key1")));
    assert!(merged.iter().any(|c| c.entries.contains_key("key2")));
  }

  #[test]
  fn test_merge_case6_is_host() {
    // Less specific config (no port)
    let base = config_with_filters(
      false,
      None,
      None,
      None,
      None,
      None,
      vec![make_entry_premerge(
        "shared",
        ServerConfigurationValue::String("base".into()),
      )],
    );

    // More specific config (with port)
    let specific = config_with_filters(
      true,
      None,
      None,
      None,
      None,
      None,
      vec![make_entry_premerge(
        "shared",
        ServerConfigurationValue::String("specific".into()),
      )],
    );

    let merged = premerge_configuration(vec![base, specific]);
    assert_eq!(merged.len(), 2);

    let entries = &merged[1].entries["shared"].inner;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].values[0].as_str(), Some("specific"));
  }

  #[test]
  fn test_merge_case5_port() {
    // Less specific config (no port)
    let base = config_with_filters(
      true,
      None,
      None,
      None,
      None,
      None,
      vec![make_entry_premerge(
        "shared",
        ServerConfigurationValue::String("base".into()),
      )],
    );

    // More specific config (with port)
    let specific = config_with_filters(
      true,
      None,
      None,
      Some(80),
      None,
      None,
      vec![make_entry_premerge(
        "shared",
        ServerConfigurationValue::String("specific".into()),
      )],
    );

    let merged = premerge_configuration(vec![base, specific]);
    assert_eq!(merged.len(), 2);

    let entries = &merged[1].entries["shared"].inner;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].values[0].as_str(), Some("specific"));
  }

  #[test]
  fn test_merge_case1_error_handler() {
    let base = config_with_filters(
      true,
      Some("host"),
      Some(IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4))),
      Some(3000),
      Some("/api"),
      None,
      vec![make_entry_premerge(
        "eh",
        ServerConfigurationValue::String("base".into()),
      )],
    );

    let specific = config_with_filters(
      true,
      Some("host"),
      Some(IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4))),
      Some(3000),
      Some("/api"),
      Some(ErrorHandlerStatus::Any),
      vec![make_entry_premerge(
        "eh",
        ServerConfigurationValue::String("specific".into()),
      )],
    );

    let merged = premerge_configuration(vec![base, specific]);
    assert_eq!(merged.len(), 2);

    let entries = &merged[1].entries["eh"].inner;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].values[0].as_str(), Some("specific"));
  }

  #[test]
  fn test_merge_preserves_specificity_order() {
    let configs = vec![
      config_with_filters(
        true,
        None,
        None,
        None,
        None,
        None,
        vec![make_entry_premerge("a", ServerConfigurationValue::String("v1".into()))],
      ),
      config_with_filters(
        true,
        None,
        None,
        Some(80),
        None,
        None,
        vec![make_entry_premerge("a", ServerConfigurationValue::String("v2".into()))],
      ),
      config_with_filters(
        true,
        Some("host"),
        None,
        Some(80),
        None,
        None,
        vec![make_entry_premerge("a", ServerConfigurationValue::String("v3".into()))],
      ),
    ];

    let merged = premerge_configuration(configs);
    assert_eq!(merged.len(), 3);

    let entries = &merged[2].entries["a"].inner;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].values[0].as_str(), Some("v3"));
  }
}
