//! Upstream resolution and backend selection logic.

use std::sync::Arc;

use parking_lot::RwLock;

use crate::config::CircuitBreakerConfig;
use crate::types::circuit::CircuitBreakerStateMap;
use crate::types::health::HealthCheckStateMap;
use crate::types::lb::SelectedBackend;
use crate::types::upstream::{Upstream, UpstreamInner};
use crate::types::ConnectionsTrackState;
use crate::upstream::circuit::try_acquire_circuit_breaker_slot;
use crate::upstream::lb::LoadBalancerAlgorithmInner;
use crate::util::TtlCache;

/// Resolve all upstreams to a flat list of `UpstreamInner` entries.
///
/// For SRV upstreams, this performs DNS resolution. For static upstreams,
/// it returns them as-is.
pub async fn resolve_upstreams(
    upstreams: &[Upstream],
    failed_backends: Arc<RwLock<TtlCache<UpstreamInner, u64>>>,
    health_check_max_fails: u64,
) -> Vec<UpstreamInner> {
    let mut resolved = Vec::new();
    for upstream in upstreams {
        resolved.extend(
            upstream
                .resolve(Arc::clone(&failed_backends), health_check_max_fails)
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
    failed_backends: &parking_lot::RwLock<TtlCache<UpstreamInner, u64>>,
    health_check_enabled: bool,
    health_check_max_fails: u64,
    algorithm: &LoadBalancerAlgorithmInner,
    conn_state: Option<&ConnectionsTrackState>,
    health_check_state: Option<&HealthCheckStateMap>,
    circuit_breaker: &CircuitBreakerConfig,
    circuit_breaker_state: Option<&CircuitBreakerStateMap>,
    selected_backends: &[UpstreamInner],
    affinity_index: Option<usize>,
) -> Option<SelectedBackend> {
    if upstreams.is_empty() {
        return None;
    }

    // Build a mutable copy of healthy backends for the selection loop
    let mut healthy: Vec<UpstreamInner> = {
        let failed = if health_check_enabled {
            Some(failed_backends.read())
        } else {
            None
        };
        upstreams
            .iter()
            .filter(|u| {
                // Check passive failure cache
                let not_failed = failed.as_ref().is_none_or(|failed| {
                    failed
                        .get(*u)
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
            .cloned()
            .collect()
    };

    if healthy.is_empty() {
        return None;
    }

    let mut affinity_index = affinity_index;
    loop {
        if healthy.is_empty() {
            return None;
        }

        let index = if let Some(idx) = affinity_index.take() {
            if idx < healthy.len() {
                idx
            } else if healthy.len() == 1 {
                0
            } else {
                super::lb::selector::select_backend_index(algorithm, &healthy, conn_state, None)
            }
        } else if healthy.len() == 1 {
            0
        } else {
            super::lb::selector::select_backend_index(algorithm, &healthy, conn_state, None)
        };
        let upstream = healthy.remove(index);

        if !try_acquire_circuit_breaker_slot(circuit_breaker_state, circuit_breaker, &upstream) {
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
