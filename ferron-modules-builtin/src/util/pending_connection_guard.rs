use std::sync::{
  atomic::{AtomicUsize, Ordering},
  Arc,
};

use arc_swap::ArcSwap;
use tokio_util::sync::CancellationToken;

pub struct PendingConnectionGuard {
  inner: Arc<AtomicUsize>,
  cancel: Arc<ArcSwap<CancellationToken>>,
}

impl PendingConnectionGuard {
  pub fn new(inner: Arc<AtomicUsize>, cancel: Arc<ArcSwap<CancellationToken>>) -> Self {
    inner.fetch_add(1, Ordering::Relaxed);
    Self { inner, cancel }
  }

  pub fn cancel_token(&self) -> Arc<ArcSwap<CancellationToken>> {
    self.cancel.clone()
  }
}

impl Drop for PendingConnectionGuard {
  fn drop(&mut self) {
    let prev_count = self.inner.fetch_sub(1, Ordering::Relaxed);
    if prev_count <= 1 {
      self.cancel.swap(Arc::new(CancellationToken::new())).cancel();
    }
  }
}
