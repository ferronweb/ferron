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
    LogMessage { is_error, message }
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
