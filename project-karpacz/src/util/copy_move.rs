use futures_util::ready;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf, Result};

pub struct Copy<R, W> {
  reader: R,
  writer: W,
  buffer: Vec<u8>,
}

impl<R, W> Copy<R, W>
where
  R: AsyncRead + Unpin,
  W: AsyncWrite + Unpin,
{
  pub fn new(reader: R, writer: W) -> Self {
    Self {
      reader,
      writer,
      buffer: vec![0; 1024], // You can adjust the buffer size as needed
    }
  }
}

impl<R, W> Future for Copy<R, W>
where
  R: AsyncRead + Unpin,
  W: AsyncWrite + Unpin,
{
  type Output = Result<usize>; // Number of bytes copied

  fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
    let this = self.get_mut();

    // Create a ReadBuf from the buffer
    let mut read_buf = ReadBuf::new(&mut this.buffer);

    // Read from the reader
    ready!(Pin::new(&mut this.reader).poll_read(cx, &mut read_buf))?;

    // Set the length of the ReadBuf
    let bytes_read = read_buf.filled().len();

    if bytes_read == 0 {
      // EOF reached
      return Poll::Ready(Ok(0));
    }

    // Write to the writer
    let bytes_written = ready!(Pin::new(&mut this.writer).poll_write(cx, read_buf.filled()))?;

    // Flush the writer to ensure all data is written
    ready!(Pin::new(&mut this.writer).poll_flush(cx))?;

    Poll::Ready(Ok(bytes_written))
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::pin::Pin;

  struct MockReader {
    data: Vec<u8>,
    position: usize,
  }

  impl MockReader {
    fn new(data: Vec<u8>) -> Self {
      Self { data, position: 0 }
    }
  }

  impl AsyncRead for MockReader {
    fn poll_read(
      self: Pin<&mut Self>,
      _cx: &mut Context<'_>,
      buf: &mut ReadBuf<'_>,
    ) -> Poll<Result<()>> {
      let this = self.get_mut();
      let remaining = this.data.len() - this.position;

      if remaining == 0 {
        return Poll::Ready(Ok(())); // EOF
      }

      let to_read = remaining.min(buf.remaining());
      buf.put_slice(&this.data[this.position..this.position + to_read]);
      this.position += to_read;

      Poll::Ready(Ok(()))
    }
  }

  struct MockWriter {
    data: Vec<u8>,
  }

  impl MockWriter {
    fn new() -> Self {
      Self { data: Vec::new() }
    }
  }

  impl AsyncWrite for MockWriter {
    fn poll_write(self: Pin<&mut Self>, _cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize>> {
      let this = self.get_mut();
      this.data.extend_from_slice(buf);
      Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<()>> {
      Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<()>> {
      Poll::Ready(Ok(()))
    }
  }

  #[tokio::test]
  async fn test_copy() {
    let data = b"Hello, world!".to_vec();
    let reader = MockReader::new(data.clone());
    let writer = MockWriter::new();

    let copy = Copy::new(reader, writer);
    let result = copy.await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), data.len());
  }

  #[tokio::test]
  async fn test_copy_empty() {
    let data = b"".to_vec();
    let reader = MockReader::new(data.clone());
    let writer = MockWriter::new();

    let copy = Copy::new(reader, writer);
    let result = copy.await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 0);
  }
}
