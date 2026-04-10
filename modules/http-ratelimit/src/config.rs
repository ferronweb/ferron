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
    /// Time window for rate calculation (used for Retry-After header estimation).
    #[allow(dead_code)]
    pub window_secs: u64,
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
    pub const DEFAULT_WINDOW_SECS: u64 = 60;
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

    let window_secs = block
        .get_value("window")
        .and_then(|v| v.as_number())
        .filter(|&n| n > 0)
        .unwrap_or(RateLimitConfig::DEFAULT_WINDOW_SECS as i64) as u64;

    let deny_status = block
        .get_value("deny_status")
        .and_then(|v| v.as_number())
        .filter(|&n| (100..=599).contains(&n))
        .unwrap_or(RateLimitConfig::DEFAULT_DENY_STATUS as i64) as u16;

    let bucket_ttl_secs = block
        .get_value("bucket_ttl")
        .and_then(|v| v.as_number())
        .filter(|&n| n > 0)
        .unwrap_or(RateLimitConfig::DEFAULT_BUCKET_TTL_SECS as i64)
        as u64;

    let max_buckets = block
        .get_value("max_buckets")
        .and_then(|v| v.as_number())
        .filter(|&n| n > 0)
        .unwrap_or(RateLimitConfig::DEFAULT_MAX_BUCKETS as i64) as usize;

    Some(RateLimitConfig {
        rate,
        burst,
        key,
        window_secs,
        deny_status,
        bucket_ttl_secs,
        max_buckets,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferron_core::config::layer::LayeredConfiguration;
    use ferron_core::config::{
        ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationValue,
    };
    use std::collections::HashMap as StdHashMap;
    use std::sync::Arc;

    fn make_block(
        directives: Vec<(&str, Vec<ServerConfigurationValue>)>,
        children: Option<ServerConfigurationBlock>,
    ) -> ServerConfigurationBlock {
        let mut d = StdHashMap::new();
        for (name, args) in directives {
            d.insert(
                name.to_string(),
                vec![ServerConfigurationDirectiveEntry {
                    args,
                    children: children.clone(),
                    span: None,
                }],
            );
        }
        ServerConfigurationBlock {
            directives: Arc::new(d),
            matchers: StdHashMap::new(),
            span: None,
        }
    }

    fn make_value_string(s: &str) -> ServerConfigurationValue {
        ServerConfigurationValue::String(s.to_string(), None)
    }

    fn make_value_number(n: i64) -> ServerConfigurationValue {
        ServerConfigurationValue::Number(n, None)
    }

    #[test]
    fn parses_required_rate_only() {
        let block = make_block(vec![("rate", vec![make_value_number(100)])], None);
        let config = parse_rate_limit_block(&block).unwrap();
        assert_eq!(config.rate, 100);
        assert_eq!(config.burst, 0);
        assert!(matches!(config.key, KeyExtractor::RemoteAddress));
        assert_eq!(config.window_secs, 60);
        assert_eq!(config.deny_status, 429);
    }

    #[test]
    fn parses_all_options() {
        let block = make_block(
            vec![
                ("rate", vec![make_value_number(50)]),
                ("burst", vec![make_value_number(20)]),
                ("key", vec![make_value_string("uri")]),
                ("window", vec![make_value_number(30)]),
                ("deny_status", vec![make_value_number(503)]),
                ("bucket_ttl", vec![make_value_number(300)]),
                ("max_buckets", vec![make_value_number(5000)]),
            ],
            None,
        );
        let config = parse_rate_limit_block(&block).unwrap();
        assert_eq!(config.rate, 50);
        assert_eq!(config.burst, 20);
        assert!(matches!(config.key, KeyExtractor::Uri));
        assert_eq!(config.window_secs, 30);
        assert_eq!(config.deny_status, 503);
        assert_eq!(config.bucket_ttl_secs, 300);
        assert_eq!(config.max_buckets, 5000);
    }

    #[test]
    fn rejects_missing_rate() {
        let block = make_block(vec![("burst", vec![make_value_number(10)])], None);
        assert!(parse_rate_limit_block(&block).is_none());
    }

    #[test]
    fn rejects_zero_rate() {
        let block = make_block(vec![("rate", vec![make_value_number(0)])], None);
        assert!(parse_rate_limit_block(&block).is_none());
    }

    #[test]
    fn parses_header_key_extractor() {
        let block = make_block(
            vec![
                ("rate", vec![make_value_number(10)]),
                ("key", vec![make_value_string("request.header.X-Api-Key")]),
            ],
            None,
        );
        let config = parse_rate_limit_block(&block).unwrap();
        assert!(matches!(config.key, KeyExtractor::Header(ref h) if h == "X-Api-Key"));
    }

    #[test]
    fn layered_config_merges_multiple_rate_limits() {
        // Simulate two rate_limit blocks in the same layer
        let block1 = make_block(vec![("rate", vec![make_value_number(100)])], None);
        let block2 = make_block(vec![("rate", vec![make_value_number(10)])], None);

        let mut config = LayeredConfiguration::default();
        // We need to simulate multiple rate_limit entries.
        // In practice, the config resolver handles this. Here we just test
        // that get_entries finds multiple entries.
        let mut directives = StdHashMap::new();
        directives.insert(
            "rate_limit".to_string(),
            vec![
                ServerConfigurationDirectiveEntry {
                    args: vec![],
                    children: Some(block1.clone()),
                    span: None,
                },
                ServerConfigurationDirectiveEntry {
                    args: vec![],
                    children: Some(block2.clone()),
                    span: None,
                },
            ],
        );
        config.layers.push(Arc::new(ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: StdHashMap::new(),
            span: None,
        }));

        let rules = parse_rate_limit_config(&config);
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].rate, 100);
        assert_eq!(rules[1].rate, 10);
    }
}
