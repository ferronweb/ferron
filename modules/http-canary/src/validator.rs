use ferron_core::config::validator::{
    entry_span, ConfigurationValidationError, ConfigurationValidator,
};
use ferron_core::config::{ServerConfigurationBlock, ServerConfigurationValue};

/// Recognized sub-directives inside a `canary { ... }` block.
const CANARY_BLOCK_DIRECTIVES: &[&str] = &["affinity", "variant", "set_cookie"];

/// Recognized affinity keywords.
const AFFINITY_KEYWORDS: &[&str] = &["ip", "cookie", "header", "hash"];

/// Validator for `canary` configuration.
#[derive(Default)]
pub struct CanaryValidator;

impl ConfigurationValidator for CanaryValidator {
    fn validate_block(
        &self,
        config: &ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), ConfigurationValidationError> {
        if let Some(entries) = config.directives.get("canary") {
            ctx.used_directives.insert("canary".to_string());
            for entry in entries {
                self.validate_canary_entry(entry, ctx)?;
            }
        }

        Ok(())
    }
}

/// Whether the block declares a `cookie`-based affinity.
#[inline]
fn has_cookie_affinity(block: &ServerConfigurationBlock) -> bool {
    block
        .directives
        .get("affinity")
        .and_then(|entries| entries.first())
        .and_then(|entry| entry.args.first())
        .and_then(|arg| arg.as_str())
        == Some("cookie")
}

impl CanaryValidator {
    fn validate_canary_entry(
        &self,
        entry: &ferron_core::config::ServerConfigurationDirectiveEntry,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), ConfigurationValidationError> {
        if entry.args.len() != 1 {
            return Err(ConfigurationValidationError::from(format!(
                "Invalid `canary` — must have exactly one argument (the canary name), got {}",
                entry.args.len()
            ))
            .with_span(entry_span(entry)));
        }

        if !matches!(
            &entry.args[0],
            ServerConfigurationValue::String(_, _)
                | ServerConfigurationValue::InterpolatedString(_, _)
        ) {
            return Err(ConfigurationValidationError::from(
                "Invalid `canary` — the canary name must be a string",
            )
            .with_span(entry_span(entry)));
        }

        let Some(children) = &entry.children else {
            return Err(ConfigurationValidationError::from(
                "Invalid `canary` — a nested block with `variant` entries is required",
            )
            .with_span(entry_span(entry)));
        };

        self.validate_canary_block(children, ctx)
    }

    fn validate_canary_block(
        &self,
        block: &ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), ConfigurationValidationError> {
        let mut sub = std::collections::HashSet::new();
        let mut variant_names = std::collections::HashSet::new();
        let mut variant_count = 0usize;

        for (key, entries) in block.directives.iter() {
            if !CANARY_BLOCK_DIRECTIVES.contains(&key.as_str()) {
                return Err(ConfigurationValidationError::from(format!(
                    "Invalid `canary` — unknown sub-directive `{key}` inside canary block (recognized: {})",
                    CANARY_BLOCK_DIRECTIVES.join(", ")
                ))
                .with_span(entry_span(
                    entries.first().expect("non-empty block directives"),
                )));
            }

            sub.insert(key.clone());
            for entry in entries {
                match key.as_str() {
                    "affinity" => self.validate_affinity_entry(entry)?,
                    "variant" => {
                        variant_count += 1;
                        if let Some(name) = self.validate_variant_entry(entry)? {
                            if !variant_names.insert(name.clone()) {
                                return Err(ConfigurationValidationError::from(format!(
                                    "Invalid `variant` — duplicate variant name `{name}` inside canary block"
                                ))
                                .with_span(entry_span(entry)));
                            }
                        }
                    }
                    "set_cookie" => self.validate_set_cookie_entry(entry)?,
                    _ => unreachable!(),
                }
            }
        }
        ferron_core::check_unused_subdirectives!(
            block,
            sub,
            &mut ctx.diagnostics,
            ctx.scope.clone()
        );

        if variant_count == 0 {
            return Err(ConfigurationValidationError::from(
                "Invalid `canary` — at least one `variant` entry is required",
            )
            .with_span(block.span.clone()));
        }

        if sub.contains("set_cookie") && !has_cookie_affinity(block) {
            return Err(ConfigurationValidationError::from(
                "Invalid `canary` — `set_cookie` requires `affinity cookie <name>` in the same block",
            )
            .with_span(block.span.clone()));
        }

        Ok(())
    }

    fn validate_set_cookie_entry(
        &self,
        entry: &ferron_core::config::ServerConfigurationDirectiveEntry,
    ) -> Result<(), ConfigurationValidationError> {
        if entry.args.len() > 1 {
            return Err(ConfigurationValidationError::from(format!(
                "Invalid `set_cookie` — takes at most one boolean argument, got {}",
                entry.args.len()
            ))
            .with_span(entry_span(entry)));
        }

        Ok(())
    }

    fn validate_affinity_entry(
        &self,
        entry: &ferron_core::config::ServerConfigurationDirectiveEntry,
    ) -> Result<(), ConfigurationValidationError> {
        let Some(keyword) = entry.args.first().and_then(|a| a.as_str()) else {
            return Err(ConfigurationValidationError::from(format!(
                "Invalid `affinity` — must have a keyword argument (recognized: {})",
                AFFINITY_KEYWORDS.join(", ")
            ))
            .with_span(entry_span(entry)));
        };

        if !AFFINITY_KEYWORDS.contains(&keyword) {
            return Err(ConfigurationValidationError::from(format!(
                "Invalid `affinity` — unknown keyword `{keyword}` (recognized: {})",
                AFFINITY_KEYWORDS.join(", ")
            ))
            .with_span(entry_span(entry)));
        }

        match keyword {
            "ip" => {
                if entry.args.len() != 1 {
                    return Err(ConfigurationValidationError::from(
                        "Invalid `affinity` — `ip` takes no additional arguments",
                    )
                    .with_span(entry_span(entry)));
                }
            }
            "cookie" | "header" | "hash" => {
                if entry.args.len() != 2 {
                    return Err(ConfigurationValidationError::from(format!(
                        "Invalid `affinity` — `{keyword}` requires exactly one name argument, got {}",
                        entry.args.len().saturating_sub(1)
                    ))
                    .with_span(entry_span(entry)));
                }
                if !matches!(
                    &entry.args[1],
                    ServerConfigurationValue::String(_, _)
                        | ServerConfigurationValue::InterpolatedString(_, _)
                ) {
                    return Err(ConfigurationValidationError::from(format!(
                        "Invalid `affinity` — the `{keyword}` name must be a string"
                    ))
                    .with_span(entry_span(entry)));
                }
            }
            _ => unreachable!(),
        }

        Ok(())
    }

    /// Validate a `variant` entry and return its name on success.
    fn validate_variant_entry(
        &self,
        entry: &ferron_core::config::ServerConfigurationDirectiveEntry,
    ) -> Result<Option<String>, ConfigurationValidationError> {
        if entry.args.len() != 2 {
            return Err(ConfigurationValidationError::from(format!(
                "Invalid `variant` — must have exactly two arguments (name and weight), got {}",
                entry.args.len()
            ))
            .with_span(entry_span(entry)));
        }

        let Some(name) = entry.args[0].as_str() else {
            return Err(ConfigurationValidationError::from(
                "Invalid `variant` — the variant name must be a string",
            )
            .with_span(entry_span(entry)));
        };

        let Some(weight) = entry.args[1].as_number() else {
            return Err(ConfigurationValidationError::from(
                "Invalid `variant` — the weight must be a positive whole number",
            )
            .with_span(entry_span(entry)));
        };

        if weight < 1 {
            return Err(ConfigurationValidationError::from(
                "Invalid `variant` — the weight must be at least 1",
            )
            .with_span(entry_span(entry)));
        }

        Ok(Some(name.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferron_core::config::{ServerConfigurationDirectiveEntry, ServerConfigurationValue};
    use std::collections::HashMap as StdHashMap;
    use std::sync::Arc;

    fn make_value_string(s: &str) -> ServerConfigurationValue {
        ServerConfigurationValue::String(s.to_string(), None)
    }

    fn make_value_number(n: i64) -> ServerConfigurationValue {
        ServerConfigurationValue::Number(n, None)
    }

    fn make_entry(
        args: Vec<ServerConfigurationValue>,
        children: Option<ServerConfigurationBlock>,
    ) -> ServerConfigurationDirectiveEntry {
        ServerConfigurationDirectiveEntry {
            args,
            children,
            span: None,
        }
    }

    fn make_block(
        directives: StdHashMap<String, Vec<ServerConfigurationDirectiveEntry>>,
    ) -> ServerConfigurationBlock {
        ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: StdHashMap::new(),
            span: None,
        }
    }

    fn make_config_block(children: ServerConfigurationBlock) -> ServerConfigurationBlock {
        let mut directives = StdHashMap::new();
        directives.insert(
            "canary".to_string(),
            vec![make_entry(
                vec![make_value_string("ab_test")],
                Some(children),
            )],
        );
        make_block(directives)
    }

    fn make_canary_block(
        variants: Vec<ServerConfigurationDirectiveEntry>,
    ) -> ServerConfigurationBlock {
        let mut directives = StdHashMap::new();
        directives.insert("variant".to_string(), variants);
        make_block(directives)
    }

    fn run_validator(block: &ServerConfigurationBlock) -> Result<(), ConfigurationValidationError> {
        let mut ctx = ferron_core::config::validator::ConfigurationValidatorContext {
            used_directives: std::collections::HashSet::new(),
            is_global: false,
            // Test-only context: the empty validator map is not shared across threads.
            #[allow(clippy::arc_with_non_send_sync)]
            scoped_validators: Arc::new(StdHashMap::new()),
            diagnostics: Vec::new(),
            scope: Some("test".to_string()),
        };
        CanaryValidator.validate_block(block, &mut ctx)
    }

    #[test]
    fn validates_valid_canary() {
        let block = make_config_block(make_canary_block(vec![
            make_entry(
                vec![make_value_string("stable"), make_value_number(90)],
                None,
            ),
            make_entry(vec![make_value_string("new"), make_value_number(10)], None),
        ]));
        assert!(run_validator(&block).is_ok());
    }

    #[test]
    fn rejects_missing_variants() {
        let block = make_config_block(make_canary_block(vec![]));
        let err = run_validator(&block).unwrap_err();
        assert!(err.to_string().contains("at least one `variant`"));
    }

    #[test]
    fn rejects_duplicate_variant_names() {
        let block = make_config_block(make_canary_block(vec![
            make_entry(vec![make_value_string("same"), make_value_number(1)], None),
            make_entry(vec![make_value_string("same"), make_value_number(2)], None),
        ]));
        let err = run_validator(&block).unwrap_err();
        assert!(err.to_string().contains("duplicate variant name"));
    }

    #[test]
    fn rejects_invalid_weights() {
        let block = make_config_block(make_canary_block(vec![make_entry(
            vec![make_value_string("zero"), make_value_number(0)],
            None,
        )]));
        let err = run_validator(&block).unwrap_err();
        assert!(err.to_string().contains("weight must be at least 1"));
    }

    #[test]
    fn rejects_unknown_affinity_keyword() {
        let mut directives = StdHashMap::new();
        directives.insert(
            "affinity".to_string(),
            vec![make_entry(vec![make_value_string("magic")], None)],
        );
        directives.insert(
            "variant".to_string(),
            vec![make_entry(
                vec![make_value_string("a"), make_value_number(1)],
                None,
            )],
        );
        let block = make_config_block(make_block(directives));
        let err = run_validator(&block).unwrap_err();
        assert!(err.to_string().contains("unknown keyword"));
    }

    #[test]
    fn rejects_affinity_missing_name() {
        let mut directives = StdHashMap::new();
        directives.insert(
            "affinity".to_string(),
            vec![make_entry(vec![make_value_string("cookie")], None)],
        );
        directives.insert(
            "variant".to_string(),
            vec![make_entry(
                vec![make_value_string("a"), make_value_number(1)],
                None,
            )],
        );
        let block = make_config_block(make_block(directives));
        let err = run_validator(&block).unwrap_err();
        assert!(err
            .to_string()
            .contains("requires exactly one name argument"));
    }

    #[test]
    fn rejects_unknown_sub_directive() {
        let mut directives = StdHashMap::new();
        directives.insert(
            "frobnicate".to_string(),
            vec![make_entry(vec![make_value_string("x")], None)],
        );
        let block = make_config_block(make_block(directives));
        let err = run_validator(&block).unwrap_err();
        assert!(err.to_string().contains("unknown sub-directive"));
    }

    #[test]
    fn rejects_wrong_canary_argument_count() {
        let mut directives = StdHashMap::new();
        directives.insert(
            "canary".to_string(),
            vec![make_entry(
                vec![make_value_string("a"), make_value_string("b")],
                None,
            )],
        );
        let block = make_block(directives);
        let err = run_validator(&block).unwrap_err();
        assert!(err.to_string().contains("exactly one argument"));
    }

    #[test]
    fn accepts_set_cookie_with_cookie_affinity() {
        let mut directives = StdHashMap::new();
        directives.insert(
            "affinity".to_string(),
            vec![make_entry(
                vec![make_value_string("cookie"), make_value_string("ab_variant")],
                None,
            )],
        );
        directives.insert("set_cookie".to_string(), vec![make_entry(vec![], None)]);
        directives.insert(
            "variant".to_string(),
            vec![make_entry(
                vec![make_value_string("a"), make_value_number(1)],
                None,
            )],
        );
        let block = make_config_block(make_block(directives));
        assert!(run_validator(&block).is_ok());
    }

    #[test]
    fn rejects_set_cookie_without_cookie_affinity() {
        let mut directives = StdHashMap::new();
        directives.insert(
            "affinity".to_string(),
            vec![make_entry(vec![make_value_string("ip")], None)],
        );
        directives.insert("set_cookie".to_string(), vec![make_entry(vec![], None)]);
        directives.insert(
            "variant".to_string(),
            vec![make_entry(
                vec![make_value_string("a"), make_value_number(1)],
                None,
            )],
        );
        let block = make_config_block(make_block(directives));
        let err = run_validator(&block).unwrap_err();
        assert!(err.to_string().contains("requires `affinity cookie"));
    }

    #[test]
    fn rejects_set_cookie_with_too_many_arguments() {
        let mut directives = StdHashMap::new();
        directives.insert(
            "affinity".to_string(),
            vec![make_entry(
                vec![make_value_string("cookie"), make_value_string("ab_variant")],
                None,
            )],
        );
        directives.insert(
            "set_cookie".to_string(),
            vec![make_entry(
                vec![make_value_string("a"), make_value_string("b")],
                None,
            )],
        );
        directives.insert(
            "variant".to_string(),
            vec![make_entry(
                vec![make_value_string("a"), make_value_number(1)],
                None,
            )],
        );
        let block = make_config_block(make_block(directives));
        let err = run_validator(&block).unwrap_err();
        assert!(err.to_string().contains("at most one boolean argument"));
    }
}
