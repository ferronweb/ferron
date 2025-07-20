use std::{
  collections::{HashMap, HashSet},
  error::Error,
};

use crate::modules::ModuleLoader;

use super::{ServerConfiguration, ServerConfigurationFilters};

/// Merges configurations with same filters
/// Combines server configurations with identical filters by merging their entries.
///
/// This function takes a vector of server configurations and combines those that have matching
/// filter criteria (hostname, IP, port, location prefix, and error handler status).
/// For configurations with identical filters, their entries are merged.
pub fn merge_duplicates(mut server_configurations: Vec<ServerConfiguration>) -> Vec<ServerConfiguration> {
  // The resulting list of unique configurations after merging
  let mut server_configurations_without_duplicates = Vec::new();

  // Process each configuration one by one
  while !server_configurations.is_empty() {
    // Take the first configuration from the list
    let mut server_configuration = server_configurations.remove(0);
    let mut server_configurations_index = 0;

    // Compare this configuration with all remaining ones
    while server_configurations_index < server_configurations.len() {
      // Get the current configuration to compare with
      let server_configuration_source = &server_configurations[server_configurations_index];

      // Check if all filter criteria match exactly between the two configurations
      if server_configuration_source.filters.is_host == server_configuration.filters.is_host
        && server_configuration_source.filters.hostname == server_configuration.filters.hostname
        && server_configuration_source.filters.ip == server_configuration.filters.ip
        && server_configuration_source.filters.port == server_configuration.filters.port
        && server_configuration_source.filters.location_prefix == server_configuration.filters.location_prefix
        && server_configuration_source.filters.error_handler_status == server_configuration.filters.error_handler_status
      {
        // Clone the entries from the matching configuration
        let mut cloned_hashmap = server_configuration_source.entries.clone();
        let moved_hashmap_iterator = server_configuration.entries.into_iter();

        // Merge entries from both configurations
        for (property_name, mut property) in moved_hashmap_iterator {
          match cloned_hashmap.get_mut(&property_name) {
            Some(obtained_property) => {
              // If property exists in both configurations, combine their values
              obtained_property.inner.append(&mut property.inner);
            }
            None => {
              // If property only exists in current configuration, add it
              cloned_hashmap.insert(property_name, property);
            }
          }
        }

        // Update entries with merged result
        server_configuration.entries = cloned_hashmap;

        // Remove the processed configuration from the list
        server_configurations.remove(server_configurations_index);
      } else {
        // Move to next configuration if no match
        server_configurations_index += 1;
      }
    }

    // Add the processed configuration (with any merged entries) to the result list
    server_configurations_without_duplicates.push(server_configuration);
  }

  // Return the deduplicated configurations
  server_configurations_without_duplicates
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
          location_prefix: None,
          error_handler_status: None,
        },
        modules: vec![],
      },
    );
  }

  // Return the processed configurations
  new_server_configurations
}

/// Pre-merges Ferron configurations
/// Merges server configurations based on a hierarchical inheritance model.
///
/// This function implements a layered configuration system where more specific configurations
/// inherit and override properties from less specific ones. It handles matching logic based
/// on specificity of filters (error handlers, location prefixes, hostnames, IPs, ports).
pub fn premerge_configuration(mut server_configurations: Vec<ServerConfiguration>) -> Vec<ServerConfiguration> {
  // Sort server configurations vector, based on the ascending specifity, to simplify the merging algorithm
  server_configurations.sort_by(|a, b| a.filters.cmp(&b.filters));
  let mut new_server_configurations = Vec::new();

  // Process configurations from most specific to least specific
  while let Some(mut server_configuration) = server_configurations.pop() {
    // Track which configurations should be merged into the current one
    let mut layers_indexes = Vec::new();
    // Check each remaining configuration in reverse order (from most to least specific)
    for sc2_index in (0..server_configurations.len()).rev() {
      // A bit complex matching logic to determine inheritance relationships...
      // sc1 is the current configuration, sc2 is the potential parent configuration
      let sc1 = &server_configuration.filters;
      let sc2 = &server_configurations[sc2_index].filters;

      // Determine if filter criteria match or if parent has wildcard (None) values
      // A None in parent (sc2) means it matches any value in child (sc1)
      let is_host_match = !sc2.is_host || sc1.is_host == sc2.is_host;
      let ports_match = sc2.port.is_none() || sc1.port == sc2.port;
      let ips_match = sc2.ip.is_none() || sc1.ip == sc2.ip;
      let hostnames_match = sc2.hostname.is_none() || sc1.hostname == sc2.hostname;
      let location_prefixes_match = sc2.location_prefix.is_none() || sc1.location_prefix == sc2.location_prefix;

      // Case 1: Child has error handler but parent doesn't, and all other filters match
      // This is for error handler inheritance
      let case1 = sc1.error_handler_status.is_some()
        && sc2.error_handler_status.is_none()
        && location_prefixes_match
        && hostnames_match
        && ips_match
        && ports_match
        && is_host_match;

      // Case 2: Location prefix inheritance
      // Child has location prefix but parent doesn't, and all other filters match
      let case2 = sc1.error_handler_status.is_none()
        && sc2.error_handler_status.is_none()
        && sc1.location_prefix.is_some()
        && sc2.location_prefix.is_none()
        && hostnames_match
        && ips_match
        && ports_match
        && is_host_match;

      // Case 3: Hostname inheritance
      // Child has hostname but parent doesn't, and all other filters match
      let case3 = sc1.error_handler_status.is_none()
        && sc2.error_handler_status.is_none()
        && sc1.location_prefix.is_none()
        && sc2.location_prefix.is_none()
        && sc1.hostname.is_some()
        && sc2.hostname.is_none()
        && ips_match
        && ports_match
        && is_host_match;

      // Case 4: IP address inheritance
      // Child has IP but parent doesn't, and all other filters match
      let case4 = sc1.error_handler_status.is_none()
        && sc2.error_handler_status.is_none()
        && sc1.location_prefix.is_none()
        && sc2.location_prefix.is_none()
        && sc1.hostname.is_none()
        && sc2.hostname.is_none()
        && sc1.ip.is_some()
        && sc2.ip.is_none()
        && ports_match
        && is_host_match;

      // Case 5: Port inheritance
      // Child has port but parent doesn't, and all other filters are None
      let case5 = sc1.error_handler_status.is_none()
        && sc2.error_handler_status.is_none()
        && sc1.location_prefix.is_none()
        && sc2.location_prefix.is_none()
        && sc1.hostname.is_none()
        && sc2.hostname.is_none()
        && sc1.ip.is_none()
        && sc2.ip.is_none()
        && sc1.port.is_some()
        && sc2.port.is_none()
        && is_host_match;

      // Case 6: Host block flag inheritance
      // Child has host block flag but parent doesn't, and all other filters are None
      let case6 = sc1.error_handler_status.is_none()
        && sc2.error_handler_status.is_none()
        && sc1.location_prefix.is_none()
        && sc2.location_prefix.is_none()
        && sc1.hostname.is_none()
        && sc2.hostname.is_none()
        && sc1.ip.is_none()
        && sc2.ip.is_none()
        && sc1.port.is_none()
        && sc2.port.is_none()
        && sc1.is_host
        && !sc2.is_host;

      // If any inheritance case matches, this configuration should inherit from the parent
      if case1 || case2 || case3 || case4 || case5 || case6 {
        layers_indexes.push(sc2_index);
      }
    }

    // Start with current configuration's entries
    let mut configuration_entries = server_configuration.entries;

    // Process all parent configurations that this one should inherit from
    for layer_index in layers_indexes {
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
            first_server_module_error.replace(error);
          }
          // Skip remaining modules for this configuration if validation fails
          break;
        }
      }
      // Only load module if its requirements are met
      if requirements_met {
        // Load the module with current configuration and global configuration
        match server_module.load_module(&server_configuration, global_configuration.as_ref()) {
          Ok(loaded_module) => server_configuration.modules.push(loaded_module),
          Err(error) => {
            // Store the first error encountered
            if first_server_module_error.is_none() {
              first_server_module_error.replace(error);
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
      location_prefix: location_prefix.map(String::from),
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
        location_prefix: location_prefix.map(|s| s.to_string()),
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
