//! Configuration validator for the http-replace module.
//!
//! Validates `replace`, `replace_last_modified`, and `replace_filter_types` directives.

use std::collections::HashSet;

use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::config::ServerConfigurationBlock;
use ferron_core::config::ServerConfigurationValue;

/// Validator for http-replace module directives.
#[derive(Default)]
pub struct ReplaceConfigurationValidator;

impl ConfigurationValidator for ReplaceConfigurationValidator {
    fn validate_block(
        &self,
        config: &ServerConfigurationBlock,
        used_directives: &mut HashSet<String>,
        _is_global: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
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
                    if let Some(once_entries) = children.directives.get("once") {
                        for once_entry in once_entries {
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
                }
            }
            used_directives.insert("replace".to_string());
        }

        // Validate `replace_last_modified` directives
        if let Some(entries) = config.directives.get("replace_last_modified") {
            for entry in entries {
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

#[cfg(test)]
mod tests {
    use super::*;
    use ferron_core::config::{ServerConfigurationDirectiveEntry, ServerConfigurationValue};
    use std::collections::HashMap;
    use std::sync::Arc;

    fn make_value_string(s: &str) -> ServerConfigurationValue {
        ServerConfigurationValue::String(s.to_string(), None)
    }

    fn make_value_bool(b: bool) -> ServerConfigurationValue {
        ServerConfigurationValue::Boolean(b, None)
    }

    fn make_config_block(
        directives: HashMap<String, Vec<ServerConfigurationDirectiveEntry>>,
    ) -> ServerConfigurationBlock {
        ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: HashMap::new(),
            span: None,
        }
    }

    #[test]
    fn validates_valid_replace_directive() {
        let mut directives = HashMap::new();
        directives.insert(
            "replace".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_string("old"), make_value_string("new")],
                children: None,
                span: None,
            }],
        );

        let config = make_config_block(directives);
        let mut used = HashSet::new();
        assert!(ReplaceConfigurationValidator
            .validate_block(&config, &mut used, false)
            .is_ok());
        assert!(used.contains("replace"));
    }

    #[test]
    fn validates_replace_with_once_option() {
        let mut child_directives = HashMap::new();
        child_directives.insert(
            "once".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_bool(true)],
                children: None,
                span: None,
            }],
        );

        let mut directives = HashMap::new();
        directives.insert(
            "replace".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_string("old"), make_value_string("new")],
                children: Some(ServerConfigurationBlock {
                    directives: Arc::new(child_directives),
                    matchers: HashMap::new(),
                    span: None,
                }),
                span: None,
            }],
        );

        let config = make_config_block(directives);
        let mut used = HashSet::new();
        assert!(ReplaceConfigurationValidator
            .validate_block(&config, &mut used, false)
            .is_ok());
    }

    #[test]
    fn rejects_replace_with_missing_arguments() {
        let mut directives = HashMap::new();
        directives.insert(
            "replace".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_string("only-search")],
                children: None,
                span: None,
            }],
        );

        let config = make_config_block(directives);
        let mut used = HashSet::new();
        let result = ReplaceConfigurationValidator.validate_block(&config, &mut used, false);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("must have at least two arguments"));
    }

    #[test]
    fn validates_replace_last_modified() {
        let mut directives = HashMap::new();
        directives.insert(
            "replace_last_modified".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_bool(true)],
                children: None,
                span: None,
            }],
        );

        let config = make_config_block(directives);
        let mut used = HashSet::new();
        assert!(ReplaceConfigurationValidator
            .validate_block(&config, &mut used, false)
            .is_ok());
        assert!(used.contains("replace_last_modified"));
    }

    #[test]
    fn rejects_replace_last_modified_with_invalid_args() {
        let mut directives = HashMap::new();
        directives.insert(
            "replace_last_modified".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_string("not-a-bool")],
                children: None,
                span: None,
            }],
        );

        let config = make_config_block(directives);
        let mut used = HashSet::new();
        let result = ReplaceConfigurationValidator.validate_block(&config, &mut used, false);
        assert!(result.is_err());
    }

    #[test]
    fn validates_replace_filter_types() {
        let mut directives = HashMap::new();
        directives.insert(
            "replace_filter_types".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![
                    make_value_string("text/html"),
                    make_value_string("text/css"),
                ],
                children: None,
                span: None,
            }],
        );

        let config = make_config_block(directives);
        let mut used = HashSet::new();
        assert!(ReplaceConfigurationValidator
            .validate_block(&config, &mut used, false)
            .is_ok());
        assert!(used.contains("replace_filter_types"));
    }

    #[test]
    fn rejects_replace_filter_types_with_empty_args() {
        let mut directives = HashMap::new();
        directives.insert(
            "replace_filter_types".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![],
                children: None,
                span: None,
            }],
        );

        let config = make_config_block(directives);
        let mut used = HashSet::new();
        let result = ReplaceConfigurationValidator.validate_block(&config, &mut used, false);
        assert!(result.is_err());
    }
}
