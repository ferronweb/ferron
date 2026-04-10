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
}

impl StatusResponse {
    /// Build from the global `ADMIN_METRICS`.
    pub fn from_global() -> Self {
        Self {
            uptime_sec: ADMIN_METRICS.start_time.elapsed().as_secs(),
            connections_active: ADMIN_METRICS.connections_active.load(Ordering::Relaxed),
            requests_total: ADMIN_METRICS.requests_total.load(Ordering::Relaxed),
            reloads: ADMIN_METRICS.reloads.load(Ordering::Relaxed),
        }
    }
}
