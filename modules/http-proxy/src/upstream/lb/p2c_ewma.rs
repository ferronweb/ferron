//! P2C (Power of Two Choices) + EWMA (Exponentially Weighted Moving Average)
//! adaptive load balancing state and functions.
//!
//! Each backend maintains an EWMA of its response latency. The selection
//! algorithm picks two random backends and compares their combined score:
//!
//! ```text
//! score = decayed_ewma + active_connections * connection_penalty
//! ```
//!
//! The backend with the lower score is selected. New backends (fewer than
//! `WARMUP_SAMPLES` samples) use a simple running average to build a baseline
//! before switching to exponential smoothing.

use std::sync::Arc;
use std::time::Instant;

use dashmap::DashMap;
use rustc_hash::FxBuildHasher;

use crate::types::upstream::UpstreamInner;

/// Number of initial samples taken with a simple running average before
/// switching to EWMA smoothing.
const WARMUP_SAMPLES: u64 = 10;

/// Per-backend EWMA state.
pub struct EwmaData {
    /// Current EWMA (or running average during warm-up) latency in seconds.
    pub ewma: f64,
    /// When this EWMA was last updated (used for time-based decay).
    pub last_update: Instant,
    /// Number of samples incorporated so far (< WARMUP_SAMPLES ⇒ warm-up).
    pub sample_count: u64,
}

/// Shared map from upstream to its EWMA data.
pub type EwmaStateMap = Arc<DashMap<Arc<UpstreamInner>, EwmaData, FxBuildHasher>>;

/// Tunable parameters for the P2C+EWMA algorithm.
///
/// Defaults are chosen for general-purpose HTTP reverse proxy workloads.
pub struct P2cEwmaParams {
    /// EWMA smoothing factor (α). Higher values weight the most recent
    /// observation more heavily. Default: 0.3
    pub alpha: f64,
    /// Half-life in seconds for time-based decay of unused backends.
    /// After this duration without updates, the EWMA decays by 50%.
    /// Default: 5.0
    pub decay_half_life_secs: f64,
    /// Penalty multiplier applied to active connection count in the
    /// combined score. Default: 0.5
    /// (meaning 2 active connections ≈ 1 second of latency)
    pub connection_penalty: f64,
    /// Default EWMA value (in seconds) assigned to backends with no
    /// recorded data yet. Default: 0.1
    pub default_ewma: f64,
}

impl Default for P2cEwmaParams {
    fn default() -> Self {
        Self {
            alpha: 0.3,
            decay_half_life_secs: 5.0,
            connection_penalty: 0.5,
            default_ewma: 0.1,
        }
    }
}

/// Update (or initialise) the EWMA for a backend with a new latency
/// observation.
///
/// During the warm-up period (first `WARMUP_SAMPLES` observations) the
/// value is a simple running average. After that, exponential smoothing
/// is used.
///
/// Non-finite latency values (NaN, Inf) are silently ignored to prevent
/// EWMA corruption and biased P2C selection.
pub fn update_ewma(
    state_map: &EwmaStateMap,
    upstream: &Arc<UpstreamInner>,
    latency_secs: f64,
    params: &P2cEwmaParams,
) {
    if !latency_secs.is_finite() || latency_secs < 0.0 {
        return;
    }
    state_map
        .entry(upstream.clone())
        .and_modify(|d| {
            if d.sample_count < WARMUP_SAMPLES {
                let n = d.sample_count as f64;
                let total = d.ewma * n;
                d.ewma = (total + latency_secs) / (n + 1.0);
            } else {
                d.ewma = params.alpha * latency_secs + (1.0 - params.alpha) * d.ewma;
            }
            d.sample_count += 1;
            d.last_update = Instant::now();
        })
        .or_insert_with(|| EwmaData {
            ewma: latency_secs,
            last_update: Instant::now(),
            sample_count: 1,
        });
}

/// Read the decayed EWMA value for a backend.
///
/// Returns `params.default_ewma` when no data exists for the backend.
/// Applies time-based exponential decay so that stale high-latency data
/// fades naturally.
pub fn get_decayed_ewma(
    state_map: &EwmaStateMap,
    upstream: &UpstreamInner,
    params: &P2cEwmaParams,
) -> f64 {
    state_map.get(upstream).map_or(params.default_ewma, |d| {
        let elapsed = d.last_update.elapsed().as_secs_f64();
        d.ewma * (-elapsed / params.decay_half_life_secs.max(0.001)).exp()
    })
}

/// Compute the combined P2C score from EWMA latency and active connections.
///
/// Lower score = more preferred.
pub fn compute_score(ewma: f64, active_connections: usize, params: &P2cEwmaParams) -> f64 {
    let score = ewma + (active_connections as f64) * params.connection_penalty;

    if !score.is_finite() || score < 0.0 {
        // The EWMA state is possibly corrupted; return a high score to avoid selecting it.
        return f64::MAX;
    }

    score
}

/// Returns `true` while the backend is still in the linear warm-up phase.
pub fn is_warming_up(state_map: &EwmaStateMap, upstream: &Arc<UpstreamInner>) -> bool {
    state_map
        .get(upstream)
        .is_none_or(|d| d.sample_count < WARMUP_SAMPLES)
}
