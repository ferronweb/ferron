use ferron_common::logging::ErrorLogger;
use pyo3::prelude::*;

/// A WSGI error stream
#[pyclass]
pub struct WsgiErrorStream {
  error_logger: ErrorLogger,
}

impl WsgiErrorStream {
  /// Creates a new WSGI error stream
  pub fn new(error_logger: ErrorLogger) -> Self {
    Self { error_logger }
  }
}

#[pymethods]
impl WsgiErrorStream {
  fn write(&self, data: &str) -> PyResult<usize> {
    futures_executor::block_on(self.error_logger.log(&format!("There was a WSGI error: {}", data)));
    Ok(data.len())
  }

  fn writelines(&self, lines: Vec<String>) -> PyResult<()> {
    for line in lines {
      // Each `log_blocking` call prints a separate line
      futures_executor::block_on(self.error_logger.log(&format!("There was a WSGI error: {}", line)));
    }
    Ok(())
  }

  fn flush(&self) -> PyResult<()> {
    // This is a no-op function
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use async_channel::{bounded, Receiver};
  use ferron_common::logging::LogMessage;

  #[test]
  fn test_write_logs_error_message() {
    let (tx, rx): (async_channel::Sender<LogMessage>, Receiver<LogMessage>) = bounded(100);
    let error_logger = ErrorLogger::new(tx);
    let wsgi_error_stream = WsgiErrorStream::new(error_logger);

    let data = "Some error occurred";
    let result = wsgi_error_stream.write(data).unwrap();

    assert_eq!(result, data.len());

    // Check if the log message was sent
    let log_message = rx.recv_blocking().unwrap();
    let (message, is_error) = log_message.get_message();
    assert_eq!(message, format!("There was a WSGI error: {}", data));
    assert!(is_error);
  }

  #[test]
  fn test_writelines_logs_multiple_error_messages() {
    let (tx, rx): (async_channel::Sender<LogMessage>, Receiver<LogMessage>) = bounded(100);
    let error_logger = ErrorLogger::new(tx);
    let wsgi_error_stream = WsgiErrorStream::new(error_logger);

    let lines = vec!["Error 1".to_string(), "Error 2".to_string()];
    let result = wsgi_error_stream.writelines(lines.clone());

    assert!(result.is_ok());

    // Check if the log messages were sent
    for line in lines {
      let log_message = rx.recv_blocking().unwrap();
      let (message, is_error) = log_message.get_message();
      assert_eq!(message, format!("There was a WSGI error: {}", line));
      assert!(is_error);
    }
  }

  #[test]
  fn test_flush_no_op() {
    let (tx, _rx): (async_channel::Sender<LogMessage>, Receiver<LogMessage>) = bounded(100);
    let error_logger = ErrorLogger::new(tx);
    let wsgi_error_stream = WsgiErrorStream::new(error_logger);

    let result = wsgi_error_stream.flush();
    assert!(result.is_ok());
  }
}
