use std::{
  future::Future,
  pin::Pin,
  task::{Context, Poll},
};

use tokio::sync::{Semaphore, SemaphorePermit};
use tokio_util::sync::{CancellationToken, WaitForCancellationFuture};

/// A struct that can be canceled multiple times until specified times.
pub struct MultiCancel {
  semaphore: Semaphore,
  cancel_token: CancellationToken,
}

impl MultiCancel {
  /// Creates a new `MultiCancel` instance with the specified maximum number of cancels.
  pub fn new(max_cancels: usize) -> Self {
    Self {
      semaphore: Semaphore::new(max_cancels),
      cancel_token: CancellationToken::new(),
    }
  }

  /// Returns a future that resolves when the cancel token is canceled.
  pub fn cancel<'a>(&'a self) -> MultiCancelFuture<'a> {
    let permit = self.semaphore.try_acquire().ok();
    if permit.is_none() {
      self.cancel_token.cancel();
    }
    MultiCancelFuture {
      permit,
      cancel_token: Box::pin(self.cancel_token.cancelled()),
    }
  }
}

/// A future created from `MultiCancel` that resolves when the cancel token is canceled.
pub struct MultiCancelFuture<'a> {
  permit: Option<SemaphorePermit<'a>>,
  cancel_token: Pin<Box<WaitForCancellationFuture<'a>>>,
}

impl<'a> Future for MultiCancelFuture<'a> {
  type Output = ();

  fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
    Pin::new(&mut self.cancel_token).poll(cx)
  }
}

impl Drop for MultiCancelFuture<'_> {
  fn drop(&mut self) {
    if let Some(permit) = self.permit.take() {
      permit.forget();
    }
  }
}
