use async_channel::Sender;

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
    Self {
      logger: Some(logger),
    }
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
