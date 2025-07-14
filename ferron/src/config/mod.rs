pub mod adapters;
pub mod processing;

use std::fmt::{Debug, Formatter};
use std::hash::Hasher;
use std::net::IpAddr;
use std::sync::Arc;
use std::{cmp::Ordering, collections::HashMap};

use crate::modules::Module;
use crate::util::{match_hostname, match_location};

/// The struct containing all the Ferron server configurations
#[derive(Debug)]
pub struct ServerConfigurations {
  /// Vector of configurations
  pub inner: Vec<Arc<ServerConfiguration>>,
}

impl ServerConfigurations {
  /// Creates the server configurations struct
  pub fn new(mut inner: Vec<ServerConfiguration>) -> Self {
    // Reverse the inner vector to ensure the location configurations are checked in the correct order
    inner.reverse();

    // Sort the configurations array by the specifity of configurations, so that it will be possible to find the configuration
    inner.sort_by(|a, b| a.filters.cmp(&b.filters));
    Self {
      inner: inner.into_iter().map(Arc::new).collect(),
    }
  }

  /// Finds a specific server configuration based on request parameters
  pub fn find_configuration(
    &self,
    request_path: &str,
    hostname: Option<&str>,
    ip: IpAddr,
    port: u16,
  ) -> Option<Arc<ServerConfiguration>> {
    // The inner array is sorted by specifity, so it's easier to find the configurations.
    // If it was not sorted, we would need to implement the specifity...
    // Also, the approach mentioned in the line above might be slower...
    // But there is one thing we're wondering: so many logical operators???
    self
      .inner
      .iter()
      .rev()
      .find(|&server_configuration| {
        server_configuration.filters.is_host
          && match_hostname(server_configuration.filters.hostname.as_deref(), hostname)
          && (server_configuration.filters.ip.is_none()
            || server_configuration.filters.ip == Some(ip))
          && (server_configuration.filters.port.is_none()
            || server_configuration.filters.port == Some(port))
          && server_configuration
            .filters
            .location_prefix
            .as_deref()
            .is_none_or(|lp| match_location(lp, request_path))
          && server_configuration.filters.error_handler_status.is_none()
      })
      .cloned()
  }

  /// Finds the server error configuration based on configuration filters
  pub fn find_error_configuration(
    &self,
    filters: &ServerConfigurationFilters,
    status_code: u16,
  ) -> Option<Arc<ServerConfiguration>> {
    self
      .inner
      .iter()
      .rev()
      .find(|c| {
        c.filters.is_host
          && c.filters.hostname == filters.hostname
          && c.filters.ip == filters.ip
          && c.filters.port == filters.port
          && (c.filters.location_prefix.is_none()
            || c.filters.location_prefix == filters.location_prefix)
          && !c.filters.error_handler_status.as_ref().is_none_or(|s| {
            !(matches!(s, ErrorHandlerStatus::Any)
              || matches!(s, ErrorHandlerStatus::Status(x) if *x == status_code))
          })
      })
      .cloned()
  }

  /// Finds the global server configuration (host or non-host)
  pub fn find_global_configuration(&self) -> Option<Arc<ServerConfiguration>> {
    self
      .inner
      .iter()
      .find(|server_configuration| {
        server_configuration.filters.is_global()
          || server_configuration.filters.is_global_non_host()
      })
      .cloned()
  }
}

/// A specific Ferron server configuration
#[derive(Clone)]
pub struct ServerConfiguration {
  /// Entries for the configuration
  pub entries: HashMap<String, ServerConfigurationEntries>,

  /// Configuration filters
  pub filters: ServerConfigurationFilters,

  /// Loaded modules
  pub modules: Vec<Arc<dyn Module + Send + Sync>>,
}

impl Debug for ServerConfiguration {
  fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("ServerConfiguration")
      .field("entries", &self.entries)
      .field("filters", &self.filters)
      .finish()
  }
}

/// A error handler status code
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErrorHandlerStatus {
  /// Any status code
  Any,

  /// Specific status code
  Status(u16),
}

/// A Ferron server configuration filter
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerConfigurationFilters {
  /// Whether the configuration represents a host block
  pub is_host: bool,

  /// The hostname
  pub hostname: Option<String>,

  /// The IP address
  pub ip: Option<IpAddr>,

  /// The port
  pub port: Option<u16>,

  /// The location prefix
  pub location_prefix: Option<String>,

  /// The error handler status code
  pub error_handler_status: Option<ErrorHandlerStatus>,
}

impl ServerConfigurationFilters {
  /// Checks if the server configuration is global
  pub fn is_global(&self) -> bool {
    self.is_host
      && self.hostname.is_none()
      && self.ip.is_none()
      && self.port.is_none()
      && self.location_prefix.is_none()
      && self.error_handler_status.is_none()
  }

  /// Checks if the server configuration is global and doesn't represent a host block
  pub fn is_global_non_host(&self) -> bool {
    !self.is_host
  }
}

impl Ord for ServerConfigurationFilters {
  fn cmp(&self, other: &Self) -> Ordering {
    self
      .is_host
      .cmp(&other.is_host)
      .then_with(|| self.port.is_some().cmp(&other.port.is_some()))
      .then_with(|| self.ip.is_some().cmp(&other.ip.is_some()))
      .then_with(|| self.hostname.is_some().cmp(&other.hostname.is_some()))
      .then_with(|| {
        self
          .location_prefix
          .is_some()
          .cmp(&other.location_prefix.is_some())
      })
      .then_with(|| {
        self
          .error_handler_status
          .is_some()
          .cmp(&other.error_handler_status.is_some())
      })
  }
}

impl PartialOrd for ServerConfigurationFilters {
  fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
    Some(self.cmp(other))
  }
}

/// A specific list of Ferron server configuration entries
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServerConfigurationEntries {
  /// Vector of configuration entries
  pub inner: Vec<ServerConfigurationEntry>,
}

impl ServerConfigurationEntries {
  /// Extracts one value from the entry list
  pub fn get_value(&self) -> Option<&ServerConfigurationValue> {
    self
      .inner
      .last()
      .and_then(|last_vector| last_vector.values.first())
  }

  /// Extracts one entry from the entry list
  pub fn get_entry(&self) -> Option<&ServerConfigurationEntry> {
    self.inner.last()
  }

  /// Extracts a vector of values from the entry list
  pub fn get_values(&self) -> Vec<&ServerConfigurationValue> {
    let mut iterator: Box<dyn Iterator<Item = &ServerConfigurationValue>> =
      Box::new(vec![].into_iter());
    for entry in &self.inner {
      iterator = Box::new(iterator.chain(entry.values.iter()));
    }
    iterator.collect()
  }
}

/// A specific Ferron server configuration entry
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfigurationEntry {
  /// Values for the entry
  pub values: Vec<ServerConfigurationValue>,

  /// Props for the entry
  pub props: HashMap<String, ServerConfigurationValue>,
}

impl std::hash::Hash for ServerConfigurationEntry {
  fn hash<H: Hasher>(&self, state: &mut H) {
    // Hash the values vector
    self.values.hash(state);

    // For HashMap, we need to hash in a deterministic order
    // since HashMap iteration order is not guaranteed
    let mut props_vec: Vec<_> = self.props.iter().collect();
    props_vec.sort_by(|a, b| a.0.cmp(b.0)); // Sort by key

    // Hash the length first, then each key-value pair
    props_vec.len().hash(state);
    for (key, value) in props_vec {
      key.hash(state);
      value.hash(state);
    }
  }
}

/// A specific Ferron server configuration value
#[derive(Debug, Clone, PartialOrd)]
pub enum ServerConfigurationValue {
  /// A string
  String(String),

  /// A non-float number
  Integer(i128),

  /// A floating point number
  Float(f64),

  /// A boolean
  Bool(bool),

  /// The null value
  Null,
}

impl std::hash::Hash for ServerConfigurationValue {
  fn hash<H: Hasher>(&self, state: &mut H) {
    match self {
      Self::String(s) => {
        0u8.hash(state);
        s.hash(state);
      }
      Self::Integer(i) => {
        1u8.hash(state);
        i.hash(state);
      }
      Self::Float(f) => {
        2u8.hash(state);
        // Convert to bits for consistent hashing
        // Handle NaN by using a consistent bit pattern
        if f.is_nan() {
          f64::NAN.to_bits().hash(state);
        } else {
          f.to_bits().hash(state);
        }
      }
      Self::Bool(b) => {
        3u8.hash(state);
        b.hash(state);
      }
      Self::Null => {
        4u8.hash(state);
      }
    }
  }
}

impl ServerConfigurationValue {
  /// Checks if the value is a string
  pub fn is_string(&self) -> bool {
    matches!(self, Self::String(..))
  }

  /// Checks if the value is a non-float number
  pub fn is_integer(&self) -> bool {
    matches!(self, Self::Integer(..))
  }

  /// Checks if the value is a floating point number
  #[allow(dead_code)]
  pub fn is_float(&self) -> bool {
    matches!(self, Self::Float(..))
  }

  /// Checks if the value is a boolean
  pub fn is_bool(&self) -> bool {
    matches!(self, Self::Bool(..))
  }

  /// Checks if the value is a null value
  pub fn is_null(&self) -> bool {
    matches!(self, Self::Null)
  }

  /// Extracts a `&str` from the value
  pub fn as_str(&self) -> Option<&str> {
    use ServerConfigurationValue::*;
    match self {
      String(s) => Some(s),
      _ => None,
    }
  }

  /// Extracts a `i128` from the value
  pub fn as_i128(&self) -> Option<i128> {
    use ServerConfigurationValue::*;
    match self {
      Integer(i) => Some(*i),
      _ => None,
    }
  }

  /// Extracts a `f64` from the value
  #[allow(dead_code)]
  pub fn as_f64(&self) -> Option<f64> {
    match self {
      Self::Float(i) => Some(*i),
      _ => None,
    }
  }

  /// Extracts a `bool` from the value
  pub fn as_bool(&self) -> Option<bool> {
    if let Self::Bool(v) = self {
      Some(*v)
    } else {
      None
    }
  }
}

impl Eq for ServerConfigurationValue {}

impl PartialEq for ServerConfigurationValue {
  fn eq(&self, other: &Self) -> bool {
    match (self, other) {
      (Self::Bool(left), Self::Bool(right)) => left == right,
      (Self::Integer(left), Self::Integer(right)) => left == right,
      (Self::Float(left), Self::Float(right)) => {
        let left = if left == &f64::NEG_INFINITY {
          -f64::MAX
        } else if left == &f64::INFINITY {
          f64::MAX
        } else if left.is_nan() {
          0.0
        } else {
          *left
        };

        let right = if right == &f64::NEG_INFINITY {
          -f64::MAX
        } else if right == &f64::INFINITY {
          f64::MAX
        } else if right.is_nan() {
          0.0
        } else {
          *right
        };

        left == right
      }
      (Self::String(left), Self::String(right)) => left == right,
      _ => core::mem::discriminant(self) == core::mem::discriminant(other),
    }
  }
}
