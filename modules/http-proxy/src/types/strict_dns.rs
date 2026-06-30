//! Strict DNS (A/AAAA) resolution for upstream discovery.
//!
//! Resolves A and AAAA records for a hostname using Hickory, treating each
//! resolved IP as a distinct upstream backend. CNAME chains are followed
//! automatically by the resolver.

use std::net::SocketAddr;
use std::sync::Arc;

use crate::types::health::HealthCheckStateMap;
use crate::types::upstream::{UpstreamConfig, UpstreamInner};

/// Resolve A/AAAA records for the hostname in `UpstreamConfig`.
///
/// Each resolved IP becomes a separate `UpstreamInner` with `connect_to`
/// set to the IP address, while `proxy_to` retains the original hostname
/// for TLS SNI and HTTP request construction.
#[inline]
pub async fn resolve_strict_dns(
    cfg: &UpstreamConfig,
    active_health_check_state: Option<HealthCheckStateMap>,
) -> Vec<Arc<UpstreamInner>> {
    let candidates = resolve_strict_dns_inner(cfg).await;

    if candidates.is_empty() {
        return Vec::new();
    }

    candidates
        .into_iter()
        .filter(move |upstream| {
            active_health_check_state.as_ref().is_none_or(|s| {
                s.get(upstream.proxy_to.as_str())
                    .is_none_or(|s| s.is_healthy)
            })
        })
        .collect()
}

/// Low-level A/AAAA resolution.
///
/// Returns one `UpstreamInner` per resolved IP address. The hostname and
/// port are extracted from `cfg.url`.
///
/// Results are cached based on the minimum TTL from the DNS response.
#[inline]
pub async fn resolve_strict_dns_inner(cfg: &UpstreamConfig) -> Vec<Arc<UpstreamInner>> {
    let url = cfg.url.clone();
    let weight = cfg.weight;
    let mtls = cfg.mtls.clone();
    let priority = cfg.priority;
    let dns_servers = cfg.dns_servers.clone();

    // Parse hostname and port from the URL
    let (hostname, port) = match parse_host_port(&url) {
        Some(v) => v,
        None => return Vec::new(),
    };

    // Check cache first
    if let Some(cached) = super::dns_cache::get_strict_dns(&hostname, port, &dns_servers) {
        return cached;
    }

    // Get the secondary runtime handle (captured globally during Module::start)
    let (handle, event_sink) = match crate::try_get_secondary_runtime_handle() {
        Some(h) => h,
        None => {
            ferron_core::log_warn!(
                "Strict DNS resolution skipped — secondary runtime not yet available"
            );
            return Vec::new();
        }
    };

    // Get or create a cached resolver for these DNS servers
    let resolver = match crate::get_or_create_resolver(&dns_servers, handle).await {
        Some(r) => r,
        None => {
            event_sink.emit(ferron_observability::Event::Log(
                ferron_observability::LogEvent {
                    level: ferron_observability::LogLevel::Warn,
                    message: "Failed to create DNS resolver for strict DNS".to_string(),
                    summary: "Failed to create DNS resolver".into(),
                    target: crate::LOG_TARGET,
                    attributes: Vec::new(),
                    trace_context: None,
                },
            ));
            return Vec::new();
        }
    };

    let mut upstreams = Vec::new();

    // Resolve A and AAAA records using lookup_ip
    match resolver.lookup_ip(hostname.clone()).await {
        Ok(lookup) => {
            // Extract per-record TTL from the raw DNS records
            let ttls =
                lookup
                    .as_lookup()
                    .answers()
                    .iter()
                    .filter_map(|record| match &record.data {
                        hickory_proto::rr::RData::A(_) | hickory_proto::rr::RData::AAAA(_) => {
                            Some(record.ttl)
                        }
                        _ => None,
                    });
            let ttl = super::dns_cache::ttl_from_records(ttls);

            for ip in lookup.iter() {
                let scheme = if url.starts_with("https://") {
                    "https"
                } else {
                    "http"
                };
                let proxy_to = format!("{}://{}:{}", scheme, hostname, port);
                upstreams.push(Arc::new(UpstreamInner {
                    proxy_to,
                    connect_to: Some(SocketAddr::new(ip, port)),
                    proxy_unix: None,
                    weight,
                    mtls: mtls.clone(),
                    priority,
                }));
            }

            // Cache the result
            if !upstreams.is_empty() {
                super::dns_cache::insert_strict_dns(
                    &hostname,
                    port,
                    &dns_servers,
                    upstreams.clone(),
                    ttl,
                );
            }
        }
        Err(e) => {
            // NXDOMAIN or no records — not necessarily an error
            event_sink.emit(ferron_observability::Event::Log(
                ferron_observability::LogEvent {
                    level: ferron_observability::LogLevel::Debug,
                    message: format!("No A/AAAA records for {}: {}", hostname, e),
                    summary: "No A/AAAA records".into(),
                    target: crate::LOG_TARGET,
                    attributes: vec![(
                        "dns.name",
                        ferron_observability::LogAttributeValue::String(hostname),
                    )],
                    trace_context: None,
                },
            ));
        }
    }

    upstreams
}

/// Extract hostname and port from a URL string.
///
/// Returns `(hostname, port)` or `None` if parsing fails.
pub fn parse_host_port(url: &str) -> Option<(String, u16)> {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))?;

    let default_port = if url.starts_with("https://") { 443 } else { 80 };

    // Handle IPv6 brackets: [::1]:8080
    if rest.starts_with('[') {
        let close = rest.find(']')?;
        let ipv6 = &rest[1..close];
        let after = &rest[close + 1..];
        let port = if let Some(stripped) = after.strip_prefix(':') {
            stripped.parse().ok()?
        } else {
            default_port
        };
        return Some((ipv6.to_string(), port));
    }

    // Handle hostname:port or IPv4:port
    let (host, port_str) = match rest.rfind(':') {
        Some(pos) => (&rest[..pos], &rest[pos + 1..]),
        None => (rest, ""),
    };

    let port = if port_str.is_empty() {
        default_port
    } else {
        port_str.parse().ok()?
    };

    Some((host.to_string(), port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_host_port_hostname() {
        assert_eq!(
            parse_host_port("http://myapp.example.com:8080"),
            Some(("myapp.example.com".to_string(), 8080))
        );
    }

    #[test]
    fn test_parse_host_port_default_port() {
        assert_eq!(
            parse_host_port("http://myapp.example.com"),
            Some(("myapp.example.com".to_string(), 80))
        );
    }

    #[test]
    fn test_parse_host_port_https() {
        assert_eq!(
            parse_host_port("https://myapp.example.com"),
            Some(("myapp.example.com".to_string(), 443))
        );
    }

    #[test]
    fn test_parse_host_port_ipv4() {
        assert_eq!(
            parse_host_port("http://1.2.3.4:8080"),
            Some(("1.2.3.4".to_string(), 8080))
        );
    }

    #[test]
    fn test_parse_host_port_ipv6() {
        assert_eq!(
            parse_host_port("http://[::1]:8080"),
            Some(("::1".to_string(), 8080))
        );
    }

    #[test]
    fn test_parse_host_port_ipv6_no_port() {
        assert_eq!(
            parse_host_port("http://[::1]"),
            Some(("::1".to_string(), 80))
        );
    }

    #[test]
    fn test_parse_host_port_invalid() {
        assert_eq!(parse_host_port("not-a-url"), None);
        assert_eq!(parse_host_port(""), None);
    }
}
