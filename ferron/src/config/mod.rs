pub mod adapters;
pub mod processing;

use std::fmt::{Debug, Formatter};
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
        match_hostname(server_configuration.filters.hostname.as_deref(), hostname)
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
        c.filters.hostname == filters.hostname
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

  /// Finds the global server configuration
  pub fn find_global_configuration(&self) -> Option<Arc<ServerConfiguration>> {
    self
      .inner
      .iter()
      .find(|server_configuration| server_configuration.filters.is_global())
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
    self.hostname.is_none()
      && self.ip.is_none()
      && self.port.is_none()
      && self.location_prefix.is_none()
      && self.error_handler_status.is_none()
  }
}

impl Ord for ServerConfigurationFilters {
  fn cmp(&self, other: &Self) -> Ordering {
    if self.port.is_none() && other.port.is_some() {
      Ordering::Less
    } else if self.port.is_some() && other.port.is_none() {
      Ordering::Greater
    } else if self.ip.is_none() && other.ip.is_some() {
      Ordering::Less
    } else if self.ip.is_some() && other.ip.is_none() {
      Ordering::Greater
    } else if self.port.is_none() && other.port.is_some() {
      Ordering::Less
    } else if self.port.is_some() && other.port.is_none() {
      Ordering::Greater
    } else if self.location_prefix.is_none() && other.location_prefix.is_some() {
      Ordering::Less
    } else if self.location_prefix.is_some() && other.location_prefix.is_none() {
      Ordering::Greater
    } else if self.error_handler_status.is_none() && other.error_handler_status.is_some() {
      Ordering::Less
    } else if self.error_handler_status.is_some() && other.error_handler_status.is_none() {
      Ordering::Greater
    } else {
      Ordering::Equal
    }
  }
}

impl PartialOrd for ServerConfigurationFilters {
  fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
    Some(self.cmp(other))
  }
}

/// A specific list of Ferron server configuration entries
#[derive(Debug, Clone, PartialEq)]
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
#[derive(Debug, Clone, PartialEq)]
pub struct ServerConfigurationEntry {
  /// Values for the entry
  pub values: Vec<ServerConfigurationValue>,

  /// Props for the entry
  pub props: HashMap<String, ServerConfigurationValue>,
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
