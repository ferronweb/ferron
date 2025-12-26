use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use arc_swap::ArcSwap;
use tokio_util::sync::CancellationToken;

/// An processing request guard
pub struct ProcessingRequestGuard {
  requests: Arc<AtomicUsize>,
  cancel: Arc<ArcSwap<CancellationToken>>,
}

impl ProcessingRequestGuard {
  /// Creates a new processing request guard
  pub fn new(requests: Arc<AtomicUsize>, cancel: Arc<ArcSwap<CancellationToken>>) -> Self {
    requests.fetch_add(1, Ordering::Relaxed);
    Self { requests, cancel }
  }

  /// Gets the cancellation token for the request
  #[inline]
  pub fn get_cancel_token(&self) -> Arc<CancellationToken> {
    self.cancel.load().clone()
  }
}

impl Drop for ProcessingRequestGuard {
  #[inline]
  fn drop(&mut self) {
    let prev_value = self.requests.fetch_sub(1, Ordering::Relaxed);
    if prev_value == 1 {
      self.cancel.swap(Arc::new(CancellationToken::new())).cancel();
    }
  }
}
