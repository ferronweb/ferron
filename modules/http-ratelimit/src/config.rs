//! Configuration parsing for `rate_limit` blocks.
//!
//! Parses `rate_limit { ... }` directive entries from the layered configuration
//! into typed `RateLimitConfig` structures.

use ferron_core::config::ServerConfigurationBlock;

use crate::key_extractor::KeyExtractor;

/// Identifies a rate limit zone — a sharing scope for token bucket registries.
///
/// Zones allow multiple hostnames to share the same rate limit registries,
/// or to isolate them into separate per-host registries.
#[derive(Debug, Clone, Hash, Eq, PartialEq, Ord, PartialOrd)]
pub enum RateLimitZoneId {
    /// Shared global zone — all hosts without explicit zones share registries.
    Global,
    /// Named zone — hosts explicitly referencing the same name share registries.
    Named(String),
    /// Per-host zone — host has its own isolated registries.
    Host(String),
}

/// A single rate limit rule parsed from configuration.
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// Sustained requests per second.
    pub rate: u64,
    /// Extra tokens above `rate` (bucket capacity = `rate + burst`).
    pub burst: u64,
    /// Strategy for extracting the rate limit key.
    pub key: KeyExtractor,
    /// HTTP status code to return when rate is exceeded.
    pub deny_status: u16,
    /// TTL for evicting stale buckets (seconds).
    pub bucket_ttl_secs: u64,
    /// Maximum number of buckets per rule (prevents unbounded memory growth).
    pub max_buckets: usize,
}

impl RateLimitZoneId {
    /// Return a stable string label for use in metric attributes.
    pub fn label(&self) -> &str {
        match self {
            RateLimitZoneId::Global => "global",
            RateLimitZoneId::Named(name) => name.as_str(),
            RateLimitZoneId::Host(host) => host.as_str(),
        }
    }
}

impl RateLimitConfig {
    /// Default values for rate limit configuration.
    pub const DEFAULT_BURST: u64 = 0;
    pub const DEFAULT_DENY_STATUS: u16 = 429;
    pub const DEFAULT_BUCKET_TTL_SECS: u64 = 600; // 10 minutes
    pub const DEFAULT_MAX_BUCKETS: usize = 100_000;
}

/// Parse all `rate_limit` directives from the layered configuration.
///
/// Each `rate_limit { ... }` block becomes a `RateLimitConfig`.
/// If no `rate_limit` blocks are present, returns an empty vec.
pub fn parse_rate_limit_config(
    config: &ferron_core::config::layer::LayeredConfiguration,
) -> Vec<RateLimitConfig> {
    let mut rules = Vec::new();

    // rate_limit can appear as:
    // 1. A block: `rate_limit { rate 100; burst 50 }`
    // 2. Multiple blocks (repeatable, like `upstream`)
    let entries = config.get_entries("rate_limit", true);

    for entry in entries {
        // If the entry has children (block form), parse from children
        if let Some(children) = &entry.children {
            if let Some(rule) = parse_rate_limit_block(children) {
                rules.push(rule);
            }
        }
        // If no children, the directive might have args (shorthand form),
        // but we only support block form for now.
    }

    rules
}

/// Parse a single `rate_limit { ... }` block into a `RateLimitConfig`.
fn parse_rate_limit_block(block: &ServerConfigurationBlock) -> Option<RateLimitConfig> {
    // `rate` is required
    let rate = block
        .get_value("rate")
        .and_then(|v| v.as_number())
        .filter(|&n| n > 0)? as u64;

    // Parse optional directives with defaults
    let burst = block
        .get_value("burst")
        .and_then(|v| v.as_number())
        .filter(|&n| n >= 0)
        .unwrap_or(RateLimitConfig::DEFAULT_BURST as i64) as u64;

    let key_str = block
        .get_value("key")
        .and_then(|v| v.as_str())
        .unwrap_or("remote_address");

    let key = KeyExtractor::from_str(key_str).unwrap_or(KeyExtractor::RemoteAddress);

    let deny_status = block
        .get_value("deny_status")
        .and_then(|v| v.as_number())
        .filter(|&n| (100..=599).contains(&n))
        .unwrap_or(RateLimitConfig::DEFAULT_DENY_STATUS as i64) as u16;

    let bucket_ttl_secs = block
        .get_value("bucket_ttl")
        .and_then(|v| v.as_duration())
        .map_or(RateLimitConfig::DEFAULT_BUCKET_TTL_SECS, |d| d.as_secs());

    let max_buckets = block
        .get_value("max_buckets")
        .and_then(|v| v.as_number())
        .filter(|&n| n > 0)
        .unwrap_or(RateLimitConfig::DEFAULT_MAX_BUCKETS as i64) as usize;

    Some(RateLimitConfig {
        rate,
        burst,
        key,
        deny_status,
        bucket_ttl_secs,
        max_buckets,
    })
}

/// Parse the `zone` directive from a host-level `rate_limit` block.
///
/// Returns `Some(name)` if the block contains `zone "name"`, or `None` otherwise.
pub fn parse_zone_name(block: &ServerConfigurationBlock) -> Option<String> {
    block
        .get_value("zone")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Check if a global `rate_limit` block defines named zones (has `zone` directives).
///
/// Returns `true` if any global `rate_limit` block contains a `zone` directive.
#[allow(dead_code)]
pub fn has_global_zone_definitions(
    configuration: &ferron_core::config::layer::LayeredConfiguration,
) -> bool {
    if let Some(global_layer) = configuration.layers.first() {
        if let Some(entries) = global_layer.directives.get("rate_limit") {
            for entry in entries {
                if let Some(children) = &entry.children {
                    if children.directives.contains_key("zone") {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Check if a global `rate_limit` block exists without zone definitions.
///
/// Returns `true` if the global (first) layer has a `rate_limit` block
/// that does NOT contain any `zone` directives — meaning all hosts share
/// a global zone by default.
pub fn has_global_zone(configuration: &ferron_core::config::layer::LayeredConfiguration) -> bool {
    // Only check the global layer (first layer), not host layers
    if let Some(global_layer) = configuration.layers.first() {
        if let Some(entries) = global_layer.directives.get("rate_limit") {
            for entry in entries {
                if let Some(children) = &entry.children {
                    if !children.directives.contains_key("zone") {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Check if a host has its own `rate_limit` block (not inherited from global).
///
/// Returns `true` if the host-level configuration has a `rate_limit` block
/// without a `zone` directive, meaning the host opts out of the global zone.
pub fn has_own_rate_limit_block(
    configuration: &ferron_core::config::layer::LayeredConfiguration,
) -> bool {
    // get_entries with inherit=false returns entries from the most recent layer
    // that has the directive. If the host has its own rate_limit block, this
    // returns the host's entries. If not, it falls back to global entries.
    //
    // To distinguish, we check if the LAST layer (host) has rate_limit entries.
    if let Some(host_layer) = configuration.layers.last() {
        if let Some(entries) = host_layer.directives.get("rate_limit") {
            for entry in entries {
                if let Some(children) = &entry.children {
                    // Host has a rate_limit block. If it doesn't have a zone
                    // directive, it's opting out of the global zone.
                    if !children.directives.contains_key("zone") {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Resolve the rate limit zone ID for a request based on configuration.
///
/// Resolution order:
/// 1. Explicit `zone "name"` in host `rate_limit` block → `Named(name)`
/// 2. Host has its own `rate_limit` block (without zone) → `Host(hostname)` (opt-out)
/// 3. Global `rate_limit` block without zone definitions → `Global`
/// 4. Fallback → `Host(hostname)`
pub fn resolve_zone_id(
    configuration: &ferron_core::config::layer::LayeredConfiguration,
    hostname: &Option<String>,
) -> RateLimitZoneId {
    // Check for explicit zone reference in host-level rate_limit blocks
    for entry in configuration.get_entries("rate_limit", false) {
        if let Some(children) = &entry.children {
            if let Some(name) = parse_zone_name(children) {
                return RateLimitZoneId::Named(name);
            }
        }
    }

    // Check if host has its own rate_limit block (without zone) → per-host zone
    if has_own_rate_limit_block(configuration) {
        return RateLimitZoneId::Host(hostname.clone().unwrap_or_else(|| "_default".to_string()));
    }

    if has_global_zone(configuration) {
        RateLimitZoneId::Global
    } else {
        RateLimitZoneId::Host(hostname.clone().unwrap_or_else(|| "_default".to_string()))
    }
}
