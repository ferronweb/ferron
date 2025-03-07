use std::{
  future::Future,
  pin::Pin,
  task::{Context, Poll},
};

use futures_util::ready;
use tokio::io::{AsyncRead, AsyncWrite};

struct ZeroWriter<I> {
  inner: I,
}

impl<I> Future for ZeroWriter<I>
where
  I: AsyncWrite + Unpin,
{
  type Output = Result<(), tokio::io::Error>;

  fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
    let mut empty_slice = [0u8; 0];
    ready!(Pin::new(&mut self.inner).poll_write(cx, &mut empty_slice))?;
    ready!(Pin::new(&mut self.inner).poll_flush(cx))?;
    Poll::Ready(Ok(()))
  }
}

pub struct Copier<R, W> {
  reader: R,
  writer: W,
  zero_packet: bool,
}

impl<R, W> Copier<R, W>
where
  R: AsyncRead + Unpin,
  W: AsyncWrite + Unpin,
{
  pub fn new(reader: R, writer: W) -> Self {
    Self {
      reader,
      writer,
      zero_packet: false,
    }
  }

  pub fn with_zero_packet_writing(reader: R, writer: W) -> Self {
    Self {
      reader,
      writer,
      zero_packet: true,
    }
  }

  pub async fn copy(mut self) -> Result<u64, tokio::io::Error> {
    let copied_size = tokio::io::copy(&mut self.reader, &mut self.writer).await?;
    if self.zero_packet {
      let zero_writer = ZeroWriter { inner: self.writer };
      zero_writer.await?;
    }
    Ok(copied_size)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::pin::Pin;
  use tokio::io::{ReadBuf, Result};

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

    let copy = Copier::new(reader, writer).copy();
    let result = copy.await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), data.len() as u64);
  }

  #[tokio::test]
  async fn test_copy_empty() {
    let data = b"".to_vec();
    let reader = MockReader::new(data.clone());
    let writer = MockWriter::new();

    let copy = Copier::new(reader, writer).copy();
    let result = copy.await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 0);
  }
}
