//! Shared admin metrics types for the admin API.
//!
//! Provides atomic counters for tracking server metrics
//! across the data plane (HTTP server) and control plane (admin API).

use std::sync::atomic::AtomicU64;
use std::sync::LazyLock;
use std::time::{Instant, SystemTime};

/// Metrics for the reload process.
pub struct ReloadMetrics {
    pub last_reload_time: SystemTime,
    pub last_reload_error: Option<String>,
    pub active_generation: u64,
}

impl Default for ReloadMetrics {
    #[inline]
    fn default() -> Self {
        Self {
            last_reload_time: SystemTime::now(),
            last_reload_error: None,
            active_generation: 0,
        }
    }
}

/// Metrics for the runtime.
pub struct RuntimeMetrics {
    pub primary_threads: usize,
    pub io_uring_supported: bool,
    pub io_uring_runtime_enabled: bool,
}

impl Default for RuntimeMetrics {
    #[inline]
    fn default() -> Self {
        Self {
            primary_threads: 0,
            io_uring_supported: false,
            io_uring_runtime_enabled: false,
        }
    }
}

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
    /// Total number of observability events dropped by non-blocking sinks.
    pub observability_events_dropped: AtomicU64,
    /// Approximate current number of enqueued observability events across sinks.
    pub observability_event_queue_len: AtomicU64,
    /// Metrics related to configuration reloads.
    pub reload_metrics: parking_lot::RwLock<ReloadMetrics>,
    /// Metrics related to runtime.
    pub runtime_metrics: parking_lot::RwLock<RuntimeMetrics>,
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
            observability_events_dropped: AtomicU64::new(0),
            observability_event_queue_len: AtomicU64::new(0),
            reload_metrics: parking_lot::RwLock::new(ReloadMetrics::default()),
            runtime_metrics: parking_lot::RwLock::new(RuntimeMetrics::default()),
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
