use async_channel::Sender;

use crate::observability::TraceCtx;

/// Represents a log message with its content and error status.
#[derive(Clone)]
pub struct LogMessage {
  is_error: bool,
  message: String,
  trace_ctx: Option<TraceCtx>,
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
    Self {
      is_error,
      message,
      trace_ctx: None,
    }
  }

  /// Attaches trace context to a metric
  ///
  /// # Parameters
  ///
  /// - `trace_ctx`: The trace context to attach to this log message.
  pub fn attach_trace_ctx(&mut self, trace_ctx: TraceCtx) {
    self.trace_ctx = Some(trace_ctx);
  }

  /// Obtains the `TraceCtx` containing the trace context
  ///
  /// # Returns
  ///
  /// An optional `TraceCtx` containing the trace context for a log message.
  pub fn trace_ctx(&self) -> Option<&TraceCtx> {
    self.trace_ctx.as_ref()
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
  loggers: Vec<Sender<LogMessage>>,
  trace_ctx: Option<TraceCtx>,
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
      loggers: vec![logger],
      trace_ctx: None,
    }
  }

  /// Creates a new `ErrorLogger` instance with multiple loggers.
  ///
  /// # Parameters
  ///
  /// - `loggers`: A vector of `Sender<LogMessage>` used for sending log messages.
  ///
  /// # Returns
  ///
  /// A new `ErrorLogger` instance associated with multiple provided loggers.
  pub fn new_multiple(loggers: Vec<Sender<LogMessage>>) -> Self {
    Self {
      loggers,
      trace_ctx: None,
    }
  }

  /// Creates a new `ErrorLogger` instance without any underlying logger.
  ///
  /// # Returns
  ///
  /// A new `ErrorLogger` instance not associated with any logger.
  pub fn without_logger() -> Self {
    Self {
      loggers: vec![],
      trace_ctx: None,
    }
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
  /// # use ferron_common::logging::ErrorLogger;
  /// # #[tokio::main]
  /// # async fn main() {
  /// let (tx, mut rx) = async_channel::bounded(100);
  /// let logger = ErrorLogger::new(tx);
  /// logger.log("An error occurred").await;
  /// # }
  /// ```
  pub async fn log(&self, message: &str) {
    let mut msg = LogMessage::new(String::from(message), true);
    if let Some(trace_ctx) = self.trace_ctx.clone() {
      msg.attach_trace_ctx(trace_ctx);
    }
    for logger in &self.loggers {
      logger.send(msg.clone()).await.unwrap_or_default();
    }
  }

  /// Attaches trace context to an error logger
  ///
  /// # Parameters
  ///
  /// - `trace_ctx`: A trace context to attach to the log.
  pub fn attach_trace_ctx(&mut self, trace_ctx: TraceCtx) {
    self.trace_ctx = Some(trace_ctx)
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
      loggers: self.loggers.clone(),
      trace_ctx: self.trace_ctx.clone(),
    }
  }
}
