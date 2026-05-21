//! Configuration validator for the `map` directive.
//!
//! Validates that `map` entries contain recognized sub-directives
//! (`default`, `exact`, `regex`) with valid argument types and block options.

use std::collections::HashSet;

use fancy_regex::Regex;
use ferron_core::config::validator::ConfigurationValidator;
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
        used_directives: &mut HashSet<String>,
        _is_global: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(entries) = config.directives.get("map") {
            used_directives.insert("map".to_string());
            for entry in entries {
                self.validate_map_entry(entry)?;
            }
        }

        Ok(())
    }
}

impl MapValidator {
    fn validate_map_entry(
        &self,
        entry: &ferron_core::config::ServerConfigurationDirectiveEntry,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Must have exactly 2 positional arguments: source and destination
        if entry.args.len() != 2 {
            return Err(format!(
                "Invalid `map` — must have exactly two arguments (source variable and destination variable), got {}",
                entry.args.len()
            )
            .into());
        }

        // First arg must be a string (source variable)
        if !matches!(
            &entry.args[0],
            ServerConfigurationValue::String(_, _)
                | ServerConfigurationValue::InterpolatedString(_, _)
        ) {
            return Err(
                "Invalid `map` — the source must be a plain string or interpolated string".into(),
            );
        }

        // Second arg must be a string (destination variable name)
        if !matches!(
            &entry.args[1],
            ServerConfigurationValue::String(_, _)
                | ServerConfigurationValue::InterpolatedString(_, _)
        ) {
            return Err("Invalid `map` — the destination variable name must be a string".into());
        }

        // Must have a child block
        let Some(children) = &entry.children else {
            return Err("Invalid `map` — a nested block with mapping entries is required".into());
        };

        // Validate the child block
        self.validate_map_block(children)?;

        Ok(())
    }

    fn validate_map_block(
        &self,
        block: &ServerConfigurationBlock,
    ) -> Result<(), Box<dyn std::error::Error>> {
        for (key, entries) in block.directives.iter() {
            if !MAP_BLOCK_DIRECTIVES.contains(&key.as_str()) {
                return Err(format!(
                    "Invalid `map` — unknown sub-directive `{key}` inside map block (recognized: {})",
                    MAP_BLOCK_DIRECTIVES.join(", ")
                )
                .into());
            }

            for entry in entries {
                match key.as_str() {
                    "default" => self.validate_default_entry(entry)?,
                    "exact" => self.validate_exact_entry(entry)?,
                    "regex" => self.validate_regex_entry(entry)?,
                    _ => unreachable!(),
                }
            }
        }

        Ok(())
    }

    fn validate_default_entry(
        &self,
        entry: &ferron_core::config::ServerConfigurationDirectiveEntry,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if entry.args.len() != 1 {
            return Err(format!(
                "Invalid `default` inside map block — must have exactly one argument (the default value), got {}",
                entry.args.len()
            )
            .into());
        }

        if !matches!(
            &entry.args[0],
            ServerConfigurationValue::String(_, _)
                | ServerConfigurationValue::InterpolatedString(_, _)
        ) {
            return Err("Invalid `default` inside map block — the value must be a string".into());
        }

        Ok(())
    }

    fn validate_exact_entry(
        &self,
        entry: &ferron_core::config::ServerConfigurationDirectiveEntry,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if entry.args.len() != 2 {
            return Err(format!(
                "Invalid `exact` inside map block — must have exactly two arguments (pattern and result), got {}",
                entry.args.len()
            )
            .into());
        }

        if !matches!(
            &entry.args[0],
            ServerConfigurationValue::String(_, _)
                | ServerConfigurationValue::InterpolatedString(_, _)
        ) {
            return Err("Invalid `exact` inside map block — the pattern must be a string".into());
        }

        if !matches!(
            &entry.args[1],
            ServerConfigurationValue::String(_, _)
                | ServerConfigurationValue::InterpolatedString(_, _)
        ) {
            return Err("Invalid `exact` inside map block — the result must be a string".into());
        }

        Ok(())
    }

    fn validate_regex_entry(
        &self,
        entry: &ferron_core::config::ServerConfigurationDirectiveEntry,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if entry.args.len() != 2 {
            return Err(format!(
                "Invalid `regex` inside map block — must have exactly two arguments (pattern and result), got {}",
                entry.args.len()
            )
            .into());
        }

        // First arg must be a string (regex pattern)
        if !matches!(
            &entry.args[0],
            ServerConfigurationValue::String(_, _)
                | ServerConfigurationValue::InterpolatedString(_, _)
        ) {
            return Err("Invalid `regex` inside map block — the pattern must be a string".into());
        }

        // Second arg must be a string (result)
        if !matches!(
            &entry.args[1],
            ServerConfigurationValue::String(_, _)
                | ServerConfigurationValue::InterpolatedString(_, _)
        ) {
            return Err("Invalid `regex` inside map block — the result must be a string".into());
        }

        // Validate regex compiles
        if let ServerConfigurationValue::String(pattern, span) = &entry.args[0] {
            if let Err(e) = Regex::new(pattern) {
                let location = span.as_ref().map_or_else(String::new, |s| {
                    format!(
                        " (file '{}', line {}, column {})",
                        s.file.as_deref().unwrap_or("unknown"),
                        s.line,
                        s.column
                    )
                });
                return Err(format!(
                    "Invalid `regex` inside map block — failed to compile regular expression{location}: {e}"
                ).into());
            }
        }

        // Validate optional block
        if let Some(ref children) = entry.children {
            self.validate_regex_block_options(children)?;
        }

        Ok(())
    }

    fn validate_regex_block_options(
        &self,
        children: &ServerConfigurationBlock,
    ) -> Result<(), Box<dyn std::error::Error>> {
        for (key, nested_entries) in children.directives.iter() {
            if !REGEX_OPTIONS.contains(&key.as_str()) {
                return Err(format!(
                    "Invalid `regex` inside map block — unknown option `{key}` (recognized options: {})",
                    REGEX_OPTIONS.join(", ")
                )
                .into());
            }
            for nested_entry in nested_entries {
                if nested_entry.args.is_empty() {
                    continue; // No args means default value (false)
                }
                if nested_entry.args.len() != 1 {
                    return Err(format!(
                        "Invalid `{key}` inside regex block — must have zero or one argument"
                    )
                    .into());
                }
                match &nested_entry.args[0] {
                    ServerConfigurationValue::Boolean(_, _) => {}
                    _ => {
                        return Err(format!(
                            "Invalid `{key}` inside regex block — must be a boolean"
                        )
                        .into());
                    }
                }
            }
        }
        Ok(())
    }
}
