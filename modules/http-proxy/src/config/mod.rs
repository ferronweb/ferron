//! Configuration parsing for the reverse proxy module.

mod directives;
mod resilience;
mod types;
mod upstream;

use std::error::Error;
use std::sync::LazyLock;
use std::time::Duration;

use dashmap::DashMap;
use ferron_core::config::{ServerConfigurationBlock, ServerConfigurationValue};

pub use self::types::{CircuitBreakerConfig, ProxyConfig, RetryBudgetConfig};
pub use crate::types::affinity::{AffinityConfig, AffinityType};

use self::directives::{parse_affinity_entry, parse_request_header_entry};
use self::resilience::{parse_circuit_breaker, parse_retry_budget};
use self::upstream::parse_upstream_entry;
use crate::types::lb::LoadBalancerAlgorithm;
use crate::types::upstream::ProxyHeader;

/// Default keep-alive idle timeout in milliseconds.
pub(super) const DEFAULT_KEEPALIVE_IDLE_TIMEOUT_MS: u64 = 60_000;
/// Default connection timeout in milliseconds.
pub(super) const DEFAULT_CONNECTION_TIMEOUT_MS: u64 = 5_000;
/// mTLS file cache
pub static MTLS_FILE_CACHE: LazyLock<DashMap<String, std::sync::Arc<Vec<u8>>>> =
    LazyLock::new(DashMap::new);

/// Parse proxy configuration from a server configuration block.
#[inline]
pub fn parse_proxy_config(
    ctx: &ferron_http::HttpContext,
) -> Result<Option<ProxyConfig>, Box<dyn Error + Send + Sync>> {
    let entries = ctx.configuration.get_entries("proxy", true);
    if entries.is_empty() {
        return Ok(None);
    }

    let entry = entries[0];
    let mut cfg = ProxyConfig::default();

    let default_timeout = Duration::from_millis(DEFAULT_KEEPALIVE_IDLE_TIMEOUT_MS);
    for arg in &entry.args {
        if let Some(url) = arg.as_string_with_interpolations(ctx) {
            cfg.upstreams.push(crate::types::upstream::Upstream::Static(
                crate::types::upstream::UpstreamConfig {
                    url: url.clone(),
                    unix_socket: None,
                    limit: None,
                    health_check_config: crate::types::health::UpstreamHealthCheckConfig::default(),
                    weight: 1,
                    mtls: None,
                    priority: 0,
                    logical_dns: false,
                    dns_servers: Vec::new(),
                    connection_timeout: Some(Duration::from_millis(DEFAULT_CONNECTION_TIMEOUT_MS)),
                    idle_timeout: default_timeout,
                },
            ));
        }
    }

    if let Some(children) = &entry.children {
        parse_proxy_block(children, &mut cfg, ctx)?;
    }

    if cfg.upstreams.is_empty() {
        return Ok(None);
    }

    if let Some(conns_entries) = ctx
        .configuration
        .get_entries("proxy_concurrent_conns", true)
        .first()
    {
        if let Some(val) = conns_entries
            .args
            .first()
            .and_then(|v: &ServerConfigurationValue| v.as_number())
        {
            cfg.concurrent_conns = Some(val as usize);
        }
    }

    Ok(Some(cfg))
}

#[inline]
fn parse_proxy_block(
    block: &ServerConfigurationBlock,
    cfg: &mut ProxyConfig,
    ctx: &ferron_http::HttpContext,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    for (name, entries) in block.directives.iter() {
        match name.as_str() {
            "upstream" => {
                for entry in entries {
                    parse_upstream_entry(entry, cfg, ctx)?;
                }
            }
            "srv" => {
                for entry in entries {
                    upstream::parse_srv_entry(entry, cfg, ctx)?;
                }
            }
            "algorithm" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_str())
                {
                    cfg.algorithm = match val {
                        "random" => LoadBalancerAlgorithm::Random,
                        "round_robin" => LoadBalancerAlgorithm::RoundRobin,
                        "least_conn" => LoadBalancerAlgorithm::LeastConnections,
                        "two_random" => LoadBalancerAlgorithm::TwoRandomChoices,
                        "p2c_ewma" => LoadBalancerAlgorithm::P2cEwma,
                        _ => {
                            return Err(
                                format!("Unsupported load balancing algorithm: {val}").into()
                            )
                        }
                    };
                }
            }
            "circuit_breaker" => {
                if let Some(val) = entries.first().map(|e| e.get_flag()) {
                    cfg.circuit_breaker.enabled = val;
                    if val {
                        if let Some(children) = entries.first().and_then(|e| e.children.as_ref()) {
                            parse_circuit_breaker(children, &mut cfg.circuit_breaker)?;
                        }
                    }
                }
            }
            "retry_connection" => {
                if let Some(val) = entries.first().map(|e| e.get_flag()) {
                    cfg.retry_connection = val;
                }
            }
            "max_retries_per_upstream" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v: &ServerConfigurationValue| v.as_number())
                {
                    if val >= 0 {
                        cfg.max_retries_per_upstream = val as u32;
                    }
                }
            }
            "retry_budget" => {
                if let Some(val) = entries.first().map(|e| e.get_flag()) {
                    if val {
                        let mut retry_budget = RetryBudgetConfig::default();
                        if let Some(children) = entries.first().and_then(|e| e.children.as_ref()) {
                            parse_retry_budget(children, &mut retry_budget)?;
                        }
                        cfg.retry_budget = Some(retry_budget);
                    }
                }
            }
            "keepalive" => {
                if let Some(val) = entries.first().map(|e| e.get_flag()) {
                    cfg.keepalive = val;
                }
            }
            "http2" => {
                if let Some(val) = entries.first().map(|e| e.get_flag()) {
                    cfg.http2 = val;
                }
            }
            "http2_only" => {
                if let Some(val) = entries.first().map(|e| e.get_flag()) {
                    cfg.http2_only = val;
                }
            }
            "intercept_errors" => {
                if let Some(val) = entries.first().map(|e| e.get_flag()) {
                    cfg.intercept_errors = val;
                }
            }
            "no_verification" => {
                if let Some(val) = entries.first().map(|e| e.get_flag()) {
                    cfg.no_verification = val;
                }
            }
            "proxy_header" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_str())
                {
                    cfg.proxy_header = match val {
                        "v1" => Some(ProxyHeader::V1),
                        "v2" => Some(ProxyHeader::V2),
                        _ => return Err(format!("Invalid PROXY header version: {val}").into()),
                    };
                }
            }
            "request_header" => {
                for entry in entries {
                    parse_request_header_entry(entry, cfg, ctx)?;
                }
            }
            "proxy_concurrent_conns" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v: &ServerConfigurationValue| v.as_number())
                {
                    cfg.concurrent_conns = Some(val as usize);
                }
            }
            "affinity" => {
                if let Some(entry) = entries.first() {
                    if let Some(type_val) = entry.args.first().and_then(|v| v.as_str()) {
                        cfg.affinity = Some(parse_affinity_entry(type_val, entry, ctx)?);
                    }
                }
            }
            "metrics_resolved_ip" => {
                if let Some(val) = entries.first().map(|e| e.get_flag()) {
                    cfg.metrics_resolved_ip = val;
                }
            }
            _ => {}
        }
    }

    Ok(())
}
