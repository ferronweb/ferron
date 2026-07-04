//! Upstream resolution and backend selection logic.

use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use rustc_hash::FxHashSet;

use crate::config::CircuitBreakerConfig;
use crate::types::circuit::{CircuitBreakerStateMap, CIRCUIT_BREAKER_STATUS_OPEN};
use crate::types::health::HealthCheckStateMap;
use crate::types::lb::SelectedBackend;
use crate::types::upstream::{Upstream, UpstreamInner};
use crate::types::ConnectionsTrackState;
use crate::upstream::circuit::try_acquire_circuit_breaker_slot;
use crate::upstream::lb::{ConsistentHashRing, EwmaStateMap, LoadBalancerAlgorithmInner};

/// Resolve all upstreams to a flat list of `Arc<UpstreamInner>` entries.
///
/// For SRV upstreams, this performs DNS resolution. For static upstreams,
/// it returns them as-is.
#[inline]
pub async fn resolve_upstreams(
    upstreams: &[Upstream],
    active_health_check_state: Option<HealthCheckStateMap>,
) -> Vec<Arc<UpstreamInner>> {
    // Capacity of at least the number of upstreams to avoid reallocations in many cases.
    let mut resolved = Vec::with_capacity(upstreams.len());
    for upstream in upstreams {
        resolved.extend(upstream.resolve(active_health_check_state.clone()).await);
    }
    resolved
}

/// Determines which backend server to proxy the request to.
///
/// Returns the selected upstream and its connection tracker (if applicable).
/// Filters out unhealthy backends when health checking is enabled.
///
/// Backends are grouped by priority (lower value = higher priority). The
/// highest-priority tier is tried first. When all backends in a tier are
/// unavailable (unhealthy, circuit-open, or already-tried), the next tier
/// is used as a fallback.
#[allow(clippy::too_many_arguments)]
pub fn determine_proxy_to(
    upstreams: &[Arc<UpstreamInner>],
    algorithm: &LoadBalancerAlgorithmInner,
    conn_state: Option<&ConnectionsTrackState>,
    ewma_state: Option<&EwmaStateMap>,
    health_check_state: Option<&HealthCheckStateMap>,
    circuit_breaker: &CircuitBreakerConfig,
    circuit_breaker_state: Option<&CircuitBreakerStateMap>,
    flapping_state: Option<&crate::types::flapping::FlappingStateMap>,
    affinity_type: Option<&crate::config::AffinityType>,
    affinity_key: Option<&[u8]>,
    ring: &parking_lot::RwLock<ConsistentHashRing>,
    event_sink: &ferron_observability::CompositeEventSink,
    metrics: &mut crate::ProxyMetrics,
    event_trace_context: Option<ferron_observability::EventTraceContext>,
) -> Option<SelectedBackend> {
    if upstreams.is_empty() {
        return None;
    }

    // Build unhealthy set — health checks + already-tried backends
    let mut unhealthy: FxHashSet<usize> = FxHashSet::default();
    for (i, u) in upstreams.iter().enumerate() {
        if let Some(state_map) = health_check_state {
            if !crate::health_check::is_upstream_healthy(state_map, &u.proxy_to) {
                unhealthy.insert(i);
            }
        }
        if !unhealthy.contains(&i) && metrics.selected_backends.contains(u) {
            unhealthy.insert(i);
            metrics.excluded_already_tried.push(Arc::clone(u));
        }
    }

    // Group healthy indices by priority (BTreeMap = sorted by key, lowest first)
    let mut priority_groups: BTreeMap<u16, Vec<usize>> = BTreeMap::new();
    for (i, u) in upstreams.iter().enumerate() {
        if !unhealthy.contains(&i) {
            priority_groups.entry(u.priority).or_default().push(i);
        }
    }

    // Resolve affinity once across all tiers
    let mut affinity_index = None;
    if let (Some(affinity_type), Some(key)) = (affinity_type, affinity_key) {
        affinity_index = super::affinity::resolve_affinity_index(
            affinity_type,
            key,
            upstreams,
            &unhealthy,
            ring,
        );
    };

    // Try each priority group in order (lowest priority value = highest priority)
    for (_priority, mut group) in priority_groups {
        loop {
            if group.is_empty() {
                break;
            }

            // Resolve affinity: find position in group whose original index
            // matches affinity_index
            if affinity_index.is_none() {
                if let (Some(affinity_type), Some(key)) = (affinity_type, affinity_key) {
                    affinity_index = super::affinity::resolve_affinity_index(
                        affinity_type,
                        key,
                        upstreams,
                        &unhealthy,
                        ring,
                    );
                };
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
                    algorithm,
                    &group,
                    upstreams,
                    conn_state,
                    ewma_state,
                    circuit_breaker_state,
                    circuit_breaker.slow_start_duration,
                );
                (result.index, result.candidate_scores)
            };
            let upstream_idx = group.swap_remove(index);
            unhealthy.insert(upstream_idx);
            let upstream = Arc::clone(&upstreams[upstream_idx]);
            if start_pos == Some(index) {
                affinity_index = None;
            }

            if !try_acquire_circuit_breaker_slot(
                circuit_breaker_state,
                flapping_state,
                circuit_breaker,
                &upstream,
                event_sink,
                event_trace_context.clone(),
            ) {
                let open = circuit_breaker
                    .enabled
                    .then_some(circuit_breaker_state)
                    .flatten()
                    .and_then(|s| s.get(&upstream))
                    .is_some_and(|s| {
                        s.status.load(Ordering::Relaxed) == CIRCUIT_BREAKER_STATUS_OPEN
                    });

                if open {
                    metrics.excluded_circuit_open.push(Arc::clone(&upstream));
                } else {
                    metrics.excluded_overloaded.push(Arc::clone(&upstream));
                }

                continue;
            }

            // Get the tracker (already initialized by select_backend_index)
            super::lb::selector::initialize_tracker(conn_state, &upstream);
            let tracker = super::lb::selector::get_tracker(conn_state, &upstream);
            return Some(SelectedBackend {
                upstream,
                tracker,
                candidate_scores,
            });
        }
    }

    None
}
