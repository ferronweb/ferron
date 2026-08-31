//! Global shutdown and reload coordination tokens.
//!
//! This module provides application-wide cancellation tokens that async
//! tasks use to detect shutdown or configuration reload events.
//!
//! # Usage
//!
//! ```ignore
//! use ferron_core::shutdown::SHUTDOWN_TOKEN;
//!
//! // In a background task
//! let token = SHUTDOWN_TOKEN.load_full();
//! tokio::select! {
//!     _ = token.cancelled() => { /* shutdown requested */ }
//!     _ = do_work() => { /* ... */ }
//! }
//! ```
//!
//! # How it works
//!
//! When the server receives a shutdown signal, it calls
//! [`CancellationToken::cancel`] on the current token and swaps in a new
//! one via [`ArcSwap::swap`]). Tasks that cloned the old token see it
//! cancel; new tasks get the fresh token.

use std::sync::{Arc, LazyLock};

use arc_swap::ArcSwap;
use tokio_util::sync::CancellationToken;

/// Global token for coordinating application shutdown.
///
/// Tasks clone this token and await [`CancellationToken::cancelled`] to be
/// notified when shutdown is requested. After cancellation, the server
/// swaps in a fresh token for the next startup cycle.
pub static SHUTDOWN_TOKEN: LazyLock<ArcSwap<CancellationToken>> =
    LazyLock::new(|| ArcSwap::from_pointee(CancellationToken::new()));

/// Global token for coordinating configuration reload events.
///
/// Tasks clone this token and await [`CancellationToken::cancelled`] to be
/// notified when a configuration reload is requested. The token is swapped
/// after each reload completes.
pub static RELOAD_TOKEN: LazyLock<ArcSwap<CancellationToken>> =
    LazyLock::new(|| ArcSwap::from_pointee(CancellationToken::new()));

/// Global state for coordinating configuration reload events.
///
/// Contains the current reload token and a [`ReloadState`] that tracks
/// whether a reload is in progress and what its outcome was.
pub static RELOAD_STATE: LazyLock<ArcSwap<(CancellationToken, ReloadState)>> =
    LazyLock::new(|| ArcSwap::from_pointee((CancellationToken::new(), ReloadState::default())));

/// State for a single configuration reload operation.
///
/// Tracks whether the reload has finished and whether it produced an error.
/// The [`get_state`](Self::get_state) method returns a future that resolves
/// when the reload completes.
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
    /// Mark the reload as finished, optionally recording an error.
    ///
    /// Cancels the internal token, which resolves any pending
    /// [`get_state`](Self::get_state) calls.
    pub fn set_state(&self, error: Option<String>) {
        *self.reload_error.write() = error;
        self.reload_finished_token.cancel();
    }

    /// Wait for the reload to finish and return its outcome.
    ///
    /// Returns `None` on success or `Some(error_message)` on failure.
    pub async fn get_state(&self) -> Option<String> {
        self.reload_finished_token.cancelled().await;
        self.reload_error.read().clone()
    }
}
