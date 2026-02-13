pub(super) mod conditionals;
mod tree;

use std::{collections::HashMap, net::IpAddr, sync::Arc};

use ferron_common::{
  config::{ErrorHandlerStatus, ServerConfiguration, ServerConfigurationFilters},
  modules::SocketData,
};
use hashlink::LinkedHashMap;

use crate::config::lookup::{
  conditionals::ConditionMatchData,
  tree::{ConfigFilterTree, ConfigFilterTreeSingleKey},
};

/// A type alias for the error handler status lookup structure, using Arc to allow for shared ownership of server configurations.
pub type ErrorHandlerStatusLookupWithConfiguration = ErrorHandlerStatusLookup<Arc<ServerConfiguration>>;

/// Converts the filters of a server configuration into a node key for the configuration filter tree.
#[inline]
fn convert_filters_to_node_key(filters: &ServerConfigurationFilters) -> Vec<ConfigFilterTreeSingleKey> {
  let mut node_key = Vec::new();
  if filters.is_host {
    node_key.push(ConfigFilterTreeSingleKey::IsHostConfiguration);

    if let Some(port) = filters.port {
      node_key.push(ConfigFilterTreeSingleKey::Port(port));
    }

    if let Some(ip) = filters.ip {
      if ip.is_loopback() {
        node_key.push(ConfigFilterTreeSingleKey::IsLocalhost);
      } else {
        match ip {
          IpAddr::V4(ipv4) => {
            for octet in ipv4.octets() {
              node_key.push(ConfigFilterTreeSingleKey::IPv4Octet(octet));
            }
          }
          IpAddr::V6(ipv6) => {
            for octet in ipv6.octets() {
              node_key.push(ConfigFilterTreeSingleKey::IPv6Octet(octet));
            }
          }
        }
      }
    }

    if let Some(hostname) = &filters.hostname {
      for part in hostname.split('.').rev() {
        if part.is_empty() {
          continue;
        }
        match part {
          "*" => node_key.push(ConfigFilterTreeSingleKey::HostDomainLevelWildcard),
          _ => node_key.push(ConfigFilterTreeSingleKey::HostDomainLevel(part.to_string())),
        }
      }
    }

    if let Some(conditions) = &filters.condition {
      let mut is_first = true;
      for segment in conditions.location_prefix.split("/") {
        if is_first || !segment.is_empty() {
          node_key.push(ConfigFilterTreeSingleKey::LocationSegment(segment.to_string()));
        }
        is_first = false;
      }
      for conditional in &conditions.conditionals {
        // Had to clone the conditional here because the ConfigFilterTreeSingleKey::Conditional variant needs
        // to own its data, and we can't move it out of the loop since it's borrowed from the filters struct...
        node_key.push(ConfigFilterTreeSingleKey::Conditional(conditional.clone()));
      }
    }
  }

  node_key
}

/// A lookup structure for error handler status codes, allowing for specific status codes,
/// a catch-all for any status code, and a default value if no other matches are found.
#[derive(Debug)]
pub struct ErrorHandlerStatusLookup<T> {
  default_value: Option<T>,
  catchall_value: Option<T>,
  status_code_values: HashMap<u16, T>,
}

impl<T> ErrorHandlerStatusLookup<T> {
  fn new() -> Self {
    Self {
      default_value: None,
      catchall_value: None,
      status_code_values: HashMap::new(),
    }
  }

  pub fn get(&self, status_code: u16) -> Option<&T> {
    self
      .status_code_values
      .get(&status_code)
      .or(self.catchall_value.as_ref())
  }

  pub fn get_default(&self) -> Option<&T> {
    self.default_value.as_ref()
  }

  fn insert(&mut self, status_code: Option<u16>, value: T) {
    if let Some(code) = status_code {
      self.status_code_values.insert(code, value);
    } else {
      self.catchall_value = Some(value);
    }
  }

  fn set_default(&mut self, value: T) {
    self.default_value = Some(value);
  }

  pub fn has_status_codes(&self) -> bool {
    self.catchall_value.is_some() || !self.status_code_values.is_empty()
  }
}

#[derive(Debug)]
pub struct ServerConfigurations {
  inner: ConfigFilterTree<ErrorHandlerStatusLookupWithConfiguration>,

  /// A vector of all host configurations, used for quickly finding host configurations without needing
  /// to traverse the configuration filter tree
  pub host_configs: Vec<Arc<ServerConfiguration>>,
}

impl ServerConfigurations {
  /// Creates the server configurations struct
  pub fn new(mut inner: Vec<ServerConfiguration>) -> Self {
    // Reverse the inner vector to ensure the location configurations are checked in the correct order
    inner.reverse();

    // Sort the configurations array by the specifity of configurations,
    // so that it will be possible to insert configurations into the configuration filter tree in a single pass,
    // without needing to backtrack to update parent nodes with default values from more specific child nodes
    inner.sort_by(|a, b| a.filters.cmp(&b.filters));

    let mut new_inner = ConfigFilterTree::new();
    let mut host_config_filters = LinkedHashMap::new();

    for config in inner {
      let config = Arc::new(config);
      if config.filters.is_host {
        if config.filters.condition.is_none() && config.filters.error_handler_status.is_none() {
          host_config_filters.insert(
            (config.filters.hostname.clone(), config.filters.port, config.filters.ip),
            config.clone(),
          );
        } else {
          host_config_filters
            .entry((config.filters.hostname.clone(), config.filters.port, config.filters.ip))
            .or_insert_with(|| config.clone());
        }
      }
      let error_handler_status = &config.filters.error_handler_status;
      let node_key = convert_filters_to_node_key(&config.filters);
      let parent_config_default_value = new_inner
        .get(node_key, None)
        .expect("configuration filter tree's get method shouldn't error out if no conditionals are checked against")
        .and_then(|lookup: &ErrorHandlerStatusLookup<_>| lookup.get_default().cloned());
      let node_key = convert_filters_to_node_key(&config.filters);
      let value_option = new_inner.insert_node(node_key);
      if value_option.is_none() {
        let mut new_value = ErrorHandlerStatusLookup::new();
        if let Some(parent_default) = parent_config_default_value {
          new_value.set_default(parent_default);
        }
        *value_option = Some(new_value);
      }
      if let Some(value) = value_option.as_mut() {
        match error_handler_status {
          Some(ErrorHandlerStatus::Status(status_code)) => value.insert(Some(*status_code), config),
          Some(ErrorHandlerStatus::Any) => value.insert(None, config),
          None => value.set_default(config),
        }
      }
    }

    Self {
      inner: new_inner,
      host_configs: host_config_filters.into_iter().map(|(_, config)| config).collect(),
    }
  }

  /// Finds a specific server configuration based on request parameters
  pub fn find_configuration(
    &self,
    request: &hyper::http::request::Parts,
    hostname: Option<&str>,
    socket_data: &SocketData,
  ) -> Result<Option<&ErrorHandlerStatusLookupWithConfiguration>, Box<dyn std::error::Error + Send + Sync>> {
    let mut node_key = Vec::new();
    node_key.push(ConfigFilterTreeSingleKey::IsHostConfiguration);
    node_key.push(ConfigFilterTreeSingleKey::Port(socket_data.local_addr.port()));
    let local_ip = socket_data.local_addr.ip();
    if local_ip.is_loopback() {
      node_key.push(ConfigFilterTreeSingleKey::IsLocalhost);
    } else {
      match local_ip {
        IpAddr::V4(ipv4) => {
          for octet in ipv4.octets() {
            node_key.push(ConfigFilterTreeSingleKey::IPv4Octet(octet));
          }
        }
        IpAddr::V6(ipv6) => {
          for octet in ipv6.octets() {
            node_key.push(ConfigFilterTreeSingleKey::IPv6Octet(octet));
          }
        }
      }
    }
    if let Some(hostname) = hostname {
      for part in hostname.split('.').rev() {
        if part.is_empty() {
          continue;
        }
        node_key.push(ConfigFilterTreeSingleKey::HostDomainLevel(part.to_string()))
      }
    }
    for part in request.uri.path().split("/") {
      node_key.push(ConfigFilterTreeSingleKey::LocationSegment(part.to_string()));
    }

    self
      .inner
      .get(node_key, Some(ConditionMatchData { request, socket_data }))
  }

  /// Finds the global server configuration (host or non-host)
  pub fn find_global_configuration(&self) -> Option<Arc<ServerConfiguration>> {
    self
      .inner
      .get(vec![ConfigFilterTreeSingleKey::IsHostConfiguration], None)
      .expect("configuration filter tree's get method shouldn't error out if no conditionals are checked against")
      .and_then(|lookup| lookup.get_default())
      .cloned()
  }
}
