use std::{
  pin::Pin,
  task::{Context, Poll},
};

use http_body_util::combinators::BoxBody;
use hyper::body::{Body, Bytes};
use tokio::io::{AsyncRead, ReadBuf};

pub struct CgiStdinReader {
  body: BoxBody<Bytes, hyper::Error>,
}

impl CgiStdinReader {
  pub fn new(body: BoxBody<Bytes, hyper::Error>) -> Self {
    CgiStdinReader { body }
  }
}

impl AsyncRead for CgiStdinReader {
  fn poll_read(
    mut self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    buf: &mut ReadBuf<'_>,
  ) -> Poll<std::io::Result<()>> {
    let body = Pin::new(&mut self.body);
    match body.poll_frame(cx) {
      Poll::Pending => Poll::Pending,
      Poll::Ready(None) => Poll::Ready(Ok(())),
      Poll::Ready(Some(Err(err))) => {
        Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::Other, err)))
      }
      Poll::Ready(Some(Ok(frame))) => match frame.into_data() {
        Ok(bytes) => {
          buf.put_slice(&bytes);
          Poll::Ready(Ok(()))
        }
        Err(_) => Poll::Pending,
      },
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use http_body_util::{BodyExt, Empty, Full};
  use tokio::io::{AsyncReadExt, ReadBuf};

  #[tokio::test]
  async fn test_cgi_stdin_reader_reads_data() {
    // Create a mock body with some data
    let body = Full::new("Hello, world!".into())
      .map_err(|e| match e {})
      .boxed();

    // Create an instance of CgiStdinReader
    let mut reader = CgiStdinReader::new(body);

    // Buffer to read data into
    let mut buf = vec![0; 128];
    let mut read_buf = ReadBuf::new(&mut buf);

    // Read data from the reader
    let n = reader.read_buf(&mut read_buf).await.unwrap();

    // Check that the data read is correct
    assert_eq!(&buf[..n], b"Hello, world!");
  }

  #[tokio::test]
  async fn test_cgi_stdin_reader_handles_empty_body() {
    // Create an empty body
    let body = Empty::new().map_err(|e| match e {}).boxed();

    // Create an instance of CgiStdinReader
    let mut reader = CgiStdinReader::new(body);

    // Buffer to read data into
    let mut buf = vec![0; 128];
    let mut read_buf = ReadBuf::new(&mut buf);

    // Read data from the reader
    let n = reader.read_buf(&mut read_buf).await.unwrap();

    // Check that no data is read
    assert_eq!(n, 0);
  }
}
