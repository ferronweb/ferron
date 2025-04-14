use crate::ferron_common::ErrorLogger;
use pyo3::prelude::*;

use super::async_to_sync::async_to_sync;

#[pyclass]
pub struct WsgiErrorStream {
  error_logger: ErrorLogger,
}

impl WsgiErrorStream {
  pub fn new(error_logger: ErrorLogger) -> Self {
    Self { error_logger }
  }
}

#[pymethods]
impl WsgiErrorStream {
  fn write(&self, data: &str) -> PyResult<usize> {
    async_to_sync(
      self
        .error_logger
        .log(&format!("There was a WSGI error: {}", data)),
    );
    Ok(data.len())
  }

  fn writelines(&self, lines: Vec<String>) -> PyResult<()> {
    for line in lines {
      // Each `log_blocking` call prints a separate line
      async_to_sync(
        self
          .error_logger
          .log(&format!("There was a WSGI error: {}", line)),
      );
    }
    Ok(())
  }

  fn flush(&self) -> PyResult<()> {
    // This is a no-op function
    Ok(())
  }
}
