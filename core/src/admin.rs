//! Shared admin metrics types for the admin API.
//!
//! Provides atomic counters for tracking server metrics
//! across the data plane (HTTP server) and control plane (admin API).

use std::sync::atomic::AtomicU64;
use std::sync::LazyLock;
use std::time::Instant;

/// Global metrics store for admin API endpoints.
///
/// Counters are updated from the data plane (HTTP server TCP listener and handler)
/// and read by the control plane (admin API axum handlers).
pub struct AdminMetrics {
    /// Server start time, used to compute uptime.
    pub start_time: Instant,
    /// Currently active TCP connections (incremented on accept, decremented on close).
    pub connections_active: AtomicU64,
    /// Total HTTP requests served across all HTTP servers.
    pub requests_total: AtomicU64,
    /// Total configuration reloads performed.
    pub reloads: AtomicU64,
}

impl AdminMetrics {
    /// Create a new metrics instance with the current time as start.
    #[inline]
    pub fn new() -> Self {
        Self {
            start_time: Instant::now(),
            connections_active: AtomicU64::new(0),
            requests_total: AtomicU64::new(0),
            reloads: AtomicU64::new(0),
        }
    }
}

impl Default for AdminMetrics {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

/// Global singleton for admin metrics.
pub static ADMIN_METRICS: LazyLock<AdminMetrics> = LazyLock::new(AdminMetrics::new);
