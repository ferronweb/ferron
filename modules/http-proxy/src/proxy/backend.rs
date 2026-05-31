//! Backend health checking and availability counting.

use std::sync::Arc;

use crate::config::CircuitBreakerConfig;
use crate::types::circuit::CircuitBreakerStateMap;
use crate::types::health::HealthCheckStateMap;
use crate::types::upstream::UpstreamInner;
use crate::upstream::FailureCache;

/// Count how many backends are currently available for selection.
#[allow(clippy::too_many_arguments)]
#[inline]
pub fn count_available_backends(
    upstreams: &[Arc<UpstreamInner>],
    failed_backends: &FailureCache,
    health_check_max_fails: u64,
    health_check_state: Option<&HealthCheckStateMap>,
    circuit_breaker_state: Option<&CircuitBreakerStateMap>,
    circuit_breaker: &CircuitBreakerConfig,
    selected_backends: &rustc_hash::FxHashSet<Arc<UpstreamInner>>,
    config_key: &[usize],
) -> usize {
    upstreams
        .iter()
        .filter(|u| {
            let passive_healthy = failed_backends
                .get(&(u.to_owned().clone(), config_key.to_vec()))
                .is_none_or(|fails| fails <= health_check_max_fails);
            let active_healthy = health_check_state.is_none_or(|state_map| {
                crate::health_check::is_upstream_healthy(state_map, &u.proxy_to)
            });
            let circuit_healthy = crate::upstream::is_circuit_breaker_available(
                circuit_breaker_state,
                circuit_breaker,
                u,
            );
            let not_selected = !selected_backends.contains(*u);

            passive_healthy && active_healthy && circuit_healthy && not_selected
        })
        .count()
}
