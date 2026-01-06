use std::{
  pin::Pin,
  task::{ready, Context, Poll},
};

use bytes::Bytes;
use futures_util::Stream;
use tokio_util::sync::CancellationToken;

pub struct FcgiProcessedStream {
  stderr_cancel: CancellationToken,
  finished: bool,
  inner: Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send + Sync + Unpin>>,
}

impl FcgiProcessedStream {
  #[cfg(feature = "runtime-tokio")]
  pub fn new(
    inner: impl Stream<Item = Result<Bytes, std::io::Error>> + Send + Sync + Unpin + 'static,
    stderr_cancel: CancellationToken,
  ) -> Self {
    Self {
      stderr_cancel,
      finished: false,
      inner: Box::pin(inner),
    }
  }

  #[cfg(feature = "runtime-monoio")]
  pub fn new(
    inner: impl Stream<Item = Result<Bytes, std::io::Error>> + Unpin + 'static,
    stderr_cancel: CancellationToken,
  ) -> Self {
    Self {
      stderr_cancel,
      finished: false,
      inner: Box::pin(send_wrapper::SendWrapper::new(inner)),
    }
  }
}

impl Stream for FcgiProcessedStream {
  type Item = Result<Bytes, std::io::Error>;

  #[inline]
  fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
    if self.finished {
      return Poll::Ready(None);
    }
    let chunk = ready!(Pin::new(&mut self.inner).poll_next(cx));
    if chunk.is_none() {
      // No more chunks
      self.finished = true;
      return Poll::Ready(None);
    }
    if chunk.as_ref().is_some_and(|r| r.as_ref().is_ok_and(|c| c.is_empty())) {
      // Received empty STDOUT chunk, likely indicating end of output
      self.finished = true;
      return Poll::Ready(None);
    }
    Poll::Ready(chunk)
  }
}

impl Drop for FcgiProcessedStream {
  #[inline]
  fn drop(&mut self) {
    self.stderr_cancel.cancel();
  }
}
