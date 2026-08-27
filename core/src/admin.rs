//! Shared admin metrics types for the admin API.
//!
//! Provides atomic counters for tracking server metrics
//! across the data plane (HTTP server) and control plane (admin API).

use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::LazyLock;
use std::time::{Instant, SystemTime};

/// Default mtime used before any configuration has been loaded.
const DEFAULT_MTIME: SystemTime = SystemTime::UNIX_EPOCH;

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
    /// Total HTTP cache journal records dropped because a zone's write queue
    /// overflowed (oldest prefix drop under backpressure).
    pub cache_persistence_dropped_records: AtomicU64,
    /// Total HTTP cache persistence errors (journal flush or snapshot
    /// compaction failures) across all zones.
    pub cache_persistence_errors: AtomicU64,
    /// Number of HTTP cache zones currently running memory-only after a
    /// journal flush failure disabled their persistence.
    pub cache_persistence_zones_inactive: AtomicU64,
    /// Metrics related to configuration reloads.
    pub reload_metrics: parking_lot::RwLock<ReloadMetrics>,
    /// Metrics related to runtime.
    pub runtime_metrics: parking_lot::RwLock<RuntimeMetrics>,
    /// Content hash of the loaded configuration (xxh3 hex).
    pub config_hash: parking_lot::RwLock<String>,
    /// Last modification time of the configuration source.
    pub config_mtime: parking_lot::RwLock<SystemTime>,
    /// Whether configuration drift is currently detected.
    pub config_drift: AtomicBool,
    /// Whether configuration drift hints are enabled.
    pub config_drift_hints_enabled: AtomicBool,
    /// Configuration metadata for drift detection (files + mtime from last load).
    pub config_drift_metadata: parking_lot::RwLock<Option<ConfigurationDriftMetadata>>,
}

/// Metadata used for configuration drift detection.
pub struct ConfigurationDriftMetadata {
    /// Files that were loaded in the last successful configuration load.
    pub config_files: Vec<std::path::PathBuf>,
    /// Last modification time recorded at the last successful load.
    pub config_mtime: SystemTime,
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
            cache_persistence_dropped_records: AtomicU64::new(0),
            cache_persistence_errors: AtomicU64::new(0),
            cache_persistence_zones_inactive: AtomicU64::new(0),
            reload_metrics: parking_lot::RwLock::new(ReloadMetrics::default()),
            runtime_metrics: parking_lot::RwLock::new(RuntimeMetrics::default()),
            config_hash: parking_lot::RwLock::new(String::new()),
            config_mtime: parking_lot::RwLock::new(DEFAULT_MTIME),
            config_drift: AtomicBool::new(false),
            config_drift_hints_enabled: AtomicBool::new(false),
            config_drift_metadata: parking_lot::RwLock::new(None),
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

/// Check whether configuration files have drifted from their last loaded state.
///
/// Re-stats all files in the metadata and compares their mtimes against the
/// recorded mtime. Returns `true` if any file has changed.
pub fn check_config_drift(metadata: &ConfigurationDriftMetadata) -> bool {
    let mut latest_mtime = std::time::UNIX_EPOCH;
    for file_path in &metadata.config_files {
        if let Ok(m) = std::fs::metadata(file_path) {
            if let Ok(mtime) = m.modified() {
                if mtime > latest_mtime {
                    latest_mtime = mtime;
                }
            }
        }
    }
    latest_mtime != metadata.config_mtime
}
