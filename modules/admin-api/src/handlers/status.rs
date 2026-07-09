use std::sync::atomic::Ordering;

use ferron_core::admin::ADMIN_METRICS;

/// Response payload for the `/status` endpoint.
pub struct StatusResponse {
    /// Seconds since server start.
    pub uptime_sec: u64,
    /// Currently active TCP connections.
    pub connections_active: u64,
    /// Total HTTP requests served.
    pub requests_total: u64,
    /// Total configuration reloads.
    pub reloads: u64,
    /// Total observability events dropped by observability sinks.
    pub observability_events_dropped: u64,
    /// Approximate current enqueued observability events.
    pub observability_event_queue_len: u64,
    /// Content hash of the loaded configuration (xxh3 hex).
    pub config_file_hash: String,
    /// Last modification time of the configuration source (epoch seconds).
    pub config_file_mtime: u64,
}

impl StatusResponse {
    /// Build from the global `ADMIN_METRICS`.
    pub fn from_global() -> Self {
        let config_mtime = ADMIN_METRICS.config_mtime.read();
        let mtime_epoch = config_mtime
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let config_hash = ADMIN_METRICS.config_hash.read().clone();

        Self {
            uptime_sec: ADMIN_METRICS.start_time.elapsed().as_secs(),
            connections_active: ADMIN_METRICS.connections_active.load(Ordering::Relaxed),
            requests_total: ADMIN_METRICS.requests_total.load(Ordering::Relaxed),
            reloads: ADMIN_METRICS.reloads.load(Ordering::Relaxed),
            observability_events_dropped: ADMIN_METRICS
                .observability_events_dropped
                .load(Ordering::Relaxed),
            observability_event_queue_len: ADMIN_METRICS
                .observability_event_queue_len
                .load(Ordering::Relaxed),
            config_file_hash: config_hash,
            config_file_mtime: mtime_epoch,
        }
    }
}
