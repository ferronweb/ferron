use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::config::{ServerConfigurationBlock, ServerConfigurationValue};

#[derive(Default)]
pub struct ReplaceConfigurationValidator;

impl ConfigurationValidator for ReplaceConfigurationValidator {
    fn validate_block(
        &self,
        config: &ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let used_directives = &mut ctx.used_directives;
        // Validate `replace` directives manually (complex structure with children)
        if let Some(entries) = config.directives.get("replace") {
            for entry in entries {
                if entry.args.len() < 2 {
                    return Err(Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "The `replace` directive must have at least two arguments: the searched string and the replacement string",
                    )));
                }

                if !matches!(&entry.args[0], ServerConfigurationValue::String(_, _)) {
                    return Err(Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "The searched string in `replace` must be a string",
                    )));
                }

                if !matches!(&entry.args[1], ServerConfigurationValue::String(_, _)) {
                    return Err(Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "The replacement string in `replace` must be a string",
                    )));
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
                                return Err(Box::new(std::io::Error::new(
                                    std::io::ErrorKind::InvalidInput,
                                    "The `once` option must have exactly one boolean argument",
                                )));
                            }
                            if !matches!(
                                &once_entry.args[0],
                                ServerConfigurationValue::Boolean(_, _)
                            ) {
                                return Err(Box::new(std::io::Error::new(
                                    std::io::ErrorKind::InvalidInput,
                                    "The `once` option must have a boolean argument",
                                )));
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
                    return Err(Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "The `replace_last_modified` directive must have exactly one boolean argument",
                    )));
                }
                if !matches!(&entry.args[0], ServerConfigurationValue::Boolean(_, _)) {
                    return Err(Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "The `replace_last_modified` directive must have a boolean argument",
                    )));
                }
            }
            used_directives.insert("replace_last_modified".to_string());
        }

        // Validate `replace_filter_types` directives
        if let Some(entries) = config.directives.get("replace_filter_types") {
            for entry in entries {
                if entry.args.is_empty() {
                    return Err(Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "The `replace_filter_types` directive must have at least one MIME type argument",
                    )));
                }
                for arg in &entry.args {
                    if !matches!(arg, ServerConfigurationValue::String(_, _)) {
                        return Err(Box::new(std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "Each MIME type in `replace_filter_types` must be a string",
                        )));
                    }
                }
            }
            used_directives.insert("replace_filter_types".to_string());
        }

        Ok(())
    }
}
