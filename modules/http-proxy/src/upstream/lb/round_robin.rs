//! Weighted round-robin load balancing algorithm.
//!
//! Implements Nginx's smooth weighted round-robin algorithm for proportional
//! distribution across backends with different weights.

use std::sync::Arc;

use parking_lot::RwLock;

#[cfg(target_has_atomic = "64")]
type AtomicEffectiveWeight = std::sync::atomic::AtomicI64;
#[cfg(not(target_has_atomic = "64"))]
type AtomicEffectiveWeight = std::sync::atomic::AtomicI32;

/// State for smooth weighted round-robin load balancing.
///
/// Uses Nginx's smooth weighted round-robin algorithm:
/// 1. Add each backend's weight to its current effective weight
/// 2. Select the backend with the highest effective weight
/// 3. Subtract total weight from the selected backend's effective weight
///
/// This ensures proportional distribution over time while avoiding bursts.
#[derive(Clone, Debug)]
pub struct WeightedRoundRobinState {
    /// Current effective weights for each backend position.
    /// Resized dynamically to match the active backend count.
    pub current_weights: Arc<RwLock<Vec<AtomicEffectiveWeight>>>,
}

impl WeightedRoundRobinState {
    /// Create a new empty state.
    pub fn new() -> Self {
        Self {
            current_weights: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Select the next backend index using smooth weighted round-robin.
    ///
    /// The `weights` slice provides the configured weight for each backend
    /// at the corresponding index. The internal `current_weights` vector
    /// is resized automatically if the backend count changes.
    ///
    /// Returns the index of the selected backend.
    pub fn next(&self, weights: &[u32]) -> usize {
        let n = weights.len();
        if n == 0 {
            return 0;
        }

        let mut current_weights = self.current_weights.read();

        // Resize current_weights if backend count changed
        if current_weights.len() != n {
            drop(current_weights);
            self.current_weights
                .write()
                .resize_with(n, || AtomicEffectiveWeight::new(0));
            current_weights = self.current_weights.read();
        }

        // Calculate total weight
        let total_weight: i64 = weights.iter().map(|w| *w as i64).sum();
        let mut best_index = 0;
        #[cfg(not(target_has_atomic = "64"))]
        let mut best_weight = i32::MIN;
        #[cfg(target_has_atomic = "64")]
        let mut best_weight = i64::MIN;

        // Step 1: Add each backend's weight to its current effective weight
        // Step 2: Find the backend with the highest effective weight
        for (i, weight) in weights.iter().enumerate() {
            let old_weight =
                current_weights[i].fetch_add(*weight as _, std::sync::atomic::Ordering::Relaxed);
            #[cfg(not(target_has_atomic = "64"))]
            let current_weight = old_weight + *weight as i32;
            #[cfg(target_has_atomic = "64")]
            let current_weight = old_weight as i64 + *weight as i64;
            if current_weight > best_weight {
                best_weight = current_weight;
                best_index = i;
            }
        }

        // Step 3: Subtract total weight from the selected backend's effective weight
        current_weights[best_index]
            .fetch_sub(total_weight as _, std::sync::atomic::Ordering::Relaxed);

        best_index
    }
}
