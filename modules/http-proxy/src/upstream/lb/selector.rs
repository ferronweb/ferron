//! Backend selection implementation for all load balancing algorithms.

use std::sync::Arc;

use crate::types::upstream::UpstreamInner;
use crate::types::ConnectionsTrackState;
use crate::upstream::lb::p2c_ewma::{self, EwmaStateMap, P2cEwmaParams};
use crate::upstream::lb::LoadBalancerAlgorithmInner;

/// Selects a backend index based on the load balancing algorithm.
///
/// For LeastConnections and TwoRandomChoices, also initializes the connection
/// tracker `Arc<()>` in the map if missing, so that the caller can simply
/// clone the existing entry without a second lock acquisition.
///
/// For ConsistentHash, `hash_key` must be provided.
pub fn select_backend_index(
    load_balancer_algorithm: &LoadBalancerAlgorithmInner,
    backends: &[(usize, UpstreamInner)],
    conn_state: Option<&ConnectionsTrackState>,
    ewma_state: Option<&EwmaStateMap>,
) -> usize {
    if backends.is_empty() {
        // Edge case: no backends...
        return 0;
    }

    match load_balancer_algorithm {
        LoadBalancerAlgorithmInner::Random => rand::random_range(0..backends.len()),
        LoadBalancerAlgorithmInner::RoundRobin(state) => {
            let weights: Vec<u32> = backends.iter().map(|(_, b)| b.weight).collect();
            state.next(&weights)
        }
        LoadBalancerAlgorithmInner::LeastConnections => {
            let Some(conn_state) = conn_state else {
                return 0;
            };
            let mut min_indexes = Vec::new();
            let mut min_connections = None;
            for (index, upstream) in backends.iter() {
                let connection_count = match conn_state.entry(upstream.clone()) {
                    dashmap::Entry::Occupied(e) => Arc::strong_count(e.get()) - 1,
                    dashmap::Entry::Vacant(e) => {
                        e.insert(Arc::new(()));
                        0
                    }
                };
                if upstream.weight == 0 {
                    // Zero-weight edge case
                    continue;
                }
                if let Some((prev_count, prev_weight)) = min_connections {
                    let current_score = (connection_count as u64) * (prev_weight as u64);
                    let prev_score = (prev_count as u64) * (upstream.weight as u64);

                    match current_score.cmp(&prev_score) {
                        std::cmp::Ordering::Less => {
                            min_indexes.clear();
                            min_indexes.push(*index);
                            min_connections = Some((connection_count, upstream.weight));
                        }
                        std::cmp::Ordering::Equal => {
                            min_indexes.push(*index);
                        }
                        _ => (),
                    }
                } else {
                    min_indexes.clear();
                    min_indexes.push(*index);
                    min_connections = Some((connection_count, upstream.weight));
                }
            }
            match min_indexes.len() {
                0 => 0,
                1 => min_indexes[0],
                _ => min_indexes[rand::random_range(0..min_indexes.len())],
            }
        }
        LoadBalancerAlgorithmInner::TwoRandomChoices => {
            let Some(conn_state) = conn_state else {
                return rand::random_range(0..backends.len());
            };
            if backends.len() < 2 {
                // Initialize tracker for single backend
                if let dashmap::Entry::Vacant(e) = conn_state.entry(backends[0].1.clone()) {
                    e.insert(Arc::new(()));
                }
                return 0;
            }
            let idx1 = rand::random_range(0..backends.len());
            let mut idx2 = rand::random_range(0..backends.len() - 1);
            if idx2 >= idx1 {
                idx2 += 1;
            }

            // Get count for first backend
            let (count1, _read_dropped) = {
                match conn_state.entry(backends[idx1].1.clone()) {
                    dashmap::Entry::Occupied(e) => (Arc::strong_count(e.get()) - 1, false),
                    dashmap::Entry::Vacant(e) => {
                        e.insert(Arc::new(()));
                        (0, true)
                    }
                }
            };

            // Get count for second backend
            let (count2, _) = {
                match conn_state.entry(backends[idx2].1.clone()) {
                    dashmap::Entry::Occupied(e) => (Arc::strong_count(e.get()) - 1, false),
                    dashmap::Entry::Vacant(e) => {
                        e.insert(Arc::new(()));
                        (0, true)
                    }
                }
            };

            if count2 >= count1 {
                idx1
            } else {
                idx2
            }
        }
        LoadBalancerAlgorithmInner::P2cEwma => {
            let params = P2cEwmaParams::default();

            // Without connection tracking, fall back to random.
            let Some(conn_state) = conn_state else {
                return rand::random_range(0..backends.len());
            };

            if backends.len() < 2 {
                // Initialise tracker for single backend
                if let dashmap::Entry::Vacant(e) = conn_state.entry(backends[0].1.clone()) {
                    e.insert(Arc::new(()));
                }
                return 0;
            }

            let idx1 = rand::random_range(0..backends.len());
            let mut idx2 = rand::random_range(0..backends.len() - 1);
            if idx2 >= idx1 {
                idx2 += 1;
            }

            // Get connection count and EWMA for each candidate
            let score_for = |idx: usize| -> f64 {
                let (_, ref upstream) = backends[idx];
                let active_conns = match conn_state.entry(upstream.clone()) {
                    dashmap::Entry::Occupied(e) => Arc::strong_count(e.get()) - 1,
                    dashmap::Entry::Vacant(e) => {
                        e.insert(Arc::new(()));
                        0
                    }
                };
                let ewma = ewma_state
                    .map(|s| p2c_ewma::get_decayed_ewma(s, upstream, &params))
                    .unwrap_or(params.default_ewma);
                p2c_ewma::compute_score(ewma, active_conns, &params)
            };

            let s1 = score_for(idx1);
            let s2 = score_for(idx2);

            if s2 < s1 {
                idx2
            } else {
                idx1
            }
        }
    }
}

/// Get or create the connection tracker for an upstream.
pub fn initialize_tracker(conn_state: Option<&ConnectionsTrackState>, upstream: &UpstreamInner) {
    if let Some(conn_state) = conn_state {
        if let dashmap::Entry::Vacant(e) = conn_state.entry(upstream.clone()) {
            e.insert(Arc::new(()));
        }
    }
}

/// Clone an existing connection tracker for an upstream.
pub fn get_tracker(
    conn_state: Option<&ConnectionsTrackState>,
    upstream: &UpstreamInner,
) -> Option<Arc<()>> {
    let conn_state = conn_state?;
    conn_state.get(upstream).as_deref().map(Arc::clone)
}
