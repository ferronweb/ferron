use ferron_core::config::validator::{
    entry_span, ConfigurationValidationError, ConfigurationValidator,
};
use ferron_core::config::{ServerConfigurationBlock, ServerConfigurationValue};

const SET_VAR_BLOCK_DIRECTIVES: &[&str] = &["value", "case_insensitive", "negate"];

#[derive(Default)]
pub struct VariablesValidator;

impl ConfigurationValidator for VariablesValidator {
    fn validate_block(
        &self,
        config: &ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
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
    ) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
        if entry.args.len() != 3 {
            return Err(ConfigurationValidationError::from(format!(
                "Invalid `set_var` — must have exactly three arguments (source, regex, variable), got {}",
                entry.args.len()
            ))
            .with_span(entry_span(entry)));
        }

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
                return Err(ConfigurationValidationError::from(format!(
                    "Invalid `set_var` — the {label} argument must be a string"
                ))
                .with_span(entry_span(entry)));
            }
        }

        if let ServerConfigurationValue::String(pattern, span) = &entry.args[1] {
            if let Err(e) = regex::Regex::new(pattern) {
                return Err(ConfigurationValidationError::from(format!(
                    "Invalid `set_var` — failed to compile regular expression: {e}"
                ))
                .with_span(span.clone()));
            }
        }

        if let Some(children) = &entry.children {
            self.validate_set_var_block(children, ctx)?;
        }

        Ok(())
    }

    fn validate_set_var_block(
        &self,
        block: &ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
        let mut sub = std::collections::HashSet::new();
        for (key, entries) in block.directives.iter() {
            if !SET_VAR_BLOCK_DIRECTIVES.contains(&key.as_str()) {
                return Err(ConfigurationValidationError::from(format!(
                    "Invalid `set_var` — unknown sub-directive `{key}` inside set_var block (recognized: {})",
                    SET_VAR_BLOCK_DIRECTIVES.join(", ")
                ))
                .with_span(entry_span(
                    entries.first().expect("non-empty block directives"),
                )));
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
    ) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
        if entry.args.len() != 1 {
            return Err(ConfigurationValidationError::from(format!(
                "Invalid `value` inside set_var block — must have exactly one argument, got {}",
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
                "Invalid `value` inside set_var block — the value must be a string",
            )
            .with_span(entry_span(entry)));
        }

        Ok(())
    }

    fn validate_boolean_entry(
        &self,
        entry: &ferron_core::config::ServerConfigurationDirectiveEntry,
        name: &str,
    ) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
        if !(0..=1).contains(&entry.args.len()) {
            return Err(ConfigurationValidationError::from(format!(
                "Invalid `{name}` inside set_var block — must have zero or one argument"
            ))
            .with_span(entry_span(entry)));
        }

        if !matches!(
            entry.args.first(),
            Some(ServerConfigurationValue::Boolean(_, _)) | None
        ) {
            return Err(ConfigurationValidationError::from(format!(
                "Invalid `{name}` inside set_var block — must be a boolean"
            ))
            .with_span(entry_span(entry)));
        }

        Ok(())
    }

    fn validate_log_field_entry(
        &self,
        entry: &ferron_core::config::ServerConfigurationDirectiveEntry,
        _ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
        if entry.args.len() != 2 {
            return Err(ConfigurationValidationError::from(format!(
                "Invalid `log_field` — must have exactly two arguments (field name and source), got {}",
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
                "Invalid `log_field` — the field name must be a string",
            )
            .with_span(entry_span(entry)));
        }

        if !matches!(
            &entry.args[1],
            ServerConfigurationValue::String(_, _)
                | ServerConfigurationValue::InterpolatedString(_, _)
        ) {
            return Err(ConfigurationValidationError::from(
                "Invalid `log_field` — the source must be a string or interpolated string",
            )
            .with_span(entry_span(entry)));
        }

        Ok(())
    }
}
