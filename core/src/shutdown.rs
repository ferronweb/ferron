//! Global shutdown and reload coordination tokens.
//!
//! This module provides application-wide cancellation tokens that can be used
//! to signal shutdown or configuration reload events to async tasks.

use std::sync::{Arc, LazyLock};

use arc_swap::ArcSwap;
use tokio_util::sync::CancellationToken;

/// Global token for coordinating application shutdown.
///
/// Tasks can clone this token and wait on it to be notified when shutdown is requested.
pub static SHUTDOWN_TOKEN: LazyLock<ArcSwap<CancellationToken>> =
    LazyLock::new(|| ArcSwap::from_pointee(CancellationToken::new()));

/// Global token for coordinating configuration reload events.
///
/// Tasks can clone this token and wait on it to be notified when a reload is requested.
pub static RELOAD_TOKEN: LazyLock<ArcSwap<CancellationToken>> =
    LazyLock::new(|| ArcSwap::from_pointee(CancellationToken::new()));

/// Global state for coordinating configuration reload events.
pub static RELOAD_STATE: LazyLock<ArcSwap<(CancellationToken, ReloadState)>> =
    LazyLock::new(|| ArcSwap::from_pointee((CancellationToken::new(), ReloadState::default())));

/// State for a single configuration reload operation.
#[derive(Clone)]
pub struct ReloadState {
    reload_finished_token: CancellationToken,
    reload_error: Arc<parking_lot::RwLock<Option<String>>>,
}

impl Default for ReloadState {
    fn default() -> Self {
        Self {
            reload_finished_token: CancellationToken::new(),
            reload_error: Arc::new(parking_lot::RwLock::new(None)),
        }
    }
}

impl ReloadState {
    /// Sets the reload state, including any error and cancelling the finished token.
    pub fn set_state(&self, error: Option<String>) {
        *self.reload_error.write() = error;
        self.reload_finished_token.cancel();
    }

    /// Returns the reload state as a future that resolves when the finished token is cancelled.
    pub async fn get_state(&self) -> Option<String> {
        self.reload_finished_token.cancelled().await;
        self.reload_error.read().clone()
    }
}
