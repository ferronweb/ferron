use ferron_core::config::validator::{
    entry_span, first_entry_span, ConfigurationValidationError, ConfigurationValidator,
};
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
    "throttle",
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
    "throttle",
];

/// Validator for `rate_limit` configuration blocks.
#[derive(Default)]
pub struct RateLimitValidator;

impl ConfigurationValidator for RateLimitValidator {
    fn validate_block(
        &self,
        config: &ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
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
    ) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
        let has_zones = block.directives.contains_key("zone");

        if has_zones {
            if let Some(entries) = block.directives.get("zone") {
                for entry in entries {
                    if entry.args.len() != 1 {
                        return Err(ConfigurationValidationError::from(
                            "Invalid `zone` — expected exactly one string argument (the zone name)",
                        )
                        .with_span(entry_span(entry)));
                    }
                    if entry.args.first().and_then(|v| v.as_str()).is_none() {
                        return Err(ConfigurationValidationError::from(
                            "Invalid `zone` — expected a string value",
                        )
                        .with_span(entry_span(entry)));
                    }
                    if entry.children.is_some() {
                        return Err(ConfigurationValidationError::from(
                            "Invalid `zone` — zone definition should not have a nested block",
                        )
                        .with_span(entry_span(entry)));
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
    ) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
        self.validate_rate_limit_block(block, ctx, HOST_RATE_LIMIT_DIRECTIVES)
    }

    /// Validate a `rate_limit` block with the given allowed directives.
    fn validate_rate_limit_block(
        &self,
        block: &ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
        allowed_directives: &[&str],
    ) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
        let mut sub = std::collections::HashSet::new();

        for directive_name in block.directives.keys() {
            if !allowed_directives.contains(&directive_name.as_str()) {
                return Err(ConfigurationValidationError::from(format!(
                    "Invalid `{directive_name}` — unknown directive in rate_limit block"
                ))
                .with_span(first_entry_span(block, directive_name)));
            }
        }

        let rate_entry = block.directives.get("rate");
        if rate_entry.is_none() {
            return Err(ConfigurationValidationError::from(
                "Invalid `rate_limit` — missing required `rate` directive",
            )
            .with_span(block.span.clone()));
        }

        for entry in rate_entry.into_iter().flatten() {
            self.validate_number_entry(entry, "rate", 1)?;
        }
        sub.insert("rate".to_string());

        if let Some(entries) = block.directives.get("burst") {
            sub.insert("burst".to_string());
            for entry in entries {
                self.validate_number_entry(entry, "burst", 0)?;
            }
        }

        if let Some(entries) = block.directives.get("key") {
            sub.insert("key".to_string());
            for entry in entries {
                if let Some(value) = entry.args.first() {
                    let key_str = value.as_str().ok_or_else(|| {
                        ConfigurationValidationError::from("Invalid `key` — must be a string value")
                            .with_span(entry_span(entry))
                    })?;
                    if KeyExtractor::from_str(key_str).is_none() {
                        return Err(ConfigurationValidationError::from(format!(
                            "Invalid `key` — must be one of: remote_address, uri, request.header.<name> (got '{key_str}')"
                        ))
                        .with_span(entry_span(entry)));
                    }
                }
            }
        }

        if let Some(entries) = block.directives.get("deny_status") {
            sub.insert("deny_status".to_string());
            for entry in entries {
                if let Some(value) = entry.args.first() {
                    let n = value.as_number().ok_or_else(|| {
                        ConfigurationValidationError::from(
                            "Invalid `deny_status` — must be an integer value",
                        )
                        .with_span(entry_span(entry))
                    })?;
                    if !(100..=599).contains(&n) {
                        return Err(ConfigurationValidationError::from(
                            "Invalid `deny_status` — must be a valid HTTP status code (100-599)",
                        )
                        .with_span(entry_span(entry)));
                    }
                }
            }
        }

        if let Some(entries) = block.directives.get("bucket_ttl") {
            sub.insert("bucket_ttl".to_string());
            for entry in entries {
                self.validate_number_entry(entry, "bucket_ttl", 1)?;
            }
        }

        if let Some(entries) = block.directives.get("max_buckets") {
            sub.insert("max_buckets".to_string());
            for entry in entries {
                self.validate_number_entry(entry, "max_buckets", 1)?;
            }
        }

        if let Some(entries) = block.directives.get("throttle") {
            sub.insert("throttle".to_string());
            for entry in entries {
                if !entry.args.is_empty()
                    && entry.args.first().and_then(|a| a.as_boolean()).is_none()
                {
                    return Err(ConfigurationValidationError::from(
                        "Invalid `throttle` — expected a boolean value",
                    )
                    .with_span(entry_span(entry)));
                }
            }
        }

        if let Some(entries) = block.directives.get("zone") {
            sub.insert("zone".to_string());
            for entry in entries {
                if entry.children.is_some() {
                    return Err(ConfigurationValidationError::from(
                        "Invalid `zone` — expected a string argument, not a block",
                    )
                    .with_span(entry_span(entry)));
                }
                if entry.args.len() != 1 {
                    return Err(ConfigurationValidationError::from(
                        "Invalid `zone` — expected exactly one string argument (the zone name)",
                    )
                    .with_span(entry_span(entry)));
                }
                if entry.args.first().and_then(|v| v.as_str()).is_none() {
                    return Err(ConfigurationValidationError::from(
                        "Invalid `zone` — expected a string value",
                    )
                    .with_span(entry_span(entry)));
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
    ) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
        let value = entry.args.first().ok_or_else(|| {
            ConfigurationValidationError::from(format!(
                "Invalid `{name}` — must be an integer value"
            ))
            .with_span(entry_span(entry))
        })?;
        let n = value.as_number().ok_or_else(|| {
            ConfigurationValidationError::from(format!(
                "Invalid `{name}` — must be an integer value"
            ))
            .with_span(entry_span(entry))
        })?;
        if n < min {
            return Err(ConfigurationValidationError::from(format!(
                "Invalid `{name}` — must be >= {min}"
            ))
            .with_span(entry_span(entry)));
        }
        Ok(())
    }
}
