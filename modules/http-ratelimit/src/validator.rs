use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::config::{ServerConfigurationBlock, ServerConfigurationDirectiveEntry};

use crate::key_extractor::KeyExtractor;

/// Directives allowed inside a global `rate_limit { ... }` block (no zone definitions).
const GLOBAL_RATE_LIMIT_DIRECTIVES: &[&str] = &[
    "rate",
    "burst",
    "key",
    "deny_status",
    "bucket_ttl",
    "max_buckets",
];

/// Directives allowed inside a host-level `rate_limit { ... }` block.
const HOST_RATE_LIMIT_DIRECTIVES: &[&str] = &[
    "rate",
    "burst",
    "key",
    "deny_status",
    "bucket_ttl",
    "max_buckets",
    "zone",
];

/// Validator for `rate_limit` configuration blocks.
#[derive(Default)]
pub struct RateLimitValidator;

impl ConfigurationValidator for RateLimitValidator {
    fn validate_block(
        &self,
        config: &ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let is_global = ctx.is_global;

        if let Some(entries) = config.directives.get("rate_limit") {
            ctx.used_directives.insert("rate_limit".to_string());
            for entry in entries {
                if let Some(ref children) = entry.children {
                    if is_global {
                        self.validate_global_rate_limit_block(children, ctx)?;
                    } else {
                        self.validate_host_rate_limit_block(children, ctx)?;
                    }
                }
            }
        }

        Ok(())
    }
}

impl RateLimitValidator {
    /// Validate a global `rate_limit` block.
    ///
    /// Global blocks can either:
    /// 1. Define rate limit rules (all hosts share a global zone)
    /// 2. Define named zones (hosts reference them with `zone "name"`)
    fn validate_global_rate_limit_block(
        &self,
        block: &ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Check if this block defines named zones
        let has_zones = block.directives.contains_key("zone");

        if has_zones {
            // Zone definition block: validate zone directives
            if let Some(entries) = block.directives.get("zone") {
                for entry in entries {
                    // Zone must have exactly one string argument
                    if entry.args.len() != 1 {
                        return Err(
                            "Invalid `zone` — expected exactly one string argument (the zone name)"
                                .into(),
                        );
                    }
                    if entry.args.first().and_then(|v| v.as_str()).is_none() {
                        return Err("Invalid `zone` — expected a string value".into());
                    }
                    // Zone definition should not have a block
                    if entry.children.is_some() {
                        return Err(
                            "Invalid `zone` — zone definition should not have a nested block"
                                .into(),
                        );
                    }
                }
            }
        } else {
            // Global rate limit rules: validate like a host block
            self.validate_rate_limit_block(block, ctx, GLOBAL_RATE_LIMIT_DIRECTIVES)?;
        }

        Ok(())
    }

    /// Validate a host-level `rate_limit` block.
    fn validate_host_rate_limit_block(
        &self,
        block: &ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.validate_rate_limit_block(block, ctx, HOST_RATE_LIMIT_DIRECTIVES)
    }

    /// Validate a `rate_limit` block with the given allowed directives.
    fn validate_rate_limit_block(
        &self,
        block: &ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
        allowed_directives: &[&str],
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut sub = std::collections::HashSet::new();

        // Check all directives are recognized
        for directive_name in block.directives.keys() {
            if !allowed_directives.contains(&directive_name.as_str()) {
                return Err(format!(
                    "Invalid `{directive_name}` — unknown directive in rate_limit block"
                )
                .into());
            }
        }

        // Validate `rate` — required, must be a positive integer
        let rate_entry = block.directives.get("rate");
        if rate_entry.is_none() {
            return Err("Invalid `rate_limit` — missing required `rate` directive".into());
        }

        for entry in rate_entry.into_iter().flatten() {
            self.validate_number_entry(entry, "rate", 1)?;
        }
        sub.insert("rate".to_string());

        // Validate `burst` — optional, must be a non-negative integer
        if let Some(entries) = block.directives.get("burst") {
            sub.insert("burst".to_string());
            for entry in entries {
                self.validate_number_entry(entry, "burst", 0)?;
            }
        }

        // Validate `key` — optional, must be a valid key extractor string
        if let Some(entries) = block.directives.get("key") {
            sub.insert("key".to_string());
            for entry in entries {
                if let Some(value) = entry.args.first() {
                    let key_str = value
                        .as_str()
                        .ok_or("Invalid `key` — must be a string value")?;
                    if KeyExtractor::from_str(key_str).is_none() {
                        return Err(format!(
                            "Invalid `key` — must be one of: remote_address, uri, request.header.<name> (got '{key_str}')"
                        )
                        .into());
                    }
                }
            }
        }

        // Validate `deny_status` — optional, must be a valid HTTP status code
        if let Some(entries) = block.directives.get("deny_status") {
            sub.insert("deny_status".to_string());
            for entry in entries {
                if let Some(value) = entry.args.first() {
                    let n = value
                        .as_number()
                        .ok_or("Invalid `deny_status` — must be an integer value")?;
                    if !(100..=599).contains(&n) {
                        return Err(
                            "Invalid `deny_status` — must be a valid HTTP status code (100-599)"
                                .into(),
                        );
                    }
                }
            }
        }

        // Validate `bucket_ttl` — optional, must be a positive integer
        if let Some(entries) = block.directives.get("bucket_ttl") {
            sub.insert("bucket_ttl".to_string());
            for entry in entries {
                self.validate_number_entry(entry, "bucket_ttl", 1)?;
            }
        }

        // Validate `max_buckets` — optional, must be a positive integer
        if let Some(entries) = block.directives.get("max_buckets") {
            sub.insert("max_buckets".to_string());
            for entry in entries {
                self.validate_number_entry(entry, "max_buckets", 1)?;
            }
        }

        // Validate `zone` — optional, must be a string argument (host-level only)
        if let Some(entries) = block.directives.get("zone") {
            sub.insert("zone".to_string());
            for entry in entries {
                if entry.children.is_some() {
                    return Err("Invalid `zone` — expected a string argument, not a block".into());
                }
                if entry.args.len() != 1 {
                    return Err(
                        "Invalid `zone` — expected exactly one string argument (the zone name)"
                            .into(),
                    );
                }
                if entry.args.first().and_then(|v| v.as_str()).is_none() {
                    return Err("Invalid `zone` — expected a string value".into());
                }
            }
        }

        ferron_core::check_unused_subdirectives!(
            block,
            sub,
            &mut ctx.diagnostics,
            ctx.scope.clone()
        );
        Ok(())
    }

    /// Validate that an entry has exactly one number argument >= min_value.
    fn validate_number_entry(
        &self,
        entry: &ServerConfigurationDirectiveEntry,
        name: &str,
        min: i64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let value = entry
            .args
            .first()
            .ok_or(format!("Invalid `{name}` — must be an integer value"))?;
        let n = value
            .as_number()
            .ok_or(format!("Invalid `{name}` — must be an integer value"))?;
        if n < min {
            return Err(format!("Invalid `{name}` — must be >= {min}").into());
        }
        Ok(())
    }
}
