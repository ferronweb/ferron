//! Backend selection implementation for all load balancing algorithms.

use std::sync::Arc;

use crate::types::upstream::UpstreamInner;
use crate::types::ConnectionsTrackState;
use crate::upstream::lb::p2c_ewma::{self, EwmaStateMap, P2cEwmaParams};
use crate::upstream::lb::LoadBalancerAlgorithmInner;

/// Selects a backend index based on the load balancing algorithm.
///
/// `healthy_indices` is a slice of indices into the `upstreams` slice,
/// built by filtering out unhealthy or already-selected backends.
/// Returns the position within `healthy_indices` of the selected backend.
///
/// For LeastConnections and TwoRandomChoices, also initializes the connection
/// tracker `Arc<()>` in the map if missing, so that the caller can simply
/// clone the existing entry without a second lock acquisition.
#[inline]
pub fn select_backend_index(
    load_balancer_algorithm: &LoadBalancerAlgorithmInner,
    healthy_indices: &[usize],
    upstreams: &[Arc<UpstreamInner>],
    conn_state: Option<&ConnectionsTrackState>,
    ewma_state: Option<&EwmaStateMap>,
) -> usize {
    if healthy_indices.len() < 2 {
        // Fast path: no load balancing needed
        return 0;
    }

    match load_balancer_algorithm {
        LoadBalancerAlgorithmInner::Random => rand::random_range(0..healthy_indices.len()),
        LoadBalancerAlgorithmInner::RoundRobin(state) => {
            let weights: Vec<u32> = healthy_indices
                .iter()
                .map(|i| upstreams[*i].weight)
                .collect();
            state.next(&weights)
        }
        LoadBalancerAlgorithmInner::LeastConnections => {
            let Some(conn_state) = conn_state else {
                return 0;
            };
            // Reservoir sampling among ties — avoids allocating a Vec for
            // equal-scoring minima.
            let mut best_pos = 0;
            let mut min_connections: Option<(usize, u32)> = None;
            let mut tie_count = 1;

            for (pos, idx) in healthy_indices.iter().enumerate() {
                let upstream = &upstreams[*idx];
                let connection_count = match conn_state.entry(upstream.clone()) {
                    dashmap::Entry::Occupied(e) => Arc::strong_count(e.get()) - 1,
                    dashmap::Entry::Vacant(e) => {
                        e.insert(Arc::new(()));
                        0
                    }
                };
                if upstream.weight == 0 {
                    continue;
                }
                if let Some((prev_count, prev_weight)) = min_connections {
                    let current_score = (connection_count as u64) * (prev_weight as u64);
                    let prev_score = (prev_count as u64) * (upstream.weight as u64);

                    match current_score.cmp(&prev_score) {
                        std::cmp::Ordering::Less => {
                            best_pos = pos;
                            min_connections = Some((connection_count, upstream.weight));
                            tie_count = 1;
                        }
                        std::cmp::Ordering::Equal => {
                            tie_count += 1;
                            // Reservoir sampling: each equal-scoring backend
                            // has 1/n chance of replacing the current pick.
                            if rand::random_range(0..tie_count) == 0 {
                                best_pos = pos;
                            }
                        }
                        _ => (),
                    }
                } else {
                    best_pos = pos;
                    min_connections = Some((connection_count, upstream.weight));
                    tie_count = 1;
                }
            }
            best_pos
        }
        LoadBalancerAlgorithmInner::TwoRandomChoices => {
            let Some(conn_state) = conn_state else {
                return rand::random_range(0..healthy_indices.len());
            };
            if healthy_indices.len() < 2 {
                if let dashmap::Entry::Vacant(e) =
                    conn_state.entry(upstreams[healthy_indices[0]].clone())
                {
                    e.insert(Arc::new(()));
                }
                return 0;
            }
            let idx1 = rand::random_range(0..healthy_indices.len());
            let mut idx2 = rand::random_range(0..healthy_indices.len() - 1);
            if idx2 >= idx1 {
                idx2 += 1;
            }

            let (count1, _) = {
                match conn_state.entry(upstreams[healthy_indices[idx1]].clone()) {
                    dashmap::Entry::Occupied(e) => (Arc::strong_count(e.get()) - 1, false),
                    dashmap::Entry::Vacant(e) => {
                        e.insert(Arc::new(()));
                        (0, true)
                    }
                }
            };

            let (count2, _) = {
                match conn_state.entry(upstreams[healthy_indices[idx2]].clone()) {
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

            let Some(conn_state) = conn_state else {
                return rand::random_range(0..healthy_indices.len());
            };

            if healthy_indices.len() < 2 {
                if let dashmap::Entry::Vacant(e) =
                    conn_state.entry(upstreams[healthy_indices[0]].clone())
                {
                    e.insert(Arc::new(()));
                }
                return 0;
            }

            let idx1 = rand::random_range(0..healthy_indices.len());
            let mut idx2 = rand::random_range(0..healthy_indices.len() - 1);
            if idx2 >= idx1 {
                idx2 += 1;
            }

            let score_for = |pos: usize| -> f64 {
                let upstream = &upstreams[healthy_indices[pos]];
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
pub fn initialize_tracker(
    conn_state: Option<&ConnectionsTrackState>,
    upstream: &Arc<UpstreamInner>,
) {
    if let Some(conn_state) = conn_state {
        if let dashmap::Entry::Vacant(e) = conn_state.entry(upstream.clone()) {
            e.insert(Arc::new(()));
        }
    }
}

/// Clone an existing connection tracker for an upstream.
pub fn get_tracker(
    conn_state: Option<&ConnectionsTrackState>,
    upstream: &Arc<UpstreamInner>,
) -> Option<Arc<()>> {
    let conn_state = conn_state?;
    conn_state.get(upstream).as_deref().map(Arc::clone)
}
