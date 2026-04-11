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
                if nested_entry.args.len() != 1 {
                    return Err(format!(
                        "Invalid `{key}` inside regex block — must have exactly one argument"
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

#[cfg(test)]
mod tests {
    use super::*;
    use ferron_core::config::ServerConfigurationDirectiveEntry;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn make_value_string(s: &str) -> ServerConfigurationValue {
        ServerConfigurationValue::String(s.to_string(), None)
    }

    fn make_value_bool(b: bool) -> ServerConfigurationValue {
        ServerConfigurationValue::Boolean(b, None)
    }

    fn make_map_entry(
        args: Vec<ServerConfigurationValue>,
        children: Option<ServerConfigurationBlock>,
    ) -> ServerConfigurationDirectiveEntry {
        ServerConfigurationDirectiveEntry {
            args,
            children,
            span: None,
        }
    }

    fn make_map_block(
        entries: Vec<(&str, Vec<ServerConfigurationDirectiveEntry>)>,
    ) -> ServerConfigurationBlock {
        let mut directives = HashMap::new();
        for (name, ents) in entries {
            directives.insert(name.to_string(), ents);
        }
        ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: HashMap::new(),
            span: None,
        }
    }

    fn make_full_map_entry(
        source: &str,
        destination: &str,
        block: ServerConfigurationBlock,
    ) -> ServerConfigurationDirectiveEntry {
        make_map_entry(
            vec![make_value_string(source), make_value_string(destination)],
            Some(block),
        )
    }

    #[test]
    fn valid_map_with_default_and_exact() {
        let block = make_map_block(vec![
            (
                "default",
                vec![make_map_entry(vec![make_value_string("default")], None)],
            ),
            (
                "exact",
                vec![make_map_entry(
                    vec![make_value_string("/api"), make_value_string("api")],
                    None,
                )],
            ),
        ]);
        let entry = make_full_map_entry("request.uri.path", "category", block);
        let parent = ServerConfigurationBlock {
            directives: Arc::new([("map".to_string(), vec![entry])].into_iter().collect()),
            matchers: HashMap::new(),
            span: None,
        };
        let mut used = HashSet::new();
        let validator = MapValidator;
        assert!(validator.validate_block(&parent, &mut used, false).is_ok());
        assert!(used.contains("map"));
    }

    #[test]
    fn valid_map_with_regex() {
        let block = make_map_block(vec![(
            "regex",
            vec![make_map_entry(
                vec![
                    make_value_string("^/users/([0-9]+)"),
                    make_value_string("$1"),
                ],
                None,
            )],
        )]);
        let entry = make_full_map_entry("request.uri.path", "user_id", block);
        let parent = ServerConfigurationBlock {
            directives: Arc::new([("map".to_string(), vec![entry])].into_iter().collect()),
            matchers: HashMap::new(),
            span: None,
        };
        let mut used = HashSet::new();
        let validator = MapValidator;
        assert!(validator.validate_block(&parent, &mut used, false).is_ok());
    }

    #[test]
    fn valid_map_with_regex_case_insensitive_option() {
        let mut opts = HashMap::new();
        opts.insert(
            "case_insensitive".to_string(),
            vec![make_map_entry(vec![make_value_bool(true)], None)],
        );
        let regex_entry = make_map_entry(
            vec![make_value_string("^/api/.*"), make_value_string("api")],
            Some(ServerConfigurationBlock {
                directives: Arc::new(opts),
                matchers: HashMap::new(),
                span: None,
            }),
        );
        let block = make_map_block(vec![("regex", vec![regex_entry])]);
        let entry = make_full_map_entry("request.uri.path", "category", block);
        let parent = ServerConfigurationBlock {
            directives: Arc::new([("map".to_string(), vec![entry])].into_iter().collect()),
            matchers: HashMap::new(),
            span: None,
        };
        let mut used = HashSet::new();
        let validator = MapValidator;
        assert!(validator.validate_block(&parent, &mut used, false).is_ok());
    }

    #[test]
    fn rejects_map_without_child_block() {
        let entry = make_map_entry(
            vec![
                make_value_string("request.uri.path"),
                make_value_string("category"),
            ],
            None,
        );
        let parent = ServerConfigurationBlock {
            directives: Arc::new([("map".to_string(), vec![entry])].into_iter().collect()),
            matchers: HashMap::new(),
            span: None,
        };
        let mut used = HashSet::new();
        let validator = MapValidator;
        assert!(validator.validate_block(&parent, &mut used, false).is_err());
    }

    #[test]
    fn rejects_map_with_wrong_arg_count() {
        let entry = make_map_entry(vec![make_value_string("request.uri.path")], None);
        let parent = ServerConfigurationBlock {
            directives: Arc::new([("map".to_string(), vec![entry])].into_iter().collect()),
            matchers: HashMap::new(),
            span: None,
        };
        let mut used = HashSet::new();
        let validator = MapValidator;
        assert!(validator.validate_block(&parent, &mut used, false).is_err());
    }

    #[test]
    fn rejects_unknown_sub_directive_in_map_block() {
        let block = make_map_block(vec![(
            "unknown",
            vec![make_map_entry(vec![make_value_string("val")], None)],
        )]);
        let entry = make_full_map_entry("request.uri.path", "category", block);
        let parent = ServerConfigurationBlock {
            directives: Arc::new([("map".to_string(), vec![entry])].into_iter().collect()),
            matchers: HashMap::new(),
            span: None,
        };
        let mut used = HashSet::new();
        let validator = MapValidator;
        let err = validator
            .validate_block(&parent, &mut used, false)
            .unwrap_err();
        assert!(err.to_string().contains("unknown"));
    }

    #[test]
    fn rejects_invalid_regex_pattern() {
        let block = make_map_block(vec![(
            "regex",
            vec![make_map_entry(
                vec![make_value_string("[invalid"), make_value_string("val")],
                None,
            )],
        )]);
        let entry = make_full_map_entry("request.uri.path", "category", block);
        let parent = ServerConfigurationBlock {
            directives: Arc::new([("map".to_string(), vec![entry])].into_iter().collect()),
            matchers: HashMap::new(),
            span: None,
        };
        let mut used = HashSet::new();
        let validator = MapValidator;
        assert!(validator.validate_block(&parent, &mut used, false).is_err());
    }

    #[test]
    fn rejects_unknown_option_in_regex_block() {
        let mut opts = HashMap::new();
        opts.insert(
            "unknown_option".to_string(),
            vec![make_map_entry(vec![make_value_bool(true)], None)],
        );
        let regex_entry = make_map_entry(
            vec![make_value_string("."), make_value_string("val")],
            Some(ServerConfigurationBlock {
                directives: Arc::new(opts),
                matchers: HashMap::new(),
                span: None,
            }),
        );
        let block = make_map_block(vec![("regex", vec![regex_entry])]);
        let entry = make_full_map_entry("request.uri.path", "category", block);
        let parent = ServerConfigurationBlock {
            directives: Arc::new([("map".to_string(), vec![entry])].into_iter().collect()),
            matchers: HashMap::new(),
            span: None,
        };
        let mut used = HashSet::new();
        let validator = MapValidator;
        let err = validator
            .validate_block(&parent, &mut used, false)
            .unwrap_err();
        assert!(err.to_string().contains("unknown_option"));
    }

    #[test]
    fn rejects_non_boolean_option_in_regex_block() {
        let mut opts = HashMap::new();
        opts.insert(
            "case_insensitive".to_string(),
            vec![make_map_entry(vec![make_value_string("yes")], None)],
        );
        let regex_entry = make_map_entry(
            vec![make_value_string("."), make_value_string("val")],
            Some(ServerConfigurationBlock {
                directives: Arc::new(opts),
                matchers: HashMap::new(),
                span: None,
            }),
        );
        let block = make_map_block(vec![("regex", vec![regex_entry])]);
        let entry = make_full_map_entry("request.uri.path", "category", block);
        let parent = ServerConfigurationBlock {
            directives: Arc::new([("map".to_string(), vec![entry])].into_iter().collect()),
            matchers: HashMap::new(),
            span: None,
        };
        let mut used = HashSet::new();
        let validator = MapValidator;
        let err = validator
            .validate_block(&parent, &mut used, false)
            .unwrap_err();
        assert!(err.to_string().contains("boolean"));
    }

    #[test]
    fn rejects_exact_with_wrong_arg_count() {
        let block = make_map_block(vec![(
            "exact",
            vec![make_map_entry(vec![make_value_string("/api")], None)],
        )]);
        let entry = make_full_map_entry("request.uri.path", "category", block);
        let parent = ServerConfigurationBlock {
            directives: Arc::new([("map".to_string(), vec![entry])].into_iter().collect()),
            matchers: HashMap::new(),
            span: None,
        };
        let mut used = HashSet::new();
        let validator = MapValidator;
        assert!(validator.validate_block(&parent, &mut used, false).is_err());
    }

    #[test]
    fn skips_block_without_map() {
        let block = ServerConfigurationBlock {
            directives: Arc::new(HashMap::new()),
            matchers: HashMap::new(),
            span: None,
        };
        let mut used = HashSet::new();
        let validator = MapValidator;
        assert!(validator.validate_block(&block, &mut used, false).is_ok());
    }
}
