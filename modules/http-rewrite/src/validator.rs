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
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Validate `rewrite` directive
        if let Some(entries) = config.directives.get("rewrite") {
            ctx.used_directives.insert("rewrite".to_string());
            for entry in entries {
                self.validate_rewrite_entry(entry, ctx)?;
            }
        }

        // Validate `rewrite_log` directive
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
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Must have exactly 2 positional arguments
        if entry.args.len() != 2 {
            return Err(format!(
                "Invalid `rewrite` — must have exactly two arguments (regex and replacement), got {}",
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
            return Err("Invalid `rewrite` — the regular expression must be a string".into());
        }

        // Second arg must be a string (replacement)
        if !matches!(
            &entry.args[1],
            ServerConfigurationValue::String(_, _)
                | ServerConfigurationValue::InterpolatedString(_, _)
        ) {
            return Err("Invalid `rewrite` — the replacement must be a string".into());
        }

        // Validate optional block
        if let Some(ref children) = entry.children {
            self.validate_rewrite_block_options(children, ctx)?;
        }

        Ok(())
    }

    fn validate_rewrite_block_options(
        &self,
        children: &ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut sub = std::collections::HashSet::new();
        for (key, nested_entries) in children.directives.iter() {
            if !RECOGNIZED_OPTIONS.contains(&key.as_str()) {
                return Err(format!(
                    "Invalid `rewrite` — unknown option `{key}` in rewrite block (recognized options: {})",
                    RECOGNIZED_OPTIONS.join(", ")
                )
                .into());
            }
            sub.insert(key.clone());
            for nested_entry in nested_entries {
                if nested_entry.args.len() > 1 {
                    return Err(format!("Invalid `{key}` — must have at most one value").into());
                }
                if !nested_entry.args.is_empty() {
                    match &nested_entry.args[0] {
                        ServerConfigurationValue::Boolean(_, _) => {}
                        _ => {
                            return Err(format!("Invalid `{key}` — must be a boolean").into());
                        }
                    }
                }
            }
        }
        ferron_core::check_unused_subdirectives!(children, sub, &mut ctx.diagnostics, ctx.scope.clone());
        Ok(())
    }

    fn validate_rewrite_log_entry(
        &self,
        entry: &ferron_core::config::ServerConfigurationDirectiveEntry,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if entry.args.len() > 1 {
            return Err(format!(
                "Invalid `rewrite_log` — must have zero or one value, got {}",
                entry.args.len()
            )
            .into());
        }

        if !entry.args.is_empty() {
            match &entry.args[0] {
                ServerConfigurationValue::Boolean(_, _) => Ok(()),
                _ => Err("Invalid `rewrite_log` — must be a boolean".into()),
            }
        } else {
            Ok(())
        }
    }
}
