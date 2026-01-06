use smallvec::SmallVec;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{self, AsyncRead, ReadBuf};

/// Constant defining the stack capacity of the buffer
const BUFFER_STACK_CAPACITY: usize = 8192;

/// A future that reads an `AsyncRead` to the end
pub struct ReadToEndFuture<R> {
  reader: R,
  buffer: SmallVec<[u8; BUFFER_STACK_CAPACITY]>,
}

impl<R> ReadToEndFuture<R> {
  /// Create a new reading future
  pub fn new(reader: R) -> Self {
    Self {
      reader,
      buffer: SmallVec::new(),
    }
  }
}

impl<R> Future for ReadToEndFuture<R>
where
  R: AsyncRead + Unpin,
{
  type Output = io::Result<Vec<u8>>;

  #[inline]
  fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
    let mut buf = [0; 1024];
    let mut read_buf = ReadBuf::new(&mut buf);

    loop {
      match Pin::new(&mut self.reader).poll_read(cx, &mut read_buf) {
        Poll::Ready(Ok(())) => {
          let n = read_buf.filled().len();
          if n == 0 {
            return Poll::Ready(Ok(self.buffer.to_vec()));
          }
          self.buffer.extend_from_slice(read_buf.filled());
          read_buf.clear();
        }
        Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
        Poll::Pending => return Poll::Pending,
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::pin::Pin;
  use std::task::{Context, Poll};
  use tokio::io::{self, AsyncRead};

  struct MockReader {
    data: Vec<u8>,
    position: usize,
  }

  impl MockReader {
    fn new(data: &[u8]) -> Self {
      Self {
        data: data.to_vec(),
        position: 0,
      }
    }
  }

  impl AsyncRead for MockReader {
    fn poll_read(mut self: Pin<&mut Self>, _cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
      if self.position >= self.data.len() {
        return Poll::Ready(Ok(()));
      }

      let end = (self.position + buf.remaining()).min(self.data.len());
      buf.put_slice(&self.data[self.position..end]);
      self.position = end;

      Poll::Ready(Ok(()))
    }
  }

  #[tokio::test]
  async fn test_read_to_end_empty_reader() {
    let reader = MockReader::new(&[]);
    let future = ReadToEndFuture::new(reader);
    let result = future.await;
    assert_eq!(result.unwrap(), Vec::<u8>::new());
  }

  #[tokio::test]
  async fn test_read_to_end_non_empty_reader() {
    let reader = MockReader::new(b"hello world");
    let future = ReadToEndFuture::new(reader);
    let result = future.await;
    assert_eq!(result.unwrap(), b"hello world");
  }

  struct ErrorReader;

  impl AsyncRead for ErrorReader {
    fn poll_read(self: Pin<&mut Self>, _cx: &mut Context<'_>, _buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
      Poll::Ready(Err(io::Error::other("read error")))
    }
  }

  #[tokio::test]
  async fn test_read_to_end_error() {
    let reader = ErrorReader;
    let future = ReadToEndFuture::new(reader);
    let result = future.await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Other);
  }
}
