//! Upstream resolution and backend selection logic.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use rustc_hash::FxHashSet;

use crate::config::{AffinityType, CircuitBreakerConfig};
use crate::types::circuit::{CircuitBreakerStateMap, CIRCUIT_BREAKER_STATUS_OPEN};
use crate::types::health::HealthCheckStateMap;
use crate::types::lb::SelectedBackend;
use crate::types::upstream::{Upstream, UpstreamInner};
use crate::types::ConnectionsTrackState;
use crate::upstream::circuit::try_acquire_circuit_breaker_slot;
use crate::upstream::lb::{ConsistentHashRing, EwmaStateMap, LoadBalancerAlgorithmInner};
use crate::upstream::FailureCache;

/// Resolve all upstreams to a flat list of `Arc<UpstreamInner>` entries.
///
/// For SRV upstreams, this performs DNS resolution. For static upstreams,
/// it returns them as-is.
#[inline]
pub async fn resolve_upstreams(
    upstreams: &[Upstream],
    failed_backends: Arc<FailureCache>,
    health_check_max_fails: u64,
    active_health_check_state: Option<HealthCheckStateMap>,
    config_key: &[usize],
) -> Vec<Arc<UpstreamInner>> {
    // Capacity of at least the number of upstreams to avoid reallocations in many cases.
    let mut resolved = Vec::with_capacity(upstreams.len());
    for upstream in upstreams {
        resolved.extend(
            upstream
                .resolve(
                    Arc::clone(&failed_backends),
                    health_check_max_fails,
                    active_health_check_state.clone(),
                    config_key,
                )
                .await,
        );
    }
    resolved
}

/// Determines which backend server to proxy the request to.
///
/// Returns the selected upstream and its connection tracker (if applicable).
/// Filters out unhealthy backends when health checking is enabled.
#[allow(clippy::too_many_arguments)]
pub fn determine_proxy_to(
    upstreams: &[Arc<UpstreamInner>],
    failed_backends: &FailureCache,
    health_check_enabled: bool,
    health_check_max_fails: u64,
    algorithm: &LoadBalancerAlgorithmInner,
    conn_state: Option<&ConnectionsTrackState>,
    ewma_state: Option<&EwmaStateMap>,
    health_check_state: Option<&HealthCheckStateMap>,
    circuit_breaker: &CircuitBreakerConfig,
    circuit_breaker_state: Option<&CircuitBreakerStateMap>,
    affinity_type: Option<&AffinityType>,
    affinity_key: Option<&[u8]>,
    ring: &parking_lot::RwLock<ConsistentHashRing>,
    event_sink: &ferron_observability::CompositeEventSink,
    metrics: &mut crate::ProxyMetrics,
    config_key: &[usize],
    event_trace_context: Option<ferron_observability::EventTraceContext>,
) -> Option<SelectedBackend> {
    if upstreams.is_empty() {
        return None;
    }

    // Build healthy list of indices into `upstreams` — avoids Arc clones
    // until the final selection, while tracking exclusion reasons.
    let mut unhealthy: FxHashSet<usize> = FxHashSet::default();
    let mut healthy: Vec<usize> = upstreams
        .iter()
        .enumerate()
        .filter_map(|(i, u)| {
            // Check passive failure cache
            if health_check_enabled {
                if let Some(fails) = failed_backends.get(&(u.clone(), config_key.to_vec())) {
                    if fails > health_check_max_fails {
                        unhealthy.insert(i);
                        metrics.excluded_passive.push(Arc::clone(u));
                        return None;
                    }
                }
            }

            // Check active health check state
            if let Some(state_map) = health_check_state {
                if !crate::health_check::is_upstream_healthy(state_map, &u.proxy_to) {
                    // Active health exclusion is tracked via the existing
                    // active_unhealthy_backends metric — no separate exclusion
                    // metric needed.
                    unhealthy.insert(i);
                    return None;
                }
            }

            // Check if backend is already selected (retry loop)
            if metrics.selected_backends.contains(u) {
                unhealthy.insert(i);
                metrics.excluded_already_tried.push(Arc::clone(u));
                return None;
            }

            Some(i)
        })
        .collect();

    let mut affinity_index = None;

    loop {
        if healthy.is_empty() {
            return None;
        }

        // Resolve affinity: find position in `healthy` whose original index
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
            // Fast path: if affine backend is still at same position in
            // filtered list
            if aff_idx < healthy.len() && healthy[aff_idx] == aff_idx {
                return Some(aff_idx);
            }
            // Fallback: search by original index identity
            healthy.iter().position(|orig_idx| *orig_idx == aff_idx)
        });

        let index = if let Some(pos) = start_pos {
            pos
        } else if healthy.len() == 1 {
            0
        } else {
            super::lb::selector::select_backend_index(
                algorithm, &healthy, upstreams, conn_state, ewma_state,
            )
        };
        let upstream_idx = healthy.swap_remove(index);
        unhealthy.insert(upstream_idx);
        let upstream = Arc::clone(&upstreams[upstream_idx]);
        if start_pos == Some(index) {
            // Affine backend is no longer healthy; reset affinity index
            affinity_index = None;
        }

        if !try_acquire_circuit_breaker_slot(
            circuit_breaker_state,
            circuit_breaker,
            &upstream,
            event_sink,
            event_trace_context.clone(),
        ) {
            // Slot acquisition may have failed due to a race — treat as overloaded
            let open = circuit_breaker
                .enabled
                .then_some(circuit_breaker_state)
                .flatten()
                .and_then(|s| s.get(&upstream))
                .is_some_and(|s| s.status.load(Ordering::Relaxed) == CIRCUIT_BREAKER_STATUS_OPEN);

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
        return Some(SelectedBackend { upstream, tracker });
    }
}
