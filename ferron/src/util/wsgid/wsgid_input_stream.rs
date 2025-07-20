use std::io::BufRead;

use pyo3::prelude::*;

/// Creates a new WSGI error stream
#[pyclass]
pub struct WsgidInputStream {
  body_reader: Box<dyn BufRead + Send + Sync>,
}

impl WsgidInputStream {
  /// Creates a new WSGI input stream from a `BufRead`
  pub fn new(body_reader: impl BufRead + Send + Sync + 'static) -> Self {
    Self {
      body_reader: Box::new(body_reader),
    }
  }
}

#[pymethods]
impl WsgidInputStream {
  fn read(&mut self, size: usize) -> PyResult<Vec<u8>> {
    let mut buffer = vec![0u8; size];
    let read_bytes = self.body_reader.read(&mut buffer)?;
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
      let reader_buffer = self.body_reader.fill_buf()?.to_vec();
      if reader_buffer.is_empty() {
        break;
      }
      if let Some(eol_position) = reader_buffer.iter().position(|&char| char == b'\n') {
        buffer.extend_from_slice(
          &reader_buffer[0..size.map_or(eol_position + 1, |size| std::cmp::min(size, eol_position + 1))],
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
    let hint = if hint.is_none_or(|s| s < 0) {
      None
    } else {
      hint.map(|s| s as usize)
    };
    loop {
      let mut line = Vec::new();
      let bytes_read = self.body_reader.read_until(b'\n', &mut line)?;
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

#[cfg(test)]
mod tests {
  use super::*;
  use std::io::{BufReader, Cursor};

  fn create_stream(data: &str) -> WsgidInputStream {
    let cursor = Cursor::new(data.as_bytes().to_vec());
    let reader = BufReader::new(cursor);
    WsgidInputStream::new(reader)
  }

  #[test]
  fn test_read() {
    let mut stream = create_stream("Hello, world!");
    let result = stream.read(5).unwrap();
    assert_eq!(result, b"Hello");
  }

  #[test]
  fn test_read_full() {
    let mut stream = create_stream("Hello");
    let result = stream.read(10).unwrap(); // try to read more than available
    assert_eq!(result, b"Hello");
  }

  #[test]
  fn test_readline_no_limit() {
    let mut stream = create_stream("line1\nline2\n");
    let result = stream.readline(None).unwrap();
    assert_eq!(result, b"line1\n");

    let result = stream.readline(None).unwrap();
    assert_eq!(result, b"line2\n");
  }

  #[test]
  fn test_readline_with_limit() {
    let mut stream = create_stream("line1\nline2\n");
    let result = stream.readline(Some(3)).unwrap();
    assert_eq!(result, b"lin"); // Only 3 bytes
  }

  #[test]
  fn test_readlines_no_hint() {
    let mut stream = create_stream("line1\nline2\nline3\n");
    let result = stream.readlines(None).unwrap();
    assert_eq!(result, vec![b"line1\n", b"line2\n", b"line3\n"]);
  }

  #[test]
  fn test_readlines_with_hint() {
    let mut stream = create_stream("line1\nline2\nline3\n");
    let result = stream.readlines(Some(10)).unwrap(); // Should stop when bytes exceed 10
    let total: usize = result.iter().map(|l| l.len()).sum();
    assert!(total > 0 && total <= 10);
  }

  #[test]
  fn test_iterator_behavior() {
    let mut stream = create_stream("line1\nline2\n");

    let mut results = Vec::new();
    while let Some(line) = stream.__next__().unwrap() {
      results.push(line);
    }

    assert_eq!(results, vec![b"line1\n", b"line2\n"]);
  }

  #[test]
  fn test_iterator_eof() {
    let mut stream = create_stream("");
    let result = stream.__next__().unwrap();
    assert_eq!(result, None);
  }
}
