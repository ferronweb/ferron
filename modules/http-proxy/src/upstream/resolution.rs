//! Upstream resolution and backend selection logic.

use std::sync::Arc;

use parking_lot::RwLock;

use super::circuit::try_acquire_circuit_breaker_slot;
use super::lb::LoadBalancerAlgorithmInner;
use super::types::ConnectionsTrackState;
#[cfg(feature = "srv-lookup")]
use super::SrvUpstreamData;
use super::{CircuitBreakerStateMap, HealthCheckStateMap, Upstream, UpstreamInner};
use crate::config::CircuitBreakerConfig;
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
) -> Option<super::lb::SelectedBackend> {
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
        return Some(super::lb::SelectedBackend { upstream, tracker });
    }
}

#[cfg(feature = "srv-lookup")]
pub async fn resolve_srv(
    srv_data: &SrvUpstreamData,
    failed_backends: std::sync::Arc<
        parking_lot::RwLock<crate::util::TtlCache<super::UpstreamInner, u64>>,
    >,
    health_check_max_fails: u64,
) -> Vec<super::UpstreamInner> {
    use hickory_resolver::config::{NameServerConfig, ResolverConfig};
    use hickory_resolver::TokioResolver;

    let srv_name = srv_data.srv_name.clone();
    let dns_servers = srv_data.dns_servers.clone();
    let weight = srv_data.weight;

    // Get the secondary runtime handle (captured globally during Module::start)
    let handle = match crate::try_get_secondary_runtime_handle() {
        Some(h) => h,
        None => {
            ferron_core::log_warn!("SRV resolution skipped — secondary runtime not yet available");
            return Vec::new();
        }
    };

    // Spawn SRV lookup on the secondary Tokio runtime
    let result = handle
        .spawn(async move {
            use hickory_resolver::net::runtime::TokioRuntimeProvider;

            // Build resolver inside the spawned task (we're on the secondary runtime)
            let resolver_result = if !dns_servers.is_empty() {
                let mut resolver_config = ResolverConfig::default();
                for server in &dns_servers {
                    resolver_config.add_name_server(NameServerConfig::udp(*server));
                }
                TokioResolver::builder_with_config(resolver_config, TokioRuntimeProvider::new())
                    .build()
            } else {
                TokioResolver::builder_tokio()
                    .unwrap_or_else(|_| {
                        TokioResolver::builder_with_config(
                            ResolverConfig::default(),
                            TokioRuntimeProvider::new(),
                        )
                    })
                    .build()
            };
            let resolver = match resolver_result {
                Ok(resolver) => resolver,
                Err(e) => {
                    ferron_core::log_warn!("Failed to create resolver: {}", e);
                    return Vec::new();
                }
            };

            // Perform SRV lookup
            let srv_records = match resolver.srv_lookup(&srv_name).await {
                Ok(records) => records,
                Err(e) => {
                    ferron_core::log_warn!("SRV lookup failed for {}: {}", srv_name, e);
                    return Vec::new();
                }
            };

            // Parse the SRV records into upstream candidates
            let candidates: Vec<(super::UpstreamInner, u16, u16)> = srv_records
                .answers()
                .iter()
                .filter_map(|record| {
                    let srv = match &record.data {
                        hickory_proto::rr::RData::SRV(srv) => srv,
                        _ => return None,
                    };

                    let target = srv.target.to_string();
                    let port = srv.port;

                    let proxy_to = format!("http://{}:{}", target.trim_end_matches('.'), port);
                    let upstream = super::UpstreamInner {
                        proxy_to,
                        proxy_unix: None,
                        weight,
                    };
                    let priority = srv.priority;

                    Some((upstream, priority, srv.weight))
                })
                .collect();

            if candidates.is_empty() {
                return Vec::new();
            }

            // Filter out unhealthy backends
            let failed = failed_backends.read();
            let healthy: Vec<(super::UpstreamInner, u16, u16)> = candidates
                .into_iter()
                .filter(|(upstream, _, _)| {
                    failed
                        .get(upstream)
                        .is_none_or(|fails| fails <= health_check_max_fails)
                })
                .collect();
            drop(failed);

            if healthy.is_empty() {
                return Vec::new();
            }

            // Select the highest-priority group (lowest numeric value)
            let highest_priority = healthy
                .iter()
                .map(|(_, _, priority)| *priority)
                .min()
                .unwrap_or(0);

            let filtered: Vec<(super::UpstreamInner, u16)> = healthy
                .into_iter()
                .filter(|(_, _, priority)| *priority == highest_priority)
                .map(|(upstream, weight, _)| (upstream, weight))
                .collect();

            // Weighted random selection
            let cumulative_weight: u32 = filtered.iter().map(|(_, w)| *w as u32).sum();
            if cumulative_weight == 0 {
                return filtered.into_iter().map(|(u, _)| u).collect();
            }

            let mut random_weight = rand::random_range(0..cumulative_weight);
            for (upstream, weight) in filtered {
                if random_weight < weight as u32 {
                    return vec![upstream];
                }
                random_weight -= weight as u32;
            }

            Vec::new()
        })
        .await;

    result.unwrap_or_default()
}
