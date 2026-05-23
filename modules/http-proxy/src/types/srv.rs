//! SRV record resolution for dynamic upstream discovery.
//!
//! This module is placed in `types/` rather than `upstream/` to avoid a circular
//! dependency: `types/` needs the SRV resolution function for `Upstream::resolve`,
//! and `upstream/` needs `types/` for its type definitions. By keeping `resolve_srv`
//! here, `upstream/resolution.rs` can call it via `crate::types::resolve_srv`
//! without creating a cycle.

#[cfg(feature = "srv-lookup")]
pub async fn resolve_srv(
    srv_data: &super::upstream::SrvUpstreamData,
    failed_backends: std::sync::Arc<
        parking_lot::RwLock<crate::util::TtlCache<super::upstream::UpstreamInner, u64>>,
    >,
    health_check_max_fails: u64,
) -> Vec<super::upstream::UpstreamInner> {
    let candidates = resolve_srv_inner(srv_data).await;

    if candidates.is_empty() {
        return Vec::new();
    }

    // Filter out unhealthy backends
    let failed = failed_backends.read();
    let healthy: Vec<(super::upstream::UpstreamInner, u16, u16)> = candidates
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

    let filtered: Vec<(super::upstream::UpstreamInner, u16)> = healthy
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
}

#[cfg(feature = "srv-lookup")]
pub async fn resolve_srv_inner(
    srv_data: &super::upstream::SrvUpstreamData,
) -> Vec<(super::upstream::UpstreamInner, u16, u16)> {
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
            let candidates: Vec<(super::upstream::UpstreamInner, u16, u16)> = srv_records
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
                    let upstream = super::upstream::UpstreamInner {
                        proxy_to,
                        proxy_unix: None,
                        weight,
                    };
                    let priority = srv.priority;

                    Some((upstream, priority, srv.weight))
                })
                .collect();

            candidates
        })
        .await;

    result.unwrap_or_default()
}
