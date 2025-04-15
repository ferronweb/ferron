use std::pin::Pin;

use crate::ferron_util::async_to_sync::async_to_sync;
use pyo3::prelude::*;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt};

#[pyclass]
pub struct WsgiInputStream {
  body_reader: Pin<Box<dyn AsyncBufRead + Send + Sync>>,
}

impl WsgiInputStream {
  pub fn new(body_reader: impl AsyncBufRead + Send + Sync + 'static) -> Self {
    Self {
      body_reader: Box::pin(body_reader),
    }
  }
}

#[pymethods]
impl WsgiInputStream {
  fn read(&mut self, size: usize) -> PyResult<Vec<u8>> {
    let mut buffer = vec![0u8; size];
    let read_bytes = async_to_sync(self.body_reader.read(&mut buffer))?;
    Ok(buffer[0..read_bytes].to_vec())
  }

  #[pyo3(signature = (size=-1))]
  fn readline(&mut self, size: Option<isize>) -> PyResult<Vec<u8>> {
    let mut buffer = Vec::new();
    let size = if size.is_none_or(|s| s < 0) {
      None
    } else {
      size.map(|s| s as usize)
    };
    loop {
      let reader_buffer = async_to_sync(self.body_reader.fill_buf())?.to_vec();
      if reader_buffer.is_empty() {
        break;
      }
      if let Some(eol_position) = reader_buffer.iter().position(|&char| char == b'\n') {
        buffer.extend_from_slice(
          &reader_buffer[0..size.map_or(eol_position + 1, |size| {
            std::cmp::min(size, eol_position + 1)
          })],
        );
        self.body_reader.consume(eol_position + 1);
        break;
      } else {
        buffer.extend_from_slice(&reader_buffer[0..size.unwrap_or(reader_buffer.len())]);
        self.body_reader.consume(reader_buffer.len());
      }
    }
    Ok(buffer)
  }

  #[pyo3(signature = (hint=-1))]
  fn readlines(&mut self, hint: Option<isize>) -> PyResult<Vec<Vec<u8>>> {
    let mut total_bytes = 0;
    let mut lines = Vec::new();
    let hint: Option<_> = if hint.is_none_or(|s| s < 0) {
      None
    } else {
      hint.map(|s| s as usize)
    };
    loop {
      let mut line = Vec::new();
      let bytes_read = async_to_sync(self.body_reader.read_until(b'\n', &mut line))?;
      if bytes_read == 0 {
        break;
      }
      total_bytes += line.len();
      lines.push(line);
      if hint.is_some_and(|hint| hint > total_bytes) {
        break;
      }
    }
    Ok(lines)
  }

  fn __iter__(this: PyRef<'_, Self>) -> PyRef<'_, Self> {
    this
  }

  fn __next__(&mut self) -> PyResult<Option<Vec<u8>>> {
    let line = self.readline(None)?;
    if line.is_empty() {
      // If a "readline()" function in WSGI input stream Python class returns 0 bytes (not even "\n"), it means EOF.
      Ok(None)
    } else {
      Ok(Some(line))
    }
  }
}
