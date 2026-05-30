//! Upstream resolution and backend selection logic.

use std::sync::Arc;

use rustc_hash::FxHashSet;

use crate::config::{AffinityType, CircuitBreakerConfig};
use crate::types::circuit::CircuitBreakerStateMap;
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
    selected_backends: &FxHashSet<Arc<UpstreamInner>>,
    affinity_type: Option<&AffinityType>,
    affinity_key: Option<&[u8]>,
    ring: &parking_lot::RwLock<ConsistentHashRing>,
    event_sink: &ferron_observability::CompositeEventSink,
) -> Option<SelectedBackend> {
    if upstreams.is_empty() {
        return None;
    }

    // Build healthy list of indices into `upstreams` — avoids Arc clones
    // until the final selection.
    let mut unhealthy: FxHashSet<usize> = FxHashSet::default();
    let mut healthy: Vec<usize> = {
        let failed = if health_check_enabled {
            Some(failed_backends)
        } else {
            None
        };
        upstreams
            .iter()
            .enumerate()
            .filter(|(i, u)| {
                // Check passive failure cache
                let not_failed = failed.as_ref().is_none_or(|failed| {
                    failed
                        .get(u)
                        .is_none_or(|fails| fails <= health_check_max_fails)
                });

                // Check active health check state
                let active_healthy = if let Some(state_map) = health_check_state {
                    crate::health_check::is_upstream_healthy(state_map, &u.proxy_to)
                } else {
                    true
                };

                // Check if backend is already selected
                let not_selected = !selected_backends.contains(*u);

                let healthy = not_failed && active_healthy && not_selected;
                if !healthy {
                    unhealthy.insert(*i);
                }
                healthy
            })
            .map(|(i, _)| i)
            .collect()
    };

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
        ) {
            continue;
        }

        // Get the tracker (already initialized by select_backend_index)
        super::lb::selector::initialize_tracker(conn_state, &upstream);
        let tracker = super::lb::selector::get_tracker(conn_state, &upstream);
        return Some(SelectedBackend { upstream, tracker });
    }
}
