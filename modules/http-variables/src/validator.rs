//! Configuration validation for the `set_var` and `log_field` directives.

use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::config::{ServerConfigurationBlock, ServerConfigurationValue};

/// Recognized sub-directives inside a `set_var { ... }` block.
const SET_VAR_BLOCK_DIRECTIVES: &[&str] = &["value", "case_insensitive", "negate"];

/// Validator for `set_var` and `log_field` configuration.
#[derive(Default)]
pub struct VariablesValidator;

impl ConfigurationValidator for VariablesValidator {
    fn validate_block(
        &self,
        config: &ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(entries) = config.directives.get("set_var") {
            ctx.used_directives.insert("set_var".to_string());
            for entry in entries {
                self.validate_set_var_entry(entry, ctx)?;
            }
        }

        if let Some(entries) = config.directives.get("log_field") {
            ctx.used_directives.insert("log_field".to_string());
            for entry in entries {
                self.validate_log_field_entry(entry, ctx)?;
            }
        }

        Ok(())
    }
}

impl VariablesValidator {
    fn validate_set_var_entry(
        &self,
        entry: &ferron_core::config::ServerConfigurationDirectiveEntry,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Must have exactly 3 positional arguments: source, regex, variable
        if entry.args.len() != 3 {
            return Err(format!(
                "Invalid `set_var` — must have exactly three arguments (source, regex, variable), got {}",
                entry.args.len()
            )
            .into());
        }

        // All three args must be strings
        for (i, arg) in entry.args.iter().enumerate() {
            let label = match i {
                0 => "source",
                1 => "regex",
                2 => "variable",
                _ => unreachable!(),
            };
            if !matches!(
                arg,
                ServerConfigurationValue::String(_, _)
                    | ServerConfigurationValue::InterpolatedString(_, _)
            ) {
                return Err(
                    format!("Invalid `set_var` — the {label} argument must be a string").into(),
                );
            }
        }

        // Validate regex compiles
        if let ServerConfigurationValue::String(pattern, span) = &entry.args[1] {
            if let Err(e) = fancy_regex::Regex::new(pattern) {
                let location = span.as_ref().map_or_else(String::new, |s| {
                    format!(
                        " (file '{}', line {}, column {})",
                        s.file.as_deref().unwrap_or("unknown"),
                        s.line,
                        s.column
                    )
                });
                return Err(format!(
                    "Invalid `set_var` — failed to compile regular expression{location}: {e}"
                )
                .into());
            }
        }

        // Validate optional block
        if let Some(children) = &entry.children {
            self.validate_set_var_block(children, ctx)?;
        }

        Ok(())
    }

    fn validate_set_var_block(
        &self,
        block: &ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut sub = std::collections::HashSet::new();
        for (key, entries) in block.directives.iter() {
            if !SET_VAR_BLOCK_DIRECTIVES.contains(&key.as_str()) {
                return Err(format!(
                    "Invalid `set_var` — unknown sub-directive `{key}` inside set_var block (recognized: {})",
                    SET_VAR_BLOCK_DIRECTIVES.join(", ")
                )
                .into());
            }

            sub.insert(key.clone());
            for entry in entries {
                match key.as_str() {
                    "value" => self.validate_value_entry(entry)?,
                    "case_insensitive" | "negate" => self.validate_boolean_entry(entry, key)?,
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

    fn validate_value_entry(
        &self,
        entry: &ferron_core::config::ServerConfigurationDirectiveEntry,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if entry.args.len() != 1 {
            return Err(format!(
                "Invalid `value` inside set_var block — must have exactly one argument, got {}",
                entry.args.len()
            )
            .into());
        }

        if !matches!(
            &entry.args[0],
            ServerConfigurationValue::String(_, _)
                | ServerConfigurationValue::InterpolatedString(_, _)
        ) {
            return Err("Invalid `value` inside set_var block — the value must be a string".into());
        }

        Ok(())
    }

    fn validate_boolean_entry(
        &self,
        entry: &ferron_core::config::ServerConfigurationDirectiveEntry,
        name: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if entry.args.len() != 1 {
            return Err(format!(
                "Invalid `{name}` inside set_var block — must have exactly one argument"
            )
            .into());
        }

        if !matches!(&entry.args[0], ServerConfigurationValue::Boolean(_, _)) {
            return Err(
                format!("Invalid `{name}` inside set_var block — must be a boolean").into(),
            );
        }

        Ok(())
    }

    fn validate_log_field_entry(
        &self,
        entry: &ferron_core::config::ServerConfigurationDirectiveEntry,
        _ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Must have exactly 2 positional arguments: field_name, source
        if entry.args.len() != 2 {
            return Err(format!(
                "Invalid `log_field` — must have exactly two arguments (field name and source), got {}",
                entry.args.len()
            )
            .into());
        }

        // First arg (field name) must be a string
        if !matches!(
            &entry.args[0],
            ServerConfigurationValue::String(_, _)
                | ServerConfigurationValue::InterpolatedString(_, _)
        ) {
            return Err("Invalid `log_field` — the field name must be a string".into());
        }

        // Second arg (source) can be a plain string or interpolated string
        if !matches!(
            &entry.args[1],
            ServerConfigurationValue::String(_, _)
                | ServerConfigurationValue::InterpolatedString(_, _)
        ) {
            return Err(
                "Invalid `log_field` — the source must be a string or interpolated string".into(),
            );
        }

        Ok(())
    }
}
