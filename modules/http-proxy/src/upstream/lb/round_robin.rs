//! Weighted round-robin load balancing algorithm.
//!
//! Implements Nginx's smooth weighted round-robin algorithm for proportional
//! distribution across backends with different weights.

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
    pub current_weights: Vec<i64>,
}

impl WeightedRoundRobinState {
    /// Create a new empty state.
    pub fn new() -> Self {
        Self {
            current_weights: Vec::new(),
        }
    }

    /// Select the next backend index using smooth weighted round-robin.
    ///
    /// The `weights` slice provides the configured weight for each backend
    /// at the corresponding index. The internal `current_weights` vector
    /// is resized automatically if the backend count changes.
    ///
    /// Returns the index of the selected backend.
    pub fn next(&mut self, weights: &[u32]) -> usize {
        let n = weights.len();
        if n == 0 {
            return 0;
        }

        // Resize current_weights if backend count changed
        if self.current_weights.len() != n {
            self.current_weights.resize(n, 0);
        }

        // Calculate total weight
        let total_weight: i64 = weights.iter().map(|w| *w as i64).sum();

        let mut best_index = 0;
        let mut best_weight = i64::MIN;

        // Step 1: Add each backend's weight to its current effective weight
        // Step 2: Find the backend with the highest effective weight
        for (i, weight) in weights.iter().enumerate() {
            self.current_weights[i] += *weight as i64;
            if self.current_weights[i] > best_weight {
                best_weight = self.current_weights[i];
                best_index = i;
            }
        }

        // Step 3: Subtract total weight from the selected backend's effective weight
        self.current_weights[best_index] -= total_weight;

        best_index
    }
}
