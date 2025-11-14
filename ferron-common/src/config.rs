use std::fmt::{Debug, Display, Formatter};
use std::hash::Hasher;
use std::net::IpAddr;
use std::sync::Arc;
use std::{cmp::Ordering, collections::HashMap};

use fancy_regex::Regex;

use crate::modules::Module;
use crate::util::IpBlockList;

/// Conditional data
#[non_exhaustive]
#[repr(u8)]
#[derive(Clone, Debug)]
pub enum ConditionalData {
  IsRemoteIp(IpBlockList),
  IsForwardedFor(IpBlockList),
  IsNotRemoteIp(IpBlockList),
  IsNotForwardedFor(IpBlockList),
  IsEqual(String, String),
  IsNotEqual(String, String),
  IsRegex(String, Regex),
  IsNotRegex(String, Regex),
  IsRego(Arc<regorus::Engine>),
}

impl PartialEq for ConditionalData {
  fn eq(&self, other: &Self) -> bool {
    match (self, other) {
      (Self::IsRemoteIp(v1), Self::IsRemoteIp(v2)) => v1 == v2,
      (Self::IsForwardedFor(v1), Self::IsForwardedFor(v2)) => v1 == v2,
      (Self::IsNotRemoteIp(v1), Self::IsNotRemoteIp(v2)) => v1 == v2,
      (Self::IsNotForwardedFor(v1), Self::IsNotForwardedFor(v2)) => v1 == v2,
      (Self::IsEqual(v1, v2), Self::IsEqual(v3, v4)) => v1 == v3 && v2 == v4,
      (Self::IsNotEqual(v1, v2), Self::IsNotEqual(v3, v4)) => v1 == v3 && v2 == v4,
      (Self::IsRegex(v1, v2), Self::IsRegex(v3, v4)) => v1 == v3 && v2.as_str() == v4.as_str(),
      (Self::IsNotRegex(v1, v2), Self::IsNotRegex(v3, v4)) => v1 == v3 && v2.as_str() == v4.as_str(),
      (Self::IsRego(v1), Self::IsRego(v2)) => v1.get_policies().ok() == v2.get_policies().ok(),
      _ => false,
    }
  }
}

impl Eq for ConditionalData {}

impl Ord for ConditionalData {
  fn cmp(&self, other: &Self) -> Ordering {
    match (self, other) {
      (Self::IsRemoteIp(v1), Self::IsRemoteIp(v2)) => v1.cmp(v2),
      (Self::IsForwardedFor(v1), Self::IsForwardedFor(v2)) => v1.cmp(v2),
      (Self::IsNotRemoteIp(v1), Self::IsNotRemoteIp(v2)) => v1.cmp(v2),
      (Self::IsNotForwardedFor(v1), Self::IsNotForwardedFor(v2)) => v1.cmp(v2),
      (Self::IsEqual(v1, v2), Self::IsEqual(v3, v4)) => v1.cmp(v3).then(v2.cmp(v4)),
      (Self::IsNotEqual(v1, v2), Self::IsNotEqual(v3, v4)) => v1.cmp(v3).then(v2.cmp(v4)),
      (Self::IsRegex(v1, v2), Self::IsRegex(v3, v4)) => v1.cmp(v3).then(v2.as_str().cmp(v4.as_str())),
      (Self::IsNotRegex(v1, v2), Self::IsNotRegex(v3, v4)) => v1.cmp(v3).then(v2.as_str().cmp(v4.as_str())),
      (Self::IsRego(v1), Self::IsRego(v2)) => v1.get_policies().ok().cmp(&v2.get_policies().ok()),
      _ => {
        // SAFETY: See https://doc.rust-lang.org/core/mem/fn.discriminant.html
        let discriminant_self = unsafe { *<*const _>::from(self).cast::<u8>() };
        let discriminant_other = unsafe { *<*const _>::from(other).cast::<u8>() };
        discriminant_self.cmp(&discriminant_other)
      }
    }
  }
}

impl PartialOrd for ConditionalData {
  fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
    Some(self.cmp(other))
  }
}

fn count_logical_slashes(s: &str) -> usize {
  if s.is_empty() {
    // Input is empty, zero slashes
    return 0;
  }
  let trimmed = s.trim_end_matches('/');
  if trimmed.is_empty() {
    // Trimmed input is empty, but the original wasn't, probably input with only slashes
    return 0;
  }

  let mut count = 0;
  let mut prev_was_slash = false;

  for ch in trimmed.chars() {
    if ch == '/' {
      if !prev_was_slash {
        count += 1;
        prev_was_slash = true;
      }
    } else {
      prev_was_slash = false;
    }
  }

  count
}

/// The struct containing conditions
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conditions {
  /// The location prefix
  pub location_prefix: String,

  /// The conditionals
  pub conditionals: Vec<Conditional>,
}

impl Ord for Conditions {
  fn cmp(&self, other: &Self) -> Ordering {
    count_logical_slashes(&self.location_prefix)
      .cmp(&count_logical_slashes(&other.location_prefix))
      .then_with(|| self.conditionals.len().cmp(&other.conditionals.len()))
  }
}

impl PartialOrd for Conditions {
  fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
    Some(self.cmp(other))
  }
}

/// The enum containing a conditional
#[derive(Clone, Debug, PartialOrd, Ord, PartialEq, Eq)]
pub enum Conditional {
  /// "if" condition
  If(Vec<ConditionalData>),

  /// "if_not" condition
  IfNot(Vec<ConditionalData>),
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
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
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

  /// The conditions
  pub condition: Option<Conditions>,

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
      && self.condition.is_none()
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
      .then_with(|| {
        self
          .hostname
          .as_ref()
          .map(|h| !h.starts_with("*."))
          .cmp(&other.hostname.as_ref().map(|h| !h.starts_with("*.")))
      }) // Take wildcard hostnames into account
      .then_with(|| {
        self
          .hostname
          .as_ref()
          .map(|h| h.trim_end_matches('.').chars().filter(|c| *c == '.').count())
          .cmp(
            &other
              .hostname
              .as_ref()
              .map(|h| h.trim_end_matches('.').chars().filter(|c| *c == '.').count()),
          )
      }) // Take also amount of dots in hostnames (domain level) into account
      .then_with(|| self.condition.cmp(&other.condition)) // Use `cmp` method for `Ord` trait implemented for `Condition`
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

impl Display for ServerConfigurationFilters {
  fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
    if !self.is_host {
      write!(f, "\"globals\" block")
    } else {
      let mut blocks = Vec::new();
      if self.ip.is_some() || self.hostname.is_some() || self.port.is_some() {
        let mut name = String::new();
        if let Some(hostname) = &self.hostname {
          name.push_str(hostname);
          if let Some(ip) = self.ip {
            name.push_str(&format!("({ip})"));
          }
        } else if let Some(ip) = self.ip {
          name.push_str(&ip.to_string());
        }
        if let Some(port) = self.port {
          name.push_str(&format!(":{port}"));
        }
        if !name.is_empty() {
          blocks.push(format!("\"{name}\" host block",));
        }
      }
      if let Some(condition) = &self.condition {
        let mut name = String::new();
        if !condition.location_prefix.is_empty() {
          name.push_str(&format!("\"{}\" location", condition.location_prefix));
        }
        if !condition.conditionals.is_empty() {
          if !name.is_empty() {
            name.push_str(" and ");
          }
          name.push_str("some conditional blocks");
        } else {
          name.push_str(" block");
        }
        blocks.push(name);
      }
      if let Some(error_handler_status) = &self.error_handler_status {
        match error_handler_status {
          ErrorHandlerStatus::Any => blocks.push("\"error_status\" block".to_string()),
          ErrorHandlerStatus::Status(status) => blocks.push(format!("\"error_status {status}\" block")),
        }
      }
      write!(f, "{}", blocks.join(" -> "))
    }
  }
}

/// A specific list of Ferron server configuration entries
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct ServerConfigurationEntries {
  /// Vector of configuration entries
  pub inner: Vec<ServerConfigurationEntry>,
}

impl ServerConfigurationEntries {
  /// Extracts one value from the entry list
  pub fn get_value(&self) -> Option<&ServerConfigurationValue> {
    self.inner.last().and_then(|last_vector| last_vector.values.first())
  }

  /// Extracts one entry from the entry list
  pub fn get_entry(&self) -> Option<&ServerConfigurationEntry> {
    self.inner.last()
  }

  /// Extracts a vector of values from the entry list
  pub fn get_values(&self) -> Vec<&ServerConfigurationValue> {
    let mut iterator: Box<dyn Iterator<Item = &ServerConfigurationValue>> = Box::new(vec![].into_iter());
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
