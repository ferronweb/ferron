use fancy_regex::Regex;
use ferron_core::config::validator::{
    entry_span, ConfigurationValidationError, ConfigurationValidator,
};
use ferron_core::config::{ServerConfigurationBlock, ServerConfigurationValue};

/// Recognized sub-directives inside a `map { ... }` block.
const MAP_BLOCK_DIRECTIVES: &[&str] = &["default", "exact", "regex"];

/// Recognized options inside a `regex { ... }` block.
const REGEX_OPTIONS: &[&str] = &["case_insensitive"];

/// Validator for `map` configuration.
#[derive(Default)]
pub struct MapValidator;

impl ConfigurationValidator for MapValidator {
    fn validate_block(
        &self,
        config: &ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
        if let Some(entries) = config.directives.get("map") {
            ctx.used_directives.insert("map".to_string());
            for entry in entries {
                self.validate_map_entry(entry, ctx)?;
            }
        }

        Ok(())
    }
}

impl MapValidator {
    fn validate_map_entry(
        &self,
        entry: &ferron_core::config::ServerConfigurationDirectiveEntry,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
        if entry.args.len() != 2 {
            return Err(ConfigurationValidationError::from(format!(
                "Invalid `map` — must have exactly two arguments (source variable and destination variable), got {}",
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
                "Invalid `map` — the source must be a plain string or interpolated string",
            )
            .with_span(entry_span(entry)));
        }

        if !matches!(
            &entry.args[1],
            ServerConfigurationValue::String(_, _)
                | ServerConfigurationValue::InterpolatedString(_, _)
        ) {
            return Err(ConfigurationValidationError::from(
                "Invalid `map` — the destination variable name must be a string",
            )
            .with_span(entry_span(entry)));
        }

        let Some(children) = &entry.children else {
            return Err(ConfigurationValidationError::from(
                "Invalid `map` — a nested block with mapping entries is required",
            )
            .with_span(entry_span(entry)));
        };

        self.validate_map_block(children, ctx)?;

        Ok(())
    }

    fn validate_map_block(
        &self,
        block: &ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
        let mut sub = std::collections::HashSet::new();
        for (key, entries) in block.directives.iter() {
            if !MAP_BLOCK_DIRECTIVES.contains(&key.as_str()) {
                return Err(ConfigurationValidationError::from(format!(
                    "Invalid `map` — unknown sub-directive `{key}` inside map block (recognized: {})",
                    MAP_BLOCK_DIRECTIVES.join(", ")
                ))
                .with_span(entry_span(
                    entries.first().expect("non-empty block directives"),
                )));
            }

            sub.insert(key.clone());
            for entry in entries {
                match key.as_str() {
                    "default" => self.validate_default_entry(entry)?,
                    "exact" => self.validate_exact_entry(entry)?,
                    "regex" => self.validate_regex_entry(entry, ctx)?,
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
        Ok(())
    }

    fn validate_default_entry(
        &self,
        entry: &ferron_core::config::ServerConfigurationDirectiveEntry,
    ) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
        if entry.args.len() != 1 {
            return Err(ConfigurationValidationError::from(format!(
                "Invalid `default` inside map block — must have exactly one argument (the default value), got {}",
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
                "Invalid `default` inside map block — the value must be a string",
            )
            .with_span(entry_span(entry)));
        }

        Ok(())
    }

    fn validate_exact_entry(
        &self,
        entry: &ferron_core::config::ServerConfigurationDirectiveEntry,
    ) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
        if entry.args.len() != 2 {
            return Err(ConfigurationValidationError::from(format!(
                "Invalid `exact` inside map block — must have exactly two arguments (pattern and result), got {}",
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
                "Invalid `exact` inside map block — the pattern must be a string",
            )
            .with_span(entry_span(entry)));
        }

        if !matches!(
            &entry.args[1],
            ServerConfigurationValue::String(_, _)
                | ServerConfigurationValue::InterpolatedString(_, _)
        ) {
            return Err(ConfigurationValidationError::from(
                "Invalid `exact` inside map block — the result must be a string",
            )
            .with_span(entry_span(entry)));
        }

        Ok(())
    }

    fn validate_regex_entry(
        &self,
        entry: &ferron_core::config::ServerConfigurationDirectiveEntry,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
        if entry.args.len() != 2 {
            return Err(ConfigurationValidationError::from(format!(
                "Invalid `regex` inside map block — must have exactly two arguments (pattern and result), got {}",
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
                "Invalid `regex` inside map block — the pattern must be a string",
            )
            .with_span(entry_span(entry)));
        }

        if !matches!(
            &entry.args[1],
            ServerConfigurationValue::String(_, _)
                | ServerConfigurationValue::InterpolatedString(_, _)
        ) {
            return Err(ConfigurationValidationError::from(
                "Invalid `regex` inside map block — the result must be a string",
            )
            .with_span(entry_span(entry)));
        }

        if let ServerConfigurationValue::String(pattern, span) = &entry.args[0] {
            if let Err(e) = Regex::new(pattern) {
                return Err(ConfigurationValidationError::from(format!(
                    "Invalid `regex` inside map block — failed to compile regular expression: {e}"
                ))
                .with_span(span.clone()));
            }
        }

        if let Some(ref children) = entry.children {
            self.validate_regex_block_options(children, ctx)?;
        }

        Ok(())
    }

    fn validate_regex_block_options(
        &self,
        children: &ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
        let mut sub = std::collections::HashSet::new();
        for (key, nested_entries) in children.directives.iter() {
            if !REGEX_OPTIONS.contains(&key.as_str()) {
                return Err(ConfigurationValidationError::from(format!(
                    "Invalid `regex` inside map block — unknown option `{key}` (recognized options: {})",
                    REGEX_OPTIONS.join(", ")
                ))
                .with_span(entry_span(
                    nested_entries.first().expect("non-empty block directives"),
                )));
            }
            sub.insert(key.clone());
            for nested_entry in nested_entries {
                if nested_entry.args.is_empty() {
                    continue;
                }
                if nested_entry.args.len() != 1 {
                    return Err(ConfigurationValidationError::from(format!(
                        "Invalid `{key}` inside regex block — must have zero or one argument"
                    ))
                    .with_span(entry_span(nested_entry)));
                }
                match &nested_entry.args[0] {
                    ServerConfigurationValue::Boolean(_, _) => {}
                    _ => {
                        return Err(ConfigurationValidationError::from(format!(
                            "Invalid `{key}` inside regex block — must be a boolean"
                        ))
                        .with_span(entry_span(nested_entry)));
                    }
                }
            }
        }
        ferron_core::check_unused_subdirectives!(
            children,
            sub,
            &mut ctx.diagnostics,
            ctx.scope.clone()
        );
        Ok(())
    }
}
