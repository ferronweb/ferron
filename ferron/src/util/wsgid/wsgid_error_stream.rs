use std::sync::Arc;

use interprocess::unnamed_pipe::Sender;
use pyo3::prelude::*;
use tokio::sync::Mutex;

use super::ProcessPoolToServerMessage;
use crate::util::write_ipc_message;

/// A WSGI error stream
#[pyclass]
pub struct WsgidErrorStream {
  ipc_tx: Arc<Mutex<Sender>>,
}

impl WsgidErrorStream {
  /// Creates a new WSGI error stream
  pub fn new(ipc_tx: Arc<Mutex<Sender>>) -> Self {
    Self { ipc_tx }
  }
}

#[pymethods]
impl WsgidErrorStream {
  fn write(&self, data: &str) -> PyResult<usize> {
    write_ipc_message(
      &mut self.ipc_tx.blocking_lock(),
      &postcard::to_allocvec::<ProcessPoolToServerMessage>(&ProcessPoolToServerMessage {
        application_id: None,
        status_code: None,
        headers: None,
        body_chunk: None,
        error_log_line: Some(data.to_string()),
        error_message: None,
        requests_body_chunk: false,
      })
      .map_err(|e| anyhow::anyhow!(e.to_string()))?,
    )?;
    Ok(data.len())
  }

  fn writelines(&self, lines: Vec<String>) -> PyResult<()> {
    for line in lines {
      // Each `write_ipc_message` call prints a separate line
      write_ipc_message(
        &mut self.ipc_tx.blocking_lock(),
        &postcard::to_allocvec::<ProcessPoolToServerMessage>(&ProcessPoolToServerMessage {
          application_id: None,
          status_code: None,
          headers: None,
          body_chunk: None,
          error_log_line: Some(line),
          error_message: None,
          requests_body_chunk: false,
        })
        .map_err(|e| anyhow::anyhow!(e.to_string()))?,
      )?;
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
  use crate::util::read_ipc_message;

  #[test]
  fn test_write_sends_correct_data() {
    let (tx, mut rx) = interprocess::unnamed_pipe::pipe().unwrap();
    let stream = WsgidErrorStream::new(Arc::new(Mutex::new(tx)));
    let input = "error log line";
    let len = stream.write(input).unwrap();
    assert_eq!(len, input.len());

    let received = read_ipc_message(&mut rx).unwrap();
    let msg: ProcessPoolToServerMessage = postcard::from_bytes(&received).unwrap();
    assert_eq!(msg.error_log_line, Some(input.to_string()));
  }

  #[test]
  fn test_writelines_sends_each_line() {
    let (tx, mut rx) = interprocess::unnamed_pipe::pipe().unwrap();
    let stream = WsgidErrorStream::new(Arc::new(Mutex::new(tx)));
    let lines = vec!["line one".into(), "line two".into()];
    stream.writelines(lines.clone()).unwrap();

    for line in lines {
      let received = read_ipc_message(&mut rx).unwrap();
      let msg: ProcessPoolToServerMessage = postcard::from_bytes(&received).unwrap();
      assert_eq!(msg.error_log_line, Some(line.clone()));
    }
  }
}
