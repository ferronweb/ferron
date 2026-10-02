//! SRV record resolution for dynamic upstream discovery.

#[inline]
pub async fn resolve_srv(
    srv_data: &super::upstream::SrvUpstream,
) -> Vec<std::sync::Arc<super::upstream::ResolvedUpstream>> {
    let candidates = resolve_srv_inner(srv_data).await;

    if candidates.is_empty() {
        return Vec::new();
    }

    let priority_offset = srv_data.priority;

    // Return all backends with their final priority.
    // Each backend's priority = DNS SRV priority + config priority offset.
    // The top-level load balancer handles tiered failover across priorities.
    candidates
        .into_iter()
        .map(|(upstream, dns_priority, dns_weight)| {
            let mut new_inner = upstream.inner.clone();
            new_inner.priority = dns_priority.saturating_add(priority_offset);
            new_inner.weight = (dns_weight as u32).saturating_mul(upstream.weight);
            std::sync::Arc::new(super::upstream::ResolvedUpstream {
                proxy_to: upstream.proxy_to.clone(),
                connect_to: None,
                proxy_unix: upstream.proxy_unix.clone(),
                inner: new_inner,
                dns_status: super::upstream::DnsResolutionStatus::Resolved,
            })
        })
        .collect()
}

#[inline]
pub async fn resolve_srv_inner(
    srv_data: &super::upstream::SrvUpstream,
) -> Vec<(std::sync::Arc<super::upstream::ResolvedUpstream>, u16, u16)> {
    if let Some(cached) = super::dns_cache::get_srv(&srv_data.srv_name, &srv_data.dns_servers).await
    {
        return cached;
    }

    let (handle, event_sink) = match crate::runtime_handle::try_get_secondary_runtime_handle() {
        Some(h) => h,
        None => {
            ferron_core::log_warn!("SRV resolution skipped — secondary runtime not yet available");
            return Vec::new();
        }
    };

    let srv_name = srv_data.srv_name.clone();
    let dns_servers = srv_data.dns_servers.clone();
    let srv_inner = srv_data.inner.clone();

    // Spawn SRV lookup on the secondary Tokio runtime
    let result = handle
        .spawn(async move {
            let resolver = match crate::runtime_handle::get_or_create_resolver(&dns_servers) {
                Some(r) => r,
                None => {
                    event_sink.emit(ferron_observability::Event::Log(
                        ferron_observability::LogEvent {
                            level: ferron_observability::LogLevel::Warn,
                            message: "Failed to create DNS resolver".to_string(),
                            summary: "Failed to create DNS resolver".into(),
                            target: crate::LOG_TARGET,
                            attributes: Vec::new(),
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
                            attributes: vec![
                                (
                                    "dns.name",
                                    ferron_observability::LogAttributeValue::String(
                                        srv_name.to_string(),
                                    ),
                                ),
                                (
                                    "error.message",
                                    ferron_observability::LogAttributeValue::String(e.to_string()),
                                ),
                            ],
                            trace_context: None,
                        },
                    ));
                    return Vec::new();
                }
            };

            // Calculate TTL for SRV records
            let ttl = srv_records
                .valid_until()
                .saturating_duration_since(std::time::Instant::now());

            let mut candidates: Vec<(std::sync::Arc<super::upstream::ResolvedUpstream>, u16, u16)> =
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
                        let upstream = std::sync::Arc::new(super::upstream::ResolvedUpstream {
                            proxy_to,
                            connect_to: None,
                            proxy_unix: None,
                            inner: srv_inner.clone(),
                            dns_status: super::upstream::DnsResolutionStatus::Resolved,
                        });

                        Some((upstream, srv_inner.priority, srv.weight))
                    })
                    .collect();

            candidates.sort_unstable_by(|a, b| {
                (&a.0.proxy_to, &a.0.proxy_unix, &a.0.connect_to).cmp(&(
                    &b.0.proxy_to,
                    &b.0.proxy_unix,
                    &b.0.connect_to,
                ))
            });

            // Store the candidates in the cache
            super::dns_cache::insert_srv(&srv_name, &dns_servers, candidates.clone(), ttl);

            candidates
        })
        .await;

    result.unwrap_or_default()
}
