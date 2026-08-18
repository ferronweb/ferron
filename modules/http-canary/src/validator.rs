use ferron_core::config::validator::{
    entry_span, ConfigurationValidationError, ConfigurationValidator,
};
use ferron_core::config::{ServerConfigurationBlock, ServerConfigurationValue};

/// Recognized sub-directives inside a `canary { ... }` block.
const CANARY_BLOCK_DIRECTIVES: &[&str] = &["affinity", "variant", "set_cookie", "cookie"];

/// Recognized sub-directives inside a `cookie { ... }` block.
const COOKIE_BLOCK_DIRECTIVES: &[&str] =
    &["ttl", "path", "domain", "secure", "httponly", "samesite"];

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
                    "cookie" => self.validate_cookie_entry(entry, ctx)?,
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

    fn validate_cookie_entry(
        &self,
        entry: &ferron_core::config::ServerConfigurationDirectiveEntry,
        _ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), ConfigurationValidationError> {
        let Some(children) = &entry.children else {
            return Ok(());
        };

        for (key, entries) in children.directives.iter() {
            if !COOKIE_BLOCK_DIRECTIVES.contains(&key.as_str()) {
                return Err(ConfigurationValidationError::from(format!(
                    "Invalid `cookie` — unknown sub-directive `{key}` inside cookie block (recognized: {})",
                    COOKIE_BLOCK_DIRECTIVES.join(", ")
                ))
                .with_span(entry_span(entries.first().expect("non-empty block directives"))));
            }

            for entry in entries {
                match key.as_str() {
                    "ttl" => {
                        if entry.args.len() != 1 {
                            return Err(ConfigurationValidationError::from(format!(
                                "Invalid `ttl` — requires exactly one duration argument, got {}",
                                entry.args.len()
                            ))
                            .with_span(entry_span(entry)));
                        }
                        if entry.args[0].as_duration().is_none() {
                            return Err(ConfigurationValidationError::from(
                                "Invalid `ttl` — the value must be a duration",
                            )
                            .with_span(entry_span(entry)));
                        }
                    }
                    "path" | "domain" | "samesite" => {
                        if entry.args.len() != 1 {
                            return Err(ConfigurationValidationError::from(format!(
                                "Invalid `{key}` — requires exactly one argument, got {}",
                                entry.args.len()
                            ))
                            .with_span(entry_span(entry)));
                        }
                        if entry.args[0].as_str().is_none() {
                            return Err(ConfigurationValidationError::from(format!(
                                "Invalid `{key}` — the value must be a string"
                            ))
                            .with_span(entry_span(entry)));
                        }
                        if key.as_str() == "samesite" {
                            let val = entry.args[0].as_str().unwrap().to_lowercase();
                            if !matches!(val.as_str(), "strict" | "lax" | "none") {
                                return Err(ConfigurationValidationError::from(
                                    "Invalid `samesite` — must be one of strict, lax, or none",
                                )
                                .with_span(entry_span(entry)));
                            }
                        }
                    }
                    "secure" | "httponly" => {
                        if entry.args.len() > 1 {
                            return Err(ConfigurationValidationError::from(format!(
                                "Invalid `{key}` — takes at most one boolean argument, got {}",
                                entry.args.len()
                            ))
                            .with_span(entry_span(entry)));
                        }
                    }
                    _ => unreachable!(),
                }
            }
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
