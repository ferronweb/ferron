use ferron_core::config::validator::{
    entry_span, ConfigurationValidationError, ConfigurationValidator,
};
use ferron_core::config::{ServerConfigurationBlock, ServerConfigurationValue};

const RECOGNIZED_OPTIONS: &[&str] = &["last", "directory", "file", "allow_double_slashes"];

#[derive(Default)]
pub struct RewriteValidator;

impl ConfigurationValidator for RewriteValidator {
    fn validate_block(
        &self,
        config: &ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
        if let Some(entries) = config.directives.get("rewrite") {
            ctx.used_directives.insert("rewrite".to_string());
            for entry in entries {
                self.validate_rewrite_entry(entry, ctx)?;
            }
        }

        if let Some(entries) = config.directives.get("rewrite_log") {
            ctx.used_directives.insert("rewrite_log".to_string());
            for entry in entries {
                self.validate_rewrite_log_entry(entry)?;
            }
        }

        Ok(())
    }
}

impl RewriteValidator {
    fn validate_rewrite_entry(
        &self,
        entry: &ferron_core::config::ServerConfigurationDirectiveEntry,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
        if entry.args.len() != 2 {
            return Err(ConfigurationValidationError::from(format!(
                "Invalid `rewrite` — must have exactly two arguments (regex and replacement), got {}",
                entry.args.len()
            ))
            .with_span(entry_span(entry)));
        }

        // Compiling full regex just to validate it... :')
        if entry.args[0]
            .as_string_with_interpolations(&std::collections::HashMap::new())
            .is_none_or(|ref re| fancy_regex::Regex::new(re).is_err())
        {
            return Err(ConfigurationValidationError::from(
                "Invalid `rewrite` — the regular expression must be a valid regular expression string",
            )
            .with_span(entry_span(entry)));
        }

        if !matches!(
            &entry.args[1],
            ServerConfigurationValue::String(_, _)
                | ServerConfigurationValue::InterpolatedString(_, _)
        ) {
            return Err(ConfigurationValidationError::from(
                "Invalid `rewrite` — the replacement must be a string",
            )
            .with_span(entry_span(entry)));
        }

        if let Some(ref children) = entry.children {
            self.validate_rewrite_block_options(children, ctx)?;
        }

        Ok(())
    }

    fn validate_rewrite_block_options(
        &self,
        children: &ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
        let mut sub = std::collections::HashSet::new();
        for (key, nested_entries) in children.directives.iter() {
            if !RECOGNIZED_OPTIONS.contains(&key.as_str()) {
                return Err(ConfigurationValidationError::from(format!(
                    "Invalid `rewrite` — unknown option `{key}` in rewrite block (recognized options: {})",
                    RECOGNIZED_OPTIONS.join(", ")
                ))
                .with_span(entry_span(
                    nested_entries.first().expect("non-empty block directives"),
                )));
            }
            sub.insert(key.clone());
            for nested_entry in nested_entries {
                if nested_entry.args.len() > 1 {
                    return Err(ConfigurationValidationError::from(format!(
                        "Invalid `{key}` — must have at most one value"
                    ))
                    .with_span(entry_span(nested_entry)));
                }
                if !nested_entry.args.is_empty() {
                    match &nested_entry.args[0] {
                        ServerConfigurationValue::Boolean(_, _) => {}
                        _ => {
                            return Err(ConfigurationValidationError::from(format!(
                                "Invalid `{key}` — must be a boolean"
                            ))
                            .with_span(entry_span(nested_entry)));
                        }
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

    fn validate_rewrite_log_entry(
        &self,
        entry: &ferron_core::config::ServerConfigurationDirectiveEntry,
    ) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
        if entry.args.len() > 1 {
            return Err(ConfigurationValidationError::from(format!(
                "Invalid `rewrite_log` — must have zero or one value, got {}",
                entry.args.len()
            ))
            .with_span(entry_span(entry)));
        }

        if !entry.args.is_empty() {
            match &entry.args[0] {
                ServerConfigurationValue::Boolean(_, _) => Ok(()),
                _ => Err(ConfigurationValidationError::from(
                    "Invalid `rewrite_log` — must be a boolean",
                )
                .with_span(entry_span(entry))),
            }
        } else {
            Ok(())
        }
    }
}
