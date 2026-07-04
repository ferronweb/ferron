//! Flapping detection types for circuit breaker and health check state transitions.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use crossbeam_queue::ArrayQueue;

/// Per-upstream flapping detection state.
///
/// Tracks recent state transition timestamps and whether the upstream
/// is currently considered "flapping" (oscillating too rapidly).
#[derive(Clone, Debug)]
pub struct FlappingState {
    /// Recent transition timestamps (ring buffer, protected by Mutex for atomic push+count).
    pub transitions: Option<Arc<ArrayQueue<Instant>>>,
    /// Whether the upstream is currently in a flapping state.
    pub is_flapping: Arc<AtomicBool>,
}

impl Default for FlappingState {
    fn default() -> Self {
        Self {
            transitions: Some(Arc::new(ArrayQueue::new(15))), // 16 - 1 = 15
            is_flapping: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl FlappingState {
    /// Create a new `FlappingState` with the given threshold.
    #[inline]
    pub fn with_threshold(threshold: u64) -> Self {
        Self {
            transitions: (threshold > 1)
                .then(|| Arc::new(ArrayQueue::new((threshold as usize).saturating_sub(1)))),
            is_flapping: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Set the threshold for this `FlappingState`.
    #[inline]
    pub fn set_threshold(&mut self, threshold: u64) {
        self.transitions = (threshold > 1)
            .then(|| Arc::new(ArrayQueue::new((threshold as usize).saturating_sub(1))));
        self.is_flapping = Arc::new(AtomicBool::new(false));
    }

    /// Get the threshold for this `FlappingState`, if applicable.
    #[inline]
    pub fn threshold(&self) -> Option<u64> {
        self.transitions.as_ref().map(|t| t.capacity() as u64 + 1)
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
    /// configured threshold, the upstream is marked as flapping.
    #[inline]
    pub fn record_transition(&self, window: std::time::Duration) -> bool {
        let flapping = self.transitions.as_ref().map_or(true, |transitions| {
            let now = Instant::now();
            let evicted = transitions.force_push(now);
            evicted.is_some_and(|t| now.duration_since(t) <= window)
        });

        self.set_flapping(flapping);
        flapping
    }
}

/// Shared map from upstream URL to flapping state.
pub type FlappingStateMap = Arc<dashmap::DashMap<String, FlappingState, rustc_hash::FxBuildHasher>>;
