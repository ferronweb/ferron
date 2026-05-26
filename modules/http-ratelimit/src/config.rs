//! Configuration parsing for `rate_limit` blocks.
//!
//! Parses `rate_limit { ... }` directive entries from the layered configuration
//! into typed `RateLimitConfig` structures.

use ferron_core::config::ServerConfigurationBlock;

use crate::key_extractor::KeyExtractor;

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
