//! Configuration validator for `rate_limit` directives.
//!
//! Validates that `rate_limit` blocks contain recognized directives
//! with valid value types.

use std::collections::HashSet;

use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::config::ServerConfigurationBlock;

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
        if !config.directives.contains_key("rate_limit") {
            return Ok(());
        }

        // Validate each rate_limit entry
        for entry in config.directives.get("rate_limit").into_iter().flatten() {
            if let Some(ref children) = entry.children {
                self.validate_rate_limit_block(children, used_directives)?;
            }
        }

        Ok(())
    }
}

impl RateLimitValidator {
    fn validate_rate_limit_block(
        &self,
        block: &ServerConfigurationBlock,
        used_directives: &mut HashSet<String>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Check all directives are recognized
        for directive_name in block.directives.keys() {
            if !RECOGNIZED_DIRECTIVES.contains(&directive_name.as_str()) {
                return Err(
                    format!("Unknown directive in rate_limit block: {directive_name}").into(),
                );
            }
        }

        // `rate` is required and must be a positive integer
        let rate_entry = block.directives.get("rate");
        if rate_entry.is_none() {
            return Err("rate_limit block is missing required 'rate' directive".into());
        }

        for entry in rate_entry.into_iter().flatten() {
            let value = entry
                .args
                .first()
                .ok_or("'rate' directive requires a value")?;
            let n = value.as_number().ok_or("'rate' must be an integer value")?;
            if n <= 0 {
                return Err("'rate' must be a positive integer".into());
            }
        }

        // Validate optional directives
        if let Some(entries) = block.directives.get("burst") {
            for entry in entries {
                if let Some(value) = entry.args.first() {
                    if value.as_number().is_none() {
                        return Err("'burst' must be an integer value".into());
                    }
                }
            }
        }

        if let Some(entries) = block.directives.get("key") {
            for entry in entries {
                if let Some(value) = entry.args.first() {
                    let key_str = value.as_str().ok_or("'key' must be a string value")?;
                    if KeyExtractor::from_str(key_str).is_none() {
                        return Err(format!(
                            "'key' must be one of: remote_address, uri, request.header.<name> (got '{key_str}')"
                        )
                        .into());
                    }
                }
            }
        }

        if let Some(entries) = block.directives.get("window") {
            for entry in entries {
                if let Some(value) = entry.args.first() {
                    let n = value
                        .as_number()
                        .ok_or("'window' must be an integer value (seconds)")?;
                    if n <= 0 {
                        return Err("'window' must be a positive integer".into());
                    }
                }
            }
        }

        if let Some(entries) = block.directives.get("deny_status") {
            for entry in entries {
                if let Some(value) = entry.args.first() {
                    let n = value
                        .as_number()
                        .ok_or("'deny_status' must be an integer value")?;
                    if !(100..=599).contains(&n) {
                        return Err(
                            "'deny_status' must be a valid HTTP status code (100-599)".into()
                        );
                    }
                }
            }
        }

        if let Some(entries) = block.directives.get("bucket_ttl") {
            for entry in entries {
                if let Some(value) = entry.args.first() {
                    let n = value
                        .as_number()
                        .ok_or("'bucket_ttl' must be an integer value (seconds)")?;
                    if n <= 0 {
                        return Err("'bucket_ttl' must be a positive integer".into());
                    }
                }
            }
        }

        if let Some(entries) = block.directives.get("max_buckets") {
            for entry in entries {
                if let Some(value) = entry.args.first() {
                    let n = value
                        .as_number()
                        .ok_or("'max_buckets' must be an integer value")?;
                    if n <= 0 {
                        return Err("'max_buckets' must be a positive integer".into());
                    }
                }
            }
        }

        // Mark rate_limit itself as used
        used_directives.insert("rate_limit".to_string());

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
        assert!(used.contains("rate"));
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
