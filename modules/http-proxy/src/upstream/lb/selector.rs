//! Backend selection implementation for all load balancing algorithms.

use std::sync::Arc;
use std::time::Duration;

use crate::types::circuit::CircuitBreakerStateMap;
use crate::types::upstream::UpstreamInner;
use crate::types::ConnectionsTrackState;
use crate::upstream::lb::p2c_ewma::{self, EwmaStateMap, P2cEwmaParams};
use crate::upstream::lb::LoadBalancerAlgorithmInner;

/// Result of backend selection, including the selected index and
/// diagnostic candidate scores for P2C-based algorithms.
pub struct SelectionResult {
    /// The position within `healthy_indices` of the selected backend.
    pub index: usize,
    /// Candidate scores from the load-balancer comparison.
    ///
    /// For `TwoRandomChoices`: weighted connection counts (`count / weight`)
    /// for the two randomly chosen candidates. For `P2cEwma`: the combined
    /// EWMA + connection-penalty score for each candidate. For other
    /// algorithms, this is empty.
    pub candidate_scores: Vec<f64>,
}

/// Compute the slow-start virtual connection penalty for an upstream.
///
/// Returns `0` when slow-start is disabled or the recovery window has elapsed.
/// Otherwise returns a decaying penalty: full `weight * 10` at recovery time,
/// linearly decreasing to `0` over `slow_start_duration`.
#[inline]
fn slow_start_virtual_conns(
    circuit_breaker_state: Option<&CircuitBreakerStateMap>,
    slow_start_duration: Duration,
    upstream: &Arc<UpstreamInner>,
) -> usize {
    if slow_start_duration.is_zero() {
        return 0;
    }
    let Some(state_map) = circuit_breaker_state else {
        return 0;
    };
    let Some(state) = state_map.get(upstream) else {
        return 0;
    };
    let Some(ref recovery_at) = state.slow_start_recovery_at else {
        return 0;
    };
    let recovery_at = *recovery_at.read();
    let Some(recovery_at) = recovery_at else {
        return 0;
    };
    let elapsed = recovery_at.elapsed();
    if elapsed >= slow_start_duration {
        return 0;
    }
    let fraction = 1.0 - elapsed.as_secs_f64() / slow_start_duration.as_secs_f64();
    (upstream.weight as f64 * 10.0 * fraction) as usize
}

/// Selects a backend index based on the load balancing algorithm.
///
/// `healthy_indices` is a slice of indices into the `upstreams` slice,
/// built by filtering out unhealthy or already-selected backends.
/// Returns a [`SelectionResult`] with the selected position and, for
/// P2C-based algorithms, the candidate scores that were compared.
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
    circuit_breaker_state: Option<&CircuitBreakerStateMap>,
    slow_start_duration: Duration,
) -> SelectionResult {
    if healthy_indices.len() < 2 {
        // Fast path: no load balancing needed
        return SelectionResult {
            index: 0,
            candidate_scores: Vec::new(),
        };
    }

    match load_balancer_algorithm {
        LoadBalancerAlgorithmInner::Random => {
            let weights: Vec<u32> = healthy_indices
                .iter()
                .map(|i| upstreams[*i].weight)
                .collect();
            let total: u64 = weights.iter().map(|w| *w as u64).sum();
            let index = if total == 0 {
                // All weights are zero; fall back to uniform random
                rand::random_range(0..healthy_indices.len())
            } else {
                let threshold = rand::random_range(0..total);
                let mut cumulative: u64 = 0;
                let mut selected = 0;
                for (i, w) in weights.iter().enumerate() {
                    cumulative += *w as u64;
                    if threshold < cumulative {
                        selected = i;
                        break;
                    }
                }
                selected
            };
            SelectionResult {
                index,
                candidate_scores: Vec::new(),
            }
        }
        LoadBalancerAlgorithmInner::RoundRobin(state) => {
            let weights: Vec<u32> = healthy_indices
                .iter()
                .map(|i| upstreams[*i].weight)
                .collect();
            SelectionResult {
                index: state.next(&weights),
                candidate_scores: Vec::new(),
            }
        }
        LoadBalancerAlgorithmInner::LeastConnections => {
            let Some(conn_state) = conn_state else {
                return SelectionResult {
                    index: 0,
                    candidate_scores: Vec::new(),
                };
            };
            // Reservoir sampling among ties — avoids allocating a Vec for
            // equal-scoring minima.
            let mut best_pos = 0;
            let mut min_connections: Option<(usize, u32)> = None;
            let mut tie_count = 1;

            for (pos, idx) in healthy_indices.iter().enumerate() {
                let upstream = &upstreams[*idx];
                let connection_count = if let Some(e) = conn_state.get(upstream) {
                    Arc::strong_count(&*e) - 1
                } else {
                    match conn_state.entry(upstream.clone()) {
                        dashmap::Entry::Occupied(e) => Arc::strong_count(e.get()) - 1,
                        dashmap::Entry::Vacant(e) => {
                            e.insert(Arc::new(()));
                            0
                        }
                    }
                };
                let virtual_conns =
                    slow_start_virtual_conns(circuit_breaker_state, slow_start_duration, upstream);
                let effective_count = connection_count + virtual_conns;
                if upstream.weight == 0 {
                    continue;
                }
                if let Some((prev_count, prev_weight)) = min_connections {
                    let current_score = (effective_count as u64) * (prev_weight as u64);
                    let prev_score = (prev_count as u64) * (upstream.weight as u64);

                    match current_score.cmp(&prev_score) {
                        std::cmp::Ordering::Less => {
                            best_pos = pos;
                            min_connections = Some((effective_count, upstream.weight));
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
                    min_connections = Some((effective_count, upstream.weight));
                    tie_count = 1;
                }
            }
            SelectionResult {
                index: best_pos,
                candidate_scores: Vec::new(),
            }
        }
        LoadBalancerAlgorithmInner::TwoRandomChoices => {
            let Some(conn_state) = conn_state else {
                return SelectionResult {
                    index: rand::random_range(0..healthy_indices.len()),
                    candidate_scores: Vec::new(),
                };
            };
            if healthy_indices.len() < 2 {
                initialize_tracker(Some(conn_state), &upstreams[healthy_indices[0]]);
                return SelectionResult {
                    index: 0,
                    candidate_scores: Vec::new(),
                };
            }
            let idx1 = rand::random_range(0..healthy_indices.len());
            let mut idx2 = rand::random_range(0..healthy_indices.len() - 1);
            if idx2 >= idx1 {
                idx2 += 1;
            }

            let (count1, _) = {
                if let Some(e) = conn_state.get(&upstreams[healthy_indices[idx1]]) {
                    (Arc::strong_count(&*e) - 1, false)
                } else {
                    match conn_state.entry(upstreams[healthy_indices[idx1]].clone()) {
                        dashmap::Entry::Occupied(e) => (Arc::strong_count(e.get()) - 1, false),
                        dashmap::Entry::Vacant(e) => {
                            e.insert(Arc::new(()));
                            (0, true)
                        }
                    }
                }
            };

            let (count2, _) = {
                if let Some(e) = conn_state.get(&upstreams[healthy_indices[idx2]]) {
                    (Arc::strong_count(&*e) - 1, false)
                } else {
                    match conn_state.entry(upstreams[healthy_indices[idx2]].clone()) {
                        dashmap::Entry::Occupied(e) => (Arc::strong_count(e.get()) - 1, false),
                        dashmap::Entry::Vacant(e) => {
                            e.insert(Arc::new(()));
                            (0, true)
                        }
                    }
                }
            };

            let weight1 = upstreams[healthy_indices[idx1]].weight;
            let weight2 = upstreams[healthy_indices[idx2]].weight;

            let virtual1 = slow_start_virtual_conns(
                circuit_breaker_state,
                slow_start_duration,
                &upstreams[healthy_indices[idx1]],
            );
            let virtual2 = slow_start_virtual_conns(
                circuit_breaker_state,
                slow_start_duration,
                &upstreams[healthy_indices[idx2]],
            );

            let effective_count1 = count1 + virtual1;
            let effective_count2 = count2 + virtual2;

            // Compute weighted connection scores for diagnostics
            let score1 = if weight1 == 0 {
                f64::MAX
            } else {
                effective_count1 as f64 / weight1 as f64
            };
            let score2 = if weight2 == 0 {
                f64::MAX
            } else {
                effective_count2 as f64 / weight2 as f64
            };

            let prefer_idx1 = weight1 != 0
                && (weight2 == 0
                    || (effective_count1 as u64) * (weight2 as u64)
                        <= (effective_count2 as u64) * (weight1 as u64));

            let index = if prefer_idx1 { idx1 } else { idx2 };
            // Winner's score first, loser's second
            let candidate_scores = if prefer_idx1 {
                vec![score1, score2]
            } else {
                vec![score2, score1]
            };

            SelectionResult {
                index,
                candidate_scores,
            }
        }
        LoadBalancerAlgorithmInner::P2cEwma => {
            let params = P2cEwmaParams::default();

            let Some(conn_state) = conn_state else {
                return SelectionResult {
                    index: rand::random_range(0..healthy_indices.len()),
                    candidate_scores: Vec::new(),
                };
            };

            if healthy_indices.len() < 2 {
                initialize_tracker(Some(conn_state), &upstreams[healthy_indices[0]]);
                return SelectionResult {
                    index: 0,
                    candidate_scores: Vec::new(),
                };
            }

            let idx1 = rand::random_range(0..healthy_indices.len());
            let mut idx2 = rand::random_range(0..healthy_indices.len() - 1);
            if idx2 >= idx1 {
                idx2 += 1;
            }

            let score_for = |pos: usize| -> f64 {
                let upstream = &upstreams[healthy_indices[pos]];
                if upstream.weight == 0 {
                    return f64::MAX;
                }

                let active_conns = if let Some(e) = conn_state.get(upstream) {
                    Arc::strong_count(&*e) - 1
                } else {
                    match conn_state.entry(upstream.clone()) {
                        dashmap::Entry::Occupied(e) => Arc::strong_count(e.get()) - 1,
                        dashmap::Entry::Vacant(e) => {
                            e.insert(Arc::new(()));
                            0
                        }
                    }
                };
                let virtual_conns =
                    slow_start_virtual_conns(circuit_breaker_state, slow_start_duration, upstream);
                let ewma = ewma_state
                    .map(|s| p2c_ewma::get_decayed_ewma(s, upstream, &params))
                    .unwrap_or(params.default_ewma);
                p2c_ewma::compute_score(ewma, active_conns + virtual_conns, &params)
                    / upstream.weight as f64
            };

            let s1 = score_for(idx1);
            let s2 = score_for(idx2);

            let index = if s2 < s1 { idx2 } else { idx1 };
            // Winner's score first, loser's second
            let candidate_scores = if s2 < s1 { vec![s2, s1] } else { vec![s1, s2] };

            SelectionResult {
                index,
                candidate_scores,
            }
        }
    }
}

/// Get or create the connection tracker for an upstream.
#[inline]
pub fn initialize_tracker(
    conn_state: Option<&ConnectionsTrackState>,
    upstream: &Arc<UpstreamInner>,
) {
    if let Some(conn_state) = conn_state {
        if !conn_state.contains_key(upstream) {
            conn_state.insert(upstream.clone(), Arc::new(()));
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
