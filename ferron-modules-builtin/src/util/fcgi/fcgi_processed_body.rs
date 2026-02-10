use std::{
  pin::Pin,
  task::{ready, Context, Poll},
};

use bytes::Buf;
use hyper::body::Body;
use tokio_util::sync::CancellationToken;

pub struct FcgiProcessedBody<B> {
  stderr_cancel: CancellationToken,
  finished: bool,
  #[cfg(feature = "runtime-tokio")]
  inner: Pin<Box<B>>,
  #[cfg(feature = "runtime-monoio")]
  inner: send_wrapper::SendWrapper<Pin<Box<B>>>,
}

impl<B> FcgiProcessedBody<B>
where
  B: Body,
{
  #[cfg(feature = "runtime-tokio")]
  pub fn new(inner: B, stderr_cancel: CancellationToken) -> Self {
    Self {
      stderr_cancel,
      finished: false,
      inner: Box::pin(inner),
    }
  }

  #[cfg(feature = "runtime-monoio")]
  pub fn new(inner: B, stderr_cancel: CancellationToken) -> Self {
    Self {
      stderr_cancel,
      finished: false,
      inner: send_wrapper::SendWrapper::new(Box::pin(inner)),
    }
  }
}

impl<B> Body for FcgiProcessedBody<B>
where
  B: Body,
{
  type Data = B::Data;
  type Error = B::Error;

  #[inline]
  fn poll_frame(
    mut self: Pin<&mut Self>,
    cx: &mut Context<'_>,
  ) -> Poll<Option<Result<hyper::body::Frame<Self::Data>, Self::Error>>> {
    if self.finished {
      return Poll::Ready(None);
    }
    #[cfg(feature = "runtime-tokio")]
    let chunk = ready!(Pin::new(&mut self.inner).poll_frame(cx));
    #[cfg(feature = "runtime-monoio")]
    let chunk = ready!(Pin::new(&mut *self.inner).poll_frame(cx));
    if chunk.is_none() {
      // No more chunks
      self.finished = true;
      return Poll::Ready(None);
    }
    if chunk.as_ref().is_some_and(|r| {
      r.as_ref()
        .is_ok_and(|c| c.data_ref().is_some_and(|d| !d.has_remaining()))
    }) {
      // Received empty STDOUT chunk, likely indicating end of output
      self.finished = true;
      return Poll::Ready(None);
    }
    Poll::Ready(chunk)
  }
}

impl<B> Drop for FcgiProcessedBody<B> {
  #[inline]
  fn drop(&mut self) {
    self.stderr_cancel.cancel();
  }
}
