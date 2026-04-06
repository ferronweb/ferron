//! Configuration validator for `rate_limit` directives.
//!
//! Validates that `rate_limit` blocks contain recognized directives
//! with valid value types.

use std::collections::HashSet;

use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::config::{ServerConfigurationBlock, ServerConfigurationDirectiveEntry};

use crate::key_extractor::KeyExtractor;

/// Recognized directives inside a `rate_limit { ... }` block.
const RECOGNIZED_DIRECTIVES: &[&str] = &[
    "rate",
    "burst",
    "key",
    "window",
    "deny_status",
    "bucket_ttl",
    "max_buckets",
];

/// Validator for `rate_limit` configuration blocks.
#[derive(Default)]
pub struct RateLimitValidator;

impl ConfigurationValidator for RateLimitValidator {
    fn validate_block(
        &self,
        config: &ServerConfigurationBlock,
        used_directives: &mut HashSet<String>,
        _is_global: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Check if this block contains a `rate_limit` directive
        if let Some(entries) = config.directives.get("rate_limit") {
            used_directives.insert("rate_limit".to_string());
            for entry in entries {
                if let Some(ref children) = entry.children {
                    self.validate_rate_limit_block(children)?;
                }
            }
        }

        Ok(())
    }
}

impl RateLimitValidator {
    fn validate_rate_limit_block(
        &self,
        block: &ServerConfigurationBlock,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Check all directives are recognized
        for directive_name in block.directives.keys() {
            if !RECOGNIZED_DIRECTIVES.contains(&directive_name.as_str()) {
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

        // Validate `burst` — optional, must be a non-negative integer
        if let Some(entries) = block.directives.get("burst") {
            for entry in entries {
                self.validate_number_entry(entry, "burst", 0)?;
            }
        }

        // Validate `key` — optional, must be a valid key extractor string
        if let Some(entries) = block.directives.get("key") {
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

        // Validate `window` — optional, must be a positive integer
        if let Some(entries) = block.directives.get("window") {
            for entry in entries {
                self.validate_number_entry(entry, "window", 1)?;
            }
        }

        // Validate `deny_status` — optional, must be a valid HTTP status code
        if let Some(entries) = block.directives.get("deny_status") {
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
            for entry in entries {
                self.validate_number_entry(entry, "bucket_ttl", 1)?;
            }
        }

        // Validate `max_buckets` — optional, must be a positive integer
        if let Some(entries) = block.directives.get("max_buckets") {
            for entry in entries {
                self.validate_number_entry(entry, "max_buckets", 1)?;
            }
        }

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

#[cfg(test)]
mod tests {
    use super::*;
    use ferron_core::config::{ServerConfigurationDirectiveEntry, ServerConfigurationValue};
    use std::collections::HashMap as StdHashMap;
    use std::sync::Arc;

    fn make_value_number(n: i64) -> ServerConfigurationValue {
        ServerConfigurationValue::Number(n, None)
    }

    fn make_value_string(s: &str) -> ServerConfigurationValue {
        ServerConfigurationValue::String(s.to_string(), None)
    }

    fn make_rate_limit_block(
        directives: Vec<(&str, Vec<ServerConfigurationValue>)>,
    ) -> ServerConfigurationBlock {
        let mut d = StdHashMap::new();
        for (name, args) in directives {
            d.insert(
                name.to_string(),
                vec![ServerConfigurationDirectiveEntry {
                    args,
                    children: None,
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

    fn make_parent_with_rate_limit(children: ServerConfigurationBlock) -> ServerConfigurationBlock {
        let mut d = StdHashMap::new();
        d.insert(
            "rate_limit".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![],
                children: Some(children),
                span: None,
            }],
        );
        ServerConfigurationBlock {
            directives: Arc::new(d),
            matchers: StdHashMap::new(),
            span: None,
        }
    }

    #[test]
    fn valid_rate_limit_block() {
        let inner = make_rate_limit_block(vec![("rate", vec![make_value_number(100)])]);
        let parent = make_parent_with_rate_limit(inner);
        let mut used = HashSet::new();
        let validator = RateLimitValidator;
        assert!(validator.validate_block(&parent, &mut used, false).is_ok());
        assert!(used.contains("rate_limit"));
    }

    #[test]
    fn rejects_missing_rate() {
        let inner = make_rate_limit_block(vec![("burst", vec![make_value_number(10)])]);
        let parent = make_parent_with_rate_limit(inner);
        let mut used = HashSet::new();
        let validator = RateLimitValidator;
        assert!(validator.validate_block(&parent, &mut used, false).is_err());
    }

    #[test]
    fn rejects_unknown_directive() {
        let inner = make_rate_limit_block(vec![
            ("rate", vec![make_value_number(100)]),
            ("foo_bar", vec![make_value_number(1)]),
        ]);
        let parent = make_parent_with_rate_limit(inner);
        let mut used = HashSet::new();
        let validator = RateLimitValidator;
        assert!(validator.validate_block(&parent, &mut used, false).is_err());
    }

    #[test]
    fn rejects_invalid_key() {
        let inner = make_rate_limit_block(vec![
            ("rate", vec![make_value_number(100)]),
            ("key", vec![make_value_string("cookie")]),
        ]);
        let parent = make_parent_with_rate_limit(inner);
        let mut used = HashSet::new();
        let validator = RateLimitValidator;
        let err = validator
            .validate_block(&parent, &mut used, false)
            .unwrap_err();
        assert!(err.to_string().contains("key"));
    }

    #[test]
    fn rejects_negative_rate() {
        let inner = make_rate_limit_block(vec![("rate", vec![make_value_number(-1)])]);
        let parent = make_parent_with_rate_limit(inner);
        let mut used = HashSet::new();
        let validator = RateLimitValidator;
        assert!(validator.validate_block(&parent, &mut used, false).is_err());
    }

    #[test]
    fn rejects_invalid_deny_status() {
        let inner = make_rate_limit_block(vec![
            ("rate", vec![make_value_number(100)]),
            ("deny_status", vec![make_value_number(999)]),
        ]);
        let parent = make_parent_with_rate_limit(inner);
        let mut used = HashSet::new();
        let validator = RateLimitValidator;
        assert!(validator.validate_block(&parent, &mut used, false).is_err());
    }

    #[test]
    fn skips_block_without_rate_limit() {
        let block = ServerConfigurationBlock {
            directives: Arc::new(StdHashMap::new()),
            matchers: StdHashMap::new(),
            span: None,
        };
        let mut used = HashSet::new();
        let validator = RateLimitValidator;
        assert!(validator.validate_block(&block, &mut used, false).is_ok());
    }
}
