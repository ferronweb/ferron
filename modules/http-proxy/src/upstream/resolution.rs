//! Upstream resolution and backend selection logic.

use std::sync::Arc;

use crate::config::{AffinityType, CircuitBreakerConfig};
use crate::types::circuit::CircuitBreakerStateMap;
use crate::types::health::HealthCheckStateMap;
use crate::types::lb::SelectedBackend;
use crate::types::upstream::{Upstream, UpstreamInner};
use crate::types::ConnectionsTrackState;
use crate::upstream::circuit::try_acquire_circuit_breaker_slot;
use crate::upstream::lb::{ConsistentHashRing, EwmaStateMap, LoadBalancerAlgorithmInner};
use crate::util::FailureCache;

/// Resolve all upstreams to a flat list of `UpstreamInner` entries.
///
/// For SRV upstreams, this performs DNS resolution. For static upstreams,
/// it returns them as-is.
pub async fn resolve_upstreams(
    upstreams: &[Upstream],
    failed_backends: Arc<FailureCache>,
    health_check_max_fails: u64,
    active_health_check_state: Option<HealthCheckStateMap>,
) -> Vec<UpstreamInner> {
    let mut resolved = Vec::new();
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
    upstreams: &[UpstreamInner],
    failed_backends: &FailureCache,
    health_check_enabled: bool,
    health_check_max_fails: u64,
    algorithm: &LoadBalancerAlgorithmInner,
    conn_state: Option<&ConnectionsTrackState>,
    ewma_state: Option<&EwmaStateMap>,
    health_check_state: Option<&HealthCheckStateMap>,
    circuit_breaker: &CircuitBreakerConfig,
    circuit_breaker_state: Option<&CircuitBreakerStateMap>,
    selected_backends: &[UpstreamInner],
    affinity_type: Option<&AffinityType>,
    affinity_key: Option<&[u8]>,
    ring: &parking_lot::RwLock<ConsistentHashRing>,
    event_sink: &ferron_observability::CompositeEventSink,
) -> Option<SelectedBackend> {
    if upstreams.is_empty() {
        return None;
    }

    // Build healthy list with original indices preserved
    let mut healthy: Vec<(usize, UpstreamInner)> = {
        let failed = if health_check_enabled {
            Some(failed_backends.read())
        } else {
            None
        };
        upstreams
            .iter()
            .cloned()
            .enumerate()
            .filter(|(_, u)| {
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
                let not_selected = !selected_backends.contains(u);

                not_failed && active_healthy && not_selected
            })
            .collect()
    };

    let mut affinity_index = None;

    loop {
        if healthy.is_empty() {
            return None;
        }

        // Resolve affinity: find position in `healthy` whose original index matches affinity_index
        if affinity_index.is_none() {
            if let (Some(affinity_type), Some(key)) = (affinity_type, affinity_key) {
                affinity_index =
                    super::affinity::resolve_affinity_index(affinity_type, key, upstreams, ring);
            };
        }
        let start_pos = affinity_index.and_then(|aff_idx| {
            // Fast path: if affine backend is still at same position in filtered list
            if aff_idx < healthy.len() {
                if let Some((orig_idx, _)) = healthy.get(aff_idx) {
                    if *orig_idx == aff_idx {
                        return Some(aff_idx);
                    }
                }
            }
            // Fallback: search by original index identity
            healthy
                .iter()
                .position(|(orig_idx, _)| *orig_idx == aff_idx)
        });

        let index = if let Some(pos) = start_pos {
            pos
        } else if healthy.len() == 1 {
            0
        } else {
            super::lb::selector::select_backend_index(algorithm, &healthy, conn_state, ewma_state)
        };
        let (_, upstream) = healthy.remove(index);
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

        if health_check_enabled {
            let failed = failed_backends.read();
            if let Some(fails) = failed.get(&upstream) {
                if fails > health_check_max_fails {
                    continue; // Skip unhealthy, try next
                }
            }
        }

        // Get the tracker (already initialized by select_backend_index)
        super::lb::selector::initialize_tracker(conn_state, &upstream);
        let tracker = super::lb::selector::get_tracker(conn_state, &upstream);
        return Some(SelectedBackend { upstream, tracker });
    }
}
