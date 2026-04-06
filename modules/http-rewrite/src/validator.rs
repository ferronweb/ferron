//! Configuration validator for `rewrite` and `rewrite_log` directives.
//!
//! Validates that `rewrite` entries contain recognized arguments and options
//! with valid value types.

use std::collections::HashSet;

use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::config::{ServerConfigurationBlock, ServerConfigurationValue};

/// Recognized directives inside a `rewrite { ... }` block.
const RECOGNIZED_OPTIONS: &[&str] = &["last", "directory", "file", "allow_double_slashes"];

/// Validator for `rewrite` and `rewrite_log` configuration.
#[derive(Default)]
pub struct RewriteValidator;

impl ConfigurationValidator for RewriteValidator {
    fn validate_block(
        &self,
        config: &ServerConfigurationBlock,
        used_directives: &mut HashSet<String>,
        _is_global: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Validate `rewrite` entries
        if let Some(entries) = config.directives.get("rewrite") {
            for entry in entries {
                self.validate_rewrite_entry(entry)?;
            }
            used_directives.insert("rewrite".to_string());
        }

        // Validate `rewrite_log` entries
        if let Some(entries) = config.directives.get("rewrite_log") {
            for entry in entries {
                self.validate_rewrite_log_entry(entry)?;
            }
            used_directives.insert("rewrite_log".to_string());
        }

        Ok(())
    }
}

impl RewriteValidator {
    fn validate_rewrite_entry(
        &self,
        entry: &ferron_core::config::ServerConfigurationDirectiveEntry,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Must have exactly 2 positional arguments
        if entry.args.len() != 2 {
            return Err(format!(
                "The `rewrite` directive must have exactly two values (regex and replacement), got {}",
                entry.args.len()
            )
            .into());
        }

        // First arg must be a string (regex)
        if !matches!(
            &entry.args[0],
            ServerConfigurationValue::String(_, _)
                | ServerConfigurationValue::InterpolatedString(_, _)
        ) {
            return Err("The URL rewrite regular expression must be a string".into());
        }

        // Second arg must be a string (replacement)
        if !matches!(
            &entry.args[1],
            ServerConfigurationValue::String(_, _)
                | ServerConfigurationValue::InterpolatedString(_, _)
        ) {
            return Err("The URL rewrite replacement must be a string".into());
        }

        // Validate optional block
        if let Some(ref children) = entry.children {
            for (key, nested_entries) in children.directives.iter() {
                if !RECOGNIZED_OPTIONS.contains(&key.as_str()) {
                    return Err(format!(
                        "Unknown directive in rewrite block: {key}. Recognized options: {}",
                        RECOGNIZED_OPTIONS.join(", ")
                    )
                    .into());
                }
                for nested_entry in nested_entries {
                    if nested_entry.args.len() > 1 {
                        return Err(format!(
                            "The `{key}` option in a rewrite block must have exactly one value"
                        )
                        .into());
                    }
                    if nested_entry.args.len() > 0 {
                        match &nested_entry.args[0] {
                            ServerConfigurationValue::Boolean(_, _) => {}
                            _ => {
                                return Err(format!(
                                    "The `{key}` option in a rewrite block must be a boolean"
                                )
                                .into());
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn validate_rewrite_log_entry(
        &self,
        entry: &ferron_core::config::ServerConfigurationDirectiveEntry,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if entry.args.len() > 1 {
            return Err(format!(
                "The `rewrite_log` directive must have zero or one value, got {}",
                entry.args.len()
            )
            .into());
        }

        if entry.args.len() > 0 {
            match &entry.args[0] {
                ServerConfigurationValue::Boolean(_, _) => Ok(()),
                _ => Err("The `rewrite_log` directive must be a boolean".into()),
            }
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferron_core::config::ServerConfigurationDirectiveEntry;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn make_value_string(s: &str) -> ServerConfigurationValue {
        ServerConfigurationValue::String(s.to_string(), None)
    }

    fn make_value_bool(b: bool) -> ServerConfigurationValue {
        ServerConfigurationValue::Boolean(b, None)
    }

    fn make_rewrite_entry(
        args: Vec<ServerConfigurationValue>,
        children: Option<ServerConfigurationBlock>,
    ) -> ServerConfigurationDirectiveEntry {
        ServerConfigurationDirectiveEntry {
            args,
            children,
            span: None,
        }
    }

    fn make_options_block(options: &[(&str, bool)]) -> ServerConfigurationBlock {
        let mut directives = HashMap::new();
        for (name, value) in options {
            directives.insert(
                name.to_string(),
                vec![make_rewrite_entry(vec![make_value_bool(*value)], None)],
            );
        }
        ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: HashMap::new(),
            span: None,
        }
    }

    fn make_parent_with_entries(
        rewrite_entries: Vec<ServerConfigurationDirectiveEntry>,
        rewrite_log_entries: Vec<ServerConfigurationDirectiveEntry>,
    ) -> ServerConfigurationBlock {
        let mut directives = HashMap::new();
        if !rewrite_entries.is_empty() {
            directives.insert("rewrite".to_string(), rewrite_entries);
        }
        if !rewrite_log_entries.is_empty() {
            directives.insert("rewrite_log".to_string(), rewrite_log_entries);
        }
        ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: HashMap::new(),
            span: None,
        }
    }

    #[test]
    fn valid_simple_rewrite() {
        let entry = make_rewrite_entry(
            vec![
                make_value_string("^/old/(.*)"),
                make_value_string("/new/$1"),
            ],
            None,
        );
        let parent = make_parent_with_entries(vec![entry], vec![]);
        let mut used = HashSet::new();
        let validator = RewriteValidator;
        assert!(validator.validate_block(&parent, &mut used, false).is_ok());
    }

    #[test]
    fn valid_rewrite_with_options() {
        let opts = make_options_block(&[("last", true)]);
        let entry = make_rewrite_entry(
            vec![make_value_string("^/api/(.*)"), make_value_string("/v2/$1")],
            Some(opts),
        );
        let parent = make_parent_with_entries(vec![entry], vec![]);
        let mut used = HashSet::new();
        let validator = RewriteValidator;
        assert!(validator.validate_block(&parent, &mut used, false).is_ok());
    }

    #[test]
    fn rejects_missing_args() {
        let entry = make_rewrite_entry(vec![make_value_string("^/old/(.*)")], None);
        let parent = make_parent_with_entries(vec![entry], vec![]);
        let mut used = HashSet::new();
        let validator = RewriteValidator;
        assert!(validator.validate_block(&parent, &mut used, false).is_err());
    }

    #[test]
    fn rejects_too_many_args() {
        let entry = make_rewrite_entry(
            vec![
                make_value_string("^/old/(.*)"),
                make_value_string("/new/$1"),
                make_value_string("extra"),
            ],
            None,
        );
        let parent = make_parent_with_entries(vec![entry], vec![]);
        let mut used = HashSet::new();
        let validator = RewriteValidator;
        assert!(validator.validate_block(&parent, &mut used, false).is_err());
    }

    #[test]
    fn rejects_unknown_option_in_block() {
        let opts = make_options_block(&[("unknown_option", true)]);
        let entry = make_rewrite_entry(
            vec![
                make_value_string("^/old/(.*)"),
                make_value_string("/new/$1"),
            ],
            Some(opts),
        );
        let parent = make_parent_with_entries(vec![entry], vec![]);
        let mut used = HashSet::new();
        let validator = RewriteValidator;
        assert!(validator.validate_block(&parent, &mut used, false).is_err());
    }

    #[test]
    fn rejects_non_boolean_option() {
        let mut opts_directives = HashMap::new();
        opts_directives.insert(
            "last".to_string(),
            vec![make_rewrite_entry(vec![make_value_string("yes")], None)],
        );
        let opts = ServerConfigurationBlock {
            directives: Arc::new(opts_directives),
            matchers: HashMap::new(),
            span: None,
        };
        let entry = make_rewrite_entry(
            vec![
                make_value_string("^/old/(.*)"),
                make_value_string("/new/$1"),
            ],
            Some(opts),
        );
        let parent = make_parent_with_entries(vec![entry], vec![]);
        let mut used = HashSet::new();
        let validator = RewriteValidator;
        let err = validator
            .validate_block(&parent, &mut used, false)
            .unwrap_err();
        assert!(err.to_string().contains("boolean"));
    }

    #[test]
    fn valid_rewrite_log() {
        let entry = make_rewrite_entry(vec![make_value_bool(true)], None);
        let parent = make_parent_with_entries(vec![], vec![entry]);
        let mut used = HashSet::new();
        let validator = RewriteValidator;
        assert!(validator.validate_block(&parent, &mut used, false).is_ok());
    }

    #[test]
    fn rejects_rewrite_log_non_boolean() {
        let entry = make_rewrite_entry(vec![make_value_string("yes")], None);
        let parent = make_parent_with_entries(vec![], vec![entry]);
        let mut used = HashSet::new();
        let validator = RewriteValidator;
        assert!(validator.validate_block(&parent, &mut used, false).is_err());
    }

    #[test]
    fn skips_block_without_rewrite_or_log() {
        let block = ServerConfigurationBlock {
            directives: Arc::new(HashMap::new()),
            matchers: HashMap::new(),
            span: None,
        };
        let mut used = HashSet::new();
        let validator = RewriteValidator;
        assert!(validator.validate_block(&block, &mut used, false).is_ok());
    }
}
