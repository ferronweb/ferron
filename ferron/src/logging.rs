use std::{cmp::Ordering, collections::HashMap, net::IpAddr, sync::Arc};

use async_channel::{Receiver, Sender};

use crate::util::match_hostname;

/// Represents a log message with its content and error status.
pub struct LogMessage {
  is_error: bool,
  message: String,
}

impl LogMessage {
  /// Creates a new `LogMessage` instance.
  ///
  /// # Parameters
  ///
  /// - `message`: The content of the log message.
  /// - `is_error`: A boolean indicating whether the message is an error (`true`) or not (`false`).
  ///
  /// # Returns
  ///
  /// A `LogMessage` object containing the specified message and error status.
  pub fn new(message: String, is_error: bool) -> Self {
    Self { is_error, message }
  }

  /// Consumes the `LogMessage` and returns its components.
  ///
  /// # Returns
  ///
  /// A tuple containing:
  /// - `String`: The content of the log message.
  /// - `bool`: A boolean indicating whether the message is an error.
  pub fn get_message(self) -> (String, bool) {
    (self.message, self.is_error)
  }
}

/// Facilitates logging of error messages through a provided logger sender.
pub struct ErrorLogger {
  logger: Option<Sender<LogMessage>>,
}

impl ErrorLogger {
  /// Creates a new `ErrorLogger` instance.
  ///
  /// # Parameters
  ///
  /// - `logger`: A `Sender<LogMessage>` used for sending log messages.
  ///
  /// # Returns
  ///
  /// A new `ErrorLogger` instance associated with the provided logger.
  pub fn new(logger: Sender<LogMessage>) -> Self {
    Self { logger: Some(logger) }
  }

  /// Creates a new `ErrorLogger` instance without any underlying logger.
  ///
  /// # Returns
  ///
  /// A new `ErrorLogger` instance not associated with any logger.
  pub fn without_logger() -> Self {
    Self { logger: None }
  }

  /// Logs an error message asynchronously.
  ///
  /// # Parameters
  ///
  /// - `message`: A string slice containing the error message to be logged.
  ///
  /// # Examples
  ///
  /// ```
  /// # use crate::ferron_common::ErrorLogger;
  /// # #[tokio::main]
  /// # async fn main() {
  /// let (tx, mut rx) = async_channel::bounded(100);
  /// let logger = ErrorLogger::new(tx);
  /// logger.log("An error occurred").await;
  /// # }
  /// ```
  pub async fn log(&self, message: &str) {
    if let Some(logger) = &self.logger {
      logger
        .send(LogMessage::new(String::from(message), true))
        .await
        .unwrap_or_default();
    }
  }
}

impl Clone for ErrorLogger {
  /// Clone a `ErrorLogger`.
  ///
  /// # Returns
  ///
  /// A cloned `ErrorLogger` instance
  fn clone(&self) -> Self {
    Self {
      logger: self.logger.clone(),
    }
  }
}

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
      .then_with(|| self.hostname.is_some().cmp(&other.hostname.is_some()))
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
          && (logger.0.ip.is_none() || logger.0.ip == Some(ip))
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
