//! Batching pipelines: buffered, batched export of OTLP signals.
//!
//! Each pipeline owns a bounded buffer of finished items and a background
//! task that flushes the buffer on batch size or interval and drains it on
//! shutdown. Signals are wired in one pipeline per step (traces first, logs
//! second).

pub mod logs;
pub mod metrics;
pub mod traces;

use std::time::Duration;

/// Default number of finished items that trigger an export.
pub const DEFAULT_BATCH_SIZE: usize = 512;
/// Default upper bound on buffered finished items. New items are dropped
/// when the buffer is full (mirrors the SDK batch processor default queue).
pub const DEFAULT_QUEUE_CAPACITY: usize = 2048;
/// Default interval at which a partially full buffer is flushed.
pub const DEFAULT_FLUSH_INTERVAL: Duration = Duration::from_secs(5);
/// Default upper bound on one export round (including transport retries).
pub const DEFAULT_EXPORT_TIMEOUT: Duration = Duration::from_secs(30);
/// Default interval at which the metric reader collects and exports all
/// series (parity with the SDK `PeriodicReader`).
pub const DEFAULT_READ_INTERVAL: Duration = Duration::from_secs(30);

/// Batching parameters for the exporters.
#[derive(Debug, Clone, Copy)]
pub struct BatchConfig {
    /// Number of finished items that trigger a flush.
    pub batch_size: usize,
    /// Upper bound on buffered finished items.
    pub queue_capacity: usize,
    /// Interval at which a partially full buffer is flushed.
    pub interval: Duration,
    /// Upper bound on one export round (including transport retries).
    pub export_timeout: Duration,
}

impl Default for BatchConfig {
    #[inline]
    fn default() -> Self {
        Self {
            batch_size: DEFAULT_BATCH_SIZE,
            queue_capacity: DEFAULT_QUEUE_CAPACITY,
            interval: DEFAULT_FLUSH_INTERVAL,
            export_timeout: DEFAULT_EXPORT_TIMEOUT,
        }
    }
}
