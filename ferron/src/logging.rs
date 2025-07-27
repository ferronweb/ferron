pub use ferron_common::logging::*;

use std::{cmp::Ordering, collections::HashMap, net::IpAddr, sync::Arc};

use async_channel::{Receiver, Sender};

use crate::util::{is_localhost, match_hostname};

/// A logger filter
#[derive(Clone, Eq, PartialEq, Hash)]
pub struct LoggerFilter {
  /// The hostname
  pub hostname: Option<String>,

  /// The IP address
  pub ip: Option<IpAddr>,

  /// The port
  pub port: Option<u16>,
}

impl Ord for LoggerFilter {
  fn cmp(&self, other: &Self) -> Ordering {
    self
      .port
      .is_some()
      .cmp(&other.port.is_some())
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
  }
}

impl PartialOrd for LoggerFilter {
  fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
    Some(self.cmp(other))
  }
}

impl LoggerFilter {
  /// Checks if the logger is global
  pub fn is_global(&self) -> bool {
    self.hostname.is_none() && self.ip.is_none() && self.port.is_none()
  }
}

/// A builder for the struct that contains loggers as specified by the server configuration
pub struct LoggersBuilder {
  pub inner: HashMap<LoggerFilter, (Sender<LogMessage>, Receiver<LogMessage>)>,
}

impl LoggersBuilder {
  /// Creates a new `LoggersBuilder` instance.
  pub fn new() -> Self {
    Self { inner: HashMap::new() }
  }

  /// Adds a new logger, if there isn't already a logger.
  pub fn add(
    &mut self,
    filter: LoggerFilter,
    logger: (Sender<LogMessage>, Receiver<LogMessage>),
  ) -> (Sender<LogMessage>, Receiver<LogMessage>) {
    if let Some(existing_logger) = self.inner.get(&filter) {
      existing_logger.clone()
    } else {
      let new_logger = logger.clone();
      self.inner.insert(filter, logger);
      new_logger
    }
  }

  /// Consumes the builder and returns a `Loggers` instance.
  #[allow(dead_code)]
  pub fn build(self) -> Loggers {
    let mut inner_vector = self.inner.into_iter().map(|(k, v)| (k, v.0)).collect::<Vec<_>>();
    inner_vector.sort_by(|a, b| a.0.cmp(&b.0));
    Loggers {
      inner: Arc::new(inner_vector),
    }
  }

  /// Returns a `Loggers` instance from the builder.
  pub fn build_borrowed(&self) -> Loggers {
    let mut inner_vector = self
      .inner
      .iter()
      .map(|(k, v)| (k.clone(), v.0.clone()))
      .collect::<Vec<_>>();
    inner_vector.reverse();
    inner_vector.sort_by(|a, b| a.0.cmp(&b.0));
    Loggers {
      inner: Arc::new(inner_vector),
    }
  }
}

pub struct Loggers {
  inner: Arc<Vec<(LoggerFilter, Sender<LogMessage>)>>,
}

impl Loggers {
  /// Finds the global logger
  pub fn find_global_logger(&self) -> Option<Sender<LogMessage>> {
    self
      .inner
      .iter()
      .find(|logger| logger.0.is_global())
      .map(|logger| &logger.1)
      .cloned()
  }

  /// Finds a specific logger based on request parameters
  pub fn find_logger(&self, hostname: Option<&str>, ip: IpAddr, port: u16) -> Option<Sender<LogMessage>> {
    // The inner array is sorted by specifity, so it's easier to find the configurations.
    // If it was not sorted, we would need to implement the specifity...
    // Also, the approach mentioned in the line above might be slower...
    // But there is one thing we're wondering: so many logical operators???
    self
      .inner
      .iter()
      .rev()
      .find(|&logger| {
        match_hostname(logger.0.hostname.as_deref(), hostname)
          && ((logger.0.ip.is_none() && (!is_localhost(logger.0.ip.as_ref(), logger.0.hostname.as_deref())
            || ip.to_canonical().is_loopback()))  // With special `localhost` check
          || logger.0.ip == Some(ip))
          && (logger.0.port.is_none() || logger.0.port == Some(port))
      })
      .map(|logger| &logger.1)
      .cloned()
  }
}

impl Clone for Loggers {
  /// Clone a `Loggers`.
  fn clone(&self) -> Self {
    Self {
      inner: self.inner.clone(),
    }
  }
}
