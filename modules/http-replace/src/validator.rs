use ferron_core::config::validator::{
    entry_span, ConfigurationValidationError, ConfigurationValidator,
};
use ferron_core::config::{ServerConfigurationBlock, ServerConfigurationValue};

#[derive(Default)]
pub struct ReplaceConfigurationValidator;

impl ConfigurationValidator for ReplaceConfigurationValidator {
    fn validate_block(
        &self,
        config: &ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
        let used_directives = &mut ctx.used_directives;
        // Validate `replace` directives manually (complex structure with children)
        if let Some(entries) = config.directives.get("replace") {
            for entry in entries {
                if entry.args.len() < 2 {
                    Err(ConfigurationValidationError::from(
                        "The `replace` directive must have at least two arguments: the searched string and the replacement string",
                    )
                    .with_span(entry_span(entry)))?;
                }

                if !matches!(&entry.args[0], ServerConfigurationValue::String(_, _)) {
                    Err(ConfigurationValidationError::from(
                        "The searched string in `replace` must be a string",
                    )
                    .with_span(entry_span(entry)))?;
                }

                if !matches!(&entry.args[1], ServerConfigurationValue::String(_, _)) {
                    Err(ConfigurationValidationError::from(
                        "The replacement string in `replace` must be a string",
                    )
                    .with_span(entry_span(entry)))?;
                }

                // Validate `once` option in child block
                if let Some(children) = &entry.children {
                    let mut sub = std::collections::HashSet::new();
                    if let Some(once_entries) = children.directives.get("once") {
                        sub.insert("once".to_string());
                        for once_entry in once_entries {
                            if once_entry.args.is_empty() {
                                continue;
                            }
                            if once_entry.args.len() != 1 {
                                Err(ConfigurationValidationError::from(
                                    "The `once` option must have exactly one boolean argument",
                                )
                                .with_span(entry_span(once_entry)))?;
                            }
                            if !matches!(
                                &once_entry.args[0],
                                ServerConfigurationValue::Boolean(_, _)
                            ) {
                                Err(ConfigurationValidationError::from(
                                    "The `once` option must have a boolean argument",
                                )
                                .with_span(entry_span(once_entry)))?;
                            }
                        }
                    }
                    ferron_core::check_unused_subdirectives!(
                        children,
                        sub,
                        &mut ctx.diagnostics,
                        ctx.scope.clone()
                    );
                }
            }
            used_directives.insert("replace".to_string());
        }

        // Validate `replace_last_modified` directives
        if let Some(entries) = config.directives.get("replace_last_modified") {
            for entry in entries {
                if entry.args.is_empty() {
                    continue;
                }
                if entry.args.len() != 1 {
                    Err(ConfigurationValidationError::from(
                        "The `replace_last_modified` directive must have exactly one boolean argument",
                    )
                    .with_span(entry_span(entry)))?;
                }
                if !matches!(&entry.args[0], ServerConfigurationValue::Boolean(_, _)) {
                    Err(ConfigurationValidationError::from(
                        "The `replace_last_modified` directive must have a boolean argument",
                    )
                    .with_span(entry_span(entry)))?;
                }
            }
            used_directives.insert("replace_last_modified".to_string());
        }

        // Validate `replace_filter_types` directives
        if let Some(entries) = config.directives.get("replace_filter_types") {
            for entry in entries {
                if entry.args.is_empty() {
                    Err(ConfigurationValidationError::from(
                        "The `replace_filter_types` directive must have at least one MIME type argument",
                    )
                    .with_span(entry_span(entry)))?;
                }
                for arg in &entry.args {
                    if !matches!(arg, ServerConfigurationValue::String(_, _)) {
                        Err(ConfigurationValidationError::from(
                            "Each MIME type in `replace_filter_types` must be a string",
                        )
                        .with_span(entry_span(entry)))?;
                    }
                }
            }
            used_directives.insert("replace_filter_types".to_string());
        }

        Ok(())
    }
}
