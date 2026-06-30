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
    active_health_check_state: Option<super::health::HealthCheckStateMap>,
) -> Vec<std::sync::Arc<super::upstream::UpstreamInner>> {
    let candidates = resolve_srv_inner(srv_data).await;

    if candidates.is_empty() {
        return Vec::new();
    }

    let priority_offset = srv_data.priority.unwrap_or(0);

    // Return all healthy backends with their final priority.
    // Each backend's priority = DNS SRV priority + config priority offset.
    // The top-level load balancer handles tiered failover across priorities.
    candidates
        .into_iter()
        .filter(move |(upstream, _, _)| {
            active_health_check_state.as_ref().is_none_or(|s| {
                s.get(upstream.proxy_to.as_str())
                    .is_none_or(|s| s.is_healthy)
            })
        })
        .map(|(upstream, dns_priority, _dns_weight)| {
            let priority = dns_priority.saturating_add(priority_offset);
            std::sync::Arc::new(super::upstream::UpstreamInner {
                proxy_to: upstream.proxy_to.clone(),
                proxy_unix: upstream.proxy_unix.clone(),
                weight: upstream.weight,
                mtls: upstream.mtls.clone(),
                priority,
            })
        })
        .collect()
}

#[cfg(feature = "srv-lookup")]
pub async fn resolve_srv_inner(
    srv_data: &super::upstream::SrvUpstreamData,
) -> Vec<(std::sync::Arc<super::upstream::UpstreamInner>, u16, u16)> {
    use hickory_resolver::config::{NameServerConfig, ResolverConfig};
    use hickory_resolver::TokioResolver;

    let srv_name = srv_data.srv_name.clone();
    let dns_servers = srv_data.dns_servers.clone();
    let weight = srv_data.weight;
    let mtls = srv_data.mtls.clone();

    // Get the secondary runtime handle (captured globally during Module::start)
    let (handle, event_sink) = match crate::try_get_secondary_runtime_handle() {
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
                    event_sink.emit(ferron_observability::Event::Log(
                        ferron_observability::LogEvent {
                            level: ferron_observability::LogLevel::Warn,
                            message: format!("Failed to create resolver: {}", e),
                            summary: "Failed to create DNS resolver".into(),
                            target: crate::LOG_TARGET,
                            attributes: vec![(
                                "error.message",
                                ferron_observability::LogAttributeValue::String(e.to_string()),
                            )],
                            trace_context: None,
                        },
                    ));
                    return Vec::new();
                }
            };

            // Perform SRV lookup
            let srv_records = match resolver.srv_lookup(&srv_name).await {
                Ok(records) => records,
                Err(e) => {
                    event_sink.emit(ferron_observability::Event::Log(
                        ferron_observability::LogEvent {
                            level: ferron_observability::LogLevel::Warn,
                            message: format!("SRV lookup failed for {}: {}", srv_name, e),
                            summary: "SRV lookup failed".into(),
                            target: crate::LOG_TARGET,
                            attributes: vec![(
                                "dns.name",
                                ferron_observability::LogAttributeValue::String(
                                    srv_name.to_string(),
                                ),
                            )],
                            trace_context: None,
                        },
                    ));
                    return Vec::new();
                }
            };

            // Parse the SRV records into upstream candidates
            let candidates: Vec<(std::sync::Arc<super::upstream::UpstreamInner>, u16, u16)> =
                srv_records
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
                        let upstream = std::sync::Arc::new(super::upstream::UpstreamInner {
                            proxy_to,
                            proxy_unix: None,
                            weight,
                            mtls: mtls.clone(),
                            priority: 0,
                        });
                        let priority = srv.priority;

                        Some((upstream, priority, srv.weight))
                    })
                    .collect();

            candidates
        })
        .await;

    result.unwrap_or_default()
}
