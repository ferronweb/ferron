//! Global shutdown and reload coordination tokens.
//!
//! This module provides application-wide cancellation tokens that can be used
//! to signal shutdown or configuration reload events to async tasks.

use std::sync::LazyLock;

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
