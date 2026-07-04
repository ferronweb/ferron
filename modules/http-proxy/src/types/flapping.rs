//! Flapping detection types for circuit breaker and health check state transitions.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// Per-upstream flapping detection state.
///
/// Tracks recent state transition timestamps and whether the upstream
/// is currently considered "flapping" (oscillating too rapidly).
#[derive(Clone, Debug)]
pub struct FlappingState {
    /// Recent transition timestamps (ring buffer, protected by Mutex for atomic push+count).
    pub transitions: Arc<parking_lot::Mutex<VecDeque<Instant>>>,
    /// Whether the upstream is currently in a flapping state.
    pub is_flapping: Arc<AtomicBool>,
}

impl Default for FlappingState {
    fn default() -> Self {
        Self {
            transitions: Arc::new(parking_lot::Mutex::new(VecDeque::with_capacity(16))),
            is_flapping: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl FlappingState {
    /// Create a new `FlappingState` with a ring buffer sized for the given threshold.
    pub fn with_capacity(threshold: u64) -> Self {
        Self {
            transitions: Arc::new(parking_lot::Mutex::new(VecDeque::with_capacity(
                (threshold as usize).saturating_mul(2).max(4),
            ))),
            is_flapping: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Check whether the upstream is currently flapping.
    #[inline]
    pub fn is_flapping(&self) -> bool {
        self.is_flapping.load(Ordering::Relaxed)
    }

    /// Set the flapping state.
    #[inline]
    pub fn set_flapping(&self, flapping: bool) {
        self.is_flapping.store(flapping, Ordering::Relaxed);
    }

    /// Record a state transition and return whether the upstream is now flapping.
    ///
    /// A transition is recorded as the current instant. Timestamps older than
    /// `window` are evicted. If the count of recent transitions exceeds
    /// `threshold`, the upstream is marked as flapping.
    pub fn record_transition(&self, window: std::time::Duration, threshold: u64) -> bool {
        let now = Instant::now();

        let count = {
            let mut transitions = self.transitions.lock();
            transitions.push_back(now);
            // Evict stale entries
            while transitions
                .front()
                .is_some_and(|t| now.duration_since(*t) > window)
            {
                transitions.pop_front();
            }
            transitions.len() as u64
        };

        let flapping = count >= threshold;
        self.set_flapping(flapping);
        flapping
    }
}

/// Shared map from upstream URL to flapping state.
pub type FlappingStateMap = Arc<dashmap::DashMap<String, FlappingState, rustc_hash::FxBuildHasher>>;
