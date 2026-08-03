//! Upstream resolution and backend selection logic.

use std::collections::BTreeMap;
use std::sync::Arc;

use rustc_hash::FxHashSet;

use crate::config::AffinityType;
use crate::types::health::HealthCheckStateMap;
use crate::types::upstream::{Upstream, UpstreamInner};
use crate::types::ConnectionsTrackState;
use crate::upstream::circuit::CircuitBreaker;
use crate::upstream::lb::{ConsistentHashRing, EwmaStateMap, LoadBalancerAlgorithmInner};

/// Resolve all upstreams to a flat list of `Arc<UpstreamInner>` entries.
///
/// For SRV upstreams, this performs DNS resolution. For static upstreams,
/// it returns them as-is.
#[inline]
pub async fn resolve_upstreams(upstreams: &[Upstream]) -> Vec<Arc<UpstreamInner>> {
    // Capacity of at least the number of upstreams to avoid reallocations in many cases.
    let mut resolved = Vec::with_capacity(upstreams.len());
    for upstream in upstreams {
        resolved.extend(upstream.resolve().await);
    }
    resolved
}

/// Backends skipped during a selection round and the reason each was skipped.
#[derive(Default)]
pub struct SelectionExclusions {
    /// Backends already tried by this request's retry loop.
    pub already_tried: Vec<Arc<UpstreamInner>>,
    /// Backends skipped because their circuit breaker is open.
    pub circuit_open: Vec<Arc<UpstreamInner>>,
    /// Backends skipped because they are overloaded (half-open slot busy).
    pub overloaded: Vec<Arc<UpstreamInner>>,
}

/// The result of one backend selection round: the selected backend, its
/// connection tracker (if any), the candidate scores from the load-balancer
/// algorithm, and the backends skipped along the way.
pub struct SelectionOutcome {
    /// The selected upstream.
    pub upstream: Arc<UpstreamInner>,
    /// Connection tracker for LeastConnections/TwoRandomChoices.
    /// `None` for Random/RoundRobin algorithms.
    pub tracker: Option<Arc<()>>,
    /// Candidate scores from the load-balancer selection algorithm.
    ///
    /// For P2C-based algorithms (`TwoRandomChoices`, `P2cEwma`), contains
    /// the two candidate scores that were compared. For other algorithms,
    /// this is empty.
    pub candidate_scores: Vec<f64>,
    /// Backends skipped during this selection round.
    pub exclusions: SelectionExclusions,
}

/// A borrow-aggregator over the backend selection state for one request.
///
/// Holds every input the selection needs (upstreams, load-balancer state,
/// health/circuit state, affinity) plus the set of backends already tried
/// by this request's retry loop. Backends are grouped by priority (lower
/// value = higher priority); the highest-priority tier is tried first and
/// the next tier is used as a fallback once a tier is exhausted.
pub struct BackendSet<'a> {
    upstreams: &'a [Arc<UpstreamInner>],
    algorithm: &'a LoadBalancerAlgorithmInner,
    conn_state: Option<&'a ConnectionsTrackState>,
    ewma_state: Option<&'a EwmaStateMap>,
    health_check_state: Option<&'a HealthCheckStateMap>,
    circuit_breaker: CircuitBreaker<'a>,
    affinity_type: Option<&'a AffinityType>,
    affinity_key: Option<&'a [u8]>,
    ring: &'a parking_lot::RwLock<ConsistentHashRing>,
    tried: FxHashSet<Arc<UpstreamInner>>,
    exclusions: SelectionExclusions,
}

impl<'a> BackendSet<'a> {
    /// Create a backend set over the given selection state.
    #[allow(clippy::too_many_arguments)]
    #[inline]
    pub fn new(
        upstreams: &'a [Arc<UpstreamInner>],
        algorithm: &'a LoadBalancerAlgorithmInner,
        conn_state: Option<&'a ConnectionsTrackState>,
        ewma_state: Option<&'a EwmaStateMap>,
        health_check_state: Option<&'a HealthCheckStateMap>,
        circuit_breaker: CircuitBreaker<'a>,
        affinity_type: Option<&'a AffinityType>,
        affinity_key: Option<&'a [u8]>,
        ring: &'a parking_lot::RwLock<ConsistentHashRing>,
    ) -> Self {
        Self {
            upstreams,
            algorithm,
            conn_state,
            ewma_state,
            health_check_state,
            circuit_breaker,
            affinity_type,
            affinity_key,
            ring,
            tried: FxHashSet::default(),
            exclusions: SelectionExclusions::default(),
        }
    }

    /// Count how many backends are currently available for selection.
    #[inline]
    pub fn available_count(&self) -> usize {
        self.upstreams
            .iter()
            .filter(|u| {
                let active_healthy = self.health_check_state.is_none_or(|state_map| {
                    crate::health_check::is_upstream_healthy(state_map, &u.proxy_to)
                });
                let circuit_healthy = self.circuit_breaker.is_available(u);
                let not_selected = !self.tried.contains(*u);

                active_healthy && circuit_healthy && not_selected
            })
            .count()
    }

    /// Select the next backend for the request.
    ///
    /// Backends that are unhealthy, circuit-open, overloaded, or already
    /// tried by this request are skipped and reported in the returned
    /// exclusions. When no backend remains, `None` is returned and any
    /// exclusions recorded by the final round stay pending for
    /// [`Self::take_exclusions`].
    #[inline]
    pub fn next_backend(&mut self) -> Option<SelectionOutcome> {
        if self.upstreams.is_empty() {
            return None;
        }

        let mut unhealthy: FxHashSet<usize> = FxHashSet::default();
        for (i, u) in self.upstreams.iter().enumerate() {
            if let Some(state_map) = self.health_check_state {
                if !crate::health_check::is_upstream_healthy(state_map, &u.proxy_to) {
                    unhealthy.insert(i);
                }
            }
            if !unhealthy.contains(&i) && self.tried.contains(u) {
                unhealthy.insert(i);
                self.exclusions.already_tried.push(Arc::clone(u));
            }
        }

        // Fast path: single-priority short-circuit.
        // Most configs have a single priority value. Detect this without allocating
        // the BTreeMap by scanning healthy indices and checking if they share the
        // same priority.
        let first_healthy_priority = self
            .upstreams
            .iter()
            .enumerate()
            .find(|(i, _)| !unhealthy.contains(i))
            .map(|(_, u)| u.priority);

        let all_same_priority = first_healthy_priority.is_some_and(|p| {
            self.upstreams
                .iter()
                .enumerate()
                .filter(|(i, _)| !unhealthy.contains(i))
                .all(|(_, u)| u.priority == p)
        });

        if all_same_priority {
            // Single priority group: build the group inline without BTreeMap
            let mut group: Vec<usize> = self
                .upstreams
                .iter()
                .enumerate()
                .filter(|(i, _)| !unhealthy.contains(i))
                .map(|(i, _)| i)
                .collect();

            let mut affinity_index = self.resolve_affinity(&unhealthy);
            return self.try_select_from_group(&mut group, &mut affinity_index, &mut unhealthy);
        }

        // Multi-priority: Group healthy indices by priority (BTreeMap = sorted by key, lowest first)
        let mut priority_groups: BTreeMap<u16, Vec<usize>> = BTreeMap::new();
        for (i, u) in self.upstreams.iter().enumerate() {
            if !unhealthy.contains(&i) {
                priority_groups.entry(u.priority).or_default().push(i);
            }
        }

        // Resolve affinity once across all tiers
        let mut affinity_index = self.resolve_affinity(&unhealthy);

        // Try each priority group in order (lowest priority value = highest priority)
        for (_priority, mut group) in priority_groups {
            if let Some(result) =
                self.try_select_from_group(&mut group, &mut affinity_index, &mut unhealthy)
            {
                return Some(result);
            }
        }

        None
    }

    /// Take the exclusions recorded by the most recent selection round.
    ///
    /// After `next_backend` returns `None`, this exposes why the last round
    /// failed so the caller can still report the excluded backends.
    #[inline]
    pub fn take_exclusions(&mut self) -> SelectionExclusions {
        std::mem::take(&mut self.exclusions)
    }

    #[inline]
    fn resolve_affinity(&self, unhealthy: &FxHashSet<usize>) -> Option<usize> {
        if let (Some(affinity_type), Some(key)) = (self.affinity_type, self.affinity_key) {
            return super::affinity::resolve_affinity_index(
                affinity_type,
                key,
                self.upstreams,
                unhealthy,
                self.ring,
            );
        }
        None
    }

    /// Try to select a backend from a single priority group.
    ///
    /// Returns `Some(SelectionOutcome)` if a backend was selected, or `None`
    /// if the group is exhausted (all circuit-open or overloaded). On each
    /// call, the group shrinks as backends are tried.
    #[inline]
    fn try_select_from_group(
        &mut self,
        group: &mut Vec<usize>,
        affinity_index: &mut Option<usize>,
        unhealthy: &mut FxHashSet<usize>,
    ) -> Option<SelectionOutcome> {
        loop {
            if group.is_empty() {
                break;
            }

            // Resolve affinity: find position in group whose original index
            // matches affinity_index
            if affinity_index.is_none() {
                *affinity_index = self.resolve_affinity(unhealthy);
            }
            let start_pos = affinity_index.and_then(|aff_idx| {
                if aff_idx < group.len() && group[aff_idx] == aff_idx {
                    return Some(aff_idx);
                }
                group.iter().position(|orig_idx| *orig_idx == aff_idx)
            });

            let (index, candidate_scores) = if let Some(pos) = start_pos {
                (pos, Vec::new())
            } else if group.len() == 1 {
                (0, Vec::new())
            } else {
                let result = super::lb::selector::select_backend_index(
                    self.algorithm,
                    group,
                    self.upstreams,
                    self.conn_state,
                    self.ewma_state,
                    self.circuit_breaker.state(),
                    self.circuit_breaker.slow_start_duration(),
                );
                (result.index, result.candidate_scores)
            };
            let upstream_idx = group.swap_remove(index);
            unhealthy.insert(upstream_idx);
            let upstream = Arc::clone(&self.upstreams[upstream_idx]);
            if start_pos == Some(index) {
                *affinity_index = None;
            }

            if !self.circuit_breaker.try_acquire(&upstream) {
                if self.circuit_breaker.is_open(&upstream) {
                    self.exclusions.circuit_open.push(Arc::clone(&upstream));
                } else {
                    self.exclusions.overloaded.push(Arc::clone(&upstream));
                }
                continue;
            }

            // Only successfully acquired backends count as tried; backends
            // refused by the circuit breaker may recover (cooldown expiry,
            // half-open slot freed) and are re-offered on later retry rounds.
            self.tried.insert(Arc::clone(&upstream));

            // Get the tracker (already initialized by select_backend_index)
            super::lb::selector::initialize_tracker(self.conn_state, &upstream);
            let tracker = super::lb::selector::get_tracker(self.conn_state, &upstream);
            return Some(SelectionOutcome {
                upstream,
                tracker,
                candidate_scores,
                exclusions: std::mem::take(&mut self.exclusions),
            });
        }

        None
    }
}
