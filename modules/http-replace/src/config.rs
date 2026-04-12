//! Configuration parsing for `replace`, `replace_last_modified`, and `replace_filter_types` directives.

use ferron_core::config::layer::LayeredConfiguration;
use ferron_http::HttpContext;

/// A single replace rule for string replacement in response bodies.
pub struct ReplaceRule {
    /// The byte sequence to search for.
    pub searched: Vec<u8>,
    /// The byte sequence to replace with.
    pub replacement: Vec<u8>,
    /// Whether to replace only the first occurrence.
    pub once: bool,
}

/// Parsed configuration for the http-replace module.
pub struct ReplaceConfig {
    /// List of replace rules to apply in order.
    pub rules: Vec<ReplaceRule>,
    /// Whether to preserve the Last-Modified header.
    pub preserve_last_modified: bool,
    /// MIME types to process for replacement.
    pub filter_types: Vec<String>,
}

impl Default for ReplaceConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl ReplaceConfig {
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
            preserve_last_modified: false,
            filter_types: vec!["text/html".to_string()],
        }
    }

    /// Parse all http-replace directives from the layered configuration.
    #[allow(dead_code)]
    pub fn from_config(config: &LayeredConfiguration) -> Self {
        let rules = parse_replace_rules(config);
        let preserve_last_modified = parse_replace_last_modified(config);
        let filter_types = parse_replace_filter_types(config);

        Self {
            rules,
            preserve_last_modified,
            filter_types,
        }
    }

    pub fn from_http_context(ctx: &HttpContext) -> Self {
        let config = &ctx.configuration;
        let rules = parse_replace_rules(config);
        let preserve_last_modified = parse_replace_last_modified(config);
        let filter_types = parse_replace_filter_types(config);

        Self {
            rules,
            preserve_last_modified,
            filter_types,
        }
    }
}

/// Parse `replace` directives from configuration.
fn parse_replace_rules(config: &LayeredConfiguration) -> Vec<ReplaceRule> {
    let mut rules = Vec::new();
    let entries = config.get_entries("replace", true);

    for entry in &entries {
        // Need at least 2 arguments: searched and replacement
        if entry.args.len() < 2 {
            continue;
        }

        let searched = match entry.args.first().and_then(|v| v.as_str()) {
            Some(s) => s.as_bytes().to_vec(),
            None => continue,
        };

        let replacement = match entry.args.get(1).and_then(|v| v.as_str()) {
            Some(s) => s.as_bytes().to_vec(),
            None => continue,
        };

        // Check for `once` option in child block
        let once = if let Some(children) = &entry.children {
            children
                .get_value("once")
                .and_then(|v| v.as_boolean())
                .unwrap_or(false)
        } else {
            false
        };

        rules.push(ReplaceRule {
            searched,
            replacement,
            once,
        });
    }

    rules
}

/// Parse `replace_last_modified` directive.
fn parse_replace_last_modified(config: &LayeredConfiguration) -> bool {
    let entries = config.get_entries("replace_last_modified", true);
    for entry in &entries {
        if let Some(value) = entry.args.first().and_then(|v| v.as_boolean()) {
            return value;
        }
    }
    false
}

/// Parse `replace_filter_types` directive.
fn parse_replace_filter_types(config: &LayeredConfiguration) -> Vec<String> {
    let mut filter_types = Vec::new();
    let entries = config.get_entries("replace_filter_types", true);

    for entry in &entries {
        for arg in &entry.args {
            if let Some(mime_type) = arg.as_str() {
                filter_types.push(mime_type.to_string());
            }
        }
    }

    // Default to text/html if no filter types specified
    if filter_types.is_empty() {
        filter_types.push("text/html".to_string());
    }

    filter_types
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferron_core::config::{
        ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationValue,
    };
    use std::collections::HashMap;
    use std::sync::Arc;

    fn make_value_string(s: &str) -> ServerConfigurationValue {
        ServerConfigurationValue::String(s.to_string(), None)
    }

    fn make_value_bool(b: bool) -> ServerConfigurationValue {
        ServerConfigurationValue::Boolean(b, None)
    }

    fn make_config_with_directives(
        directives: HashMap<String, Vec<ServerConfigurationDirectiveEntry>>,
    ) -> LayeredConfiguration {
        let mut config = LayeredConfiguration::new();
        config.layers.push(Arc::new(ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: HashMap::new(),
            span: None,
        }));
        config
    }

    fn make_replace_entry(
        searched: &str,
        replacement: &str,
        once: Option<bool>,
    ) -> ServerConfigurationDirectiveEntry {
        let children = once.map(|once_value| {
            let mut d = HashMap::new();
            d.insert(
                "once".to_string(),
                vec![ServerConfigurationDirectiveEntry {
                    args: vec![make_value_bool(once_value)],
                    children: None,
                    span: None,
                }],
            );
            ServerConfigurationBlock {
                directives: Arc::new(d),
                matchers: HashMap::new(),
                span: None,
            }
        });

        ServerConfigurationDirectiveEntry {
            args: vec![make_value_string(searched), make_value_string(replacement)],
            children,
            span: None,
        }
    }

    #[test]
    fn parses_single_replace_rule() {
        let mut directives = HashMap::new();
        directives.insert(
            "replace".to_string(),
            vec![make_replace_entry("old", "new", None)],
        );

        let config = make_config_with_directives(directives);
        let rules = parse_replace_rules(&config);

        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].searched, b"old");
        assert_eq!(rules[0].replacement, b"new");
        assert!(!rules[0].once);
    }

    #[test]
    fn parses_multiple_replace_rules() {
        let mut directives = HashMap::new();
        directives.insert(
            "replace".to_string(),
            vec![
                make_replace_entry("foo", "bar", None),
                make_replace_entry("baz", "qux", Some(true)),
            ],
        );

        let config = make_config_with_directives(directives);
        let rules = parse_replace_rules(&config);

        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].searched, b"foo");
        assert!(!rules[0].once);
        assert_eq!(rules[1].searched, b"baz");
        assert!(rules[1].once);
    }

    #[test]
    fn skips_invalid_replace_rules() {
        let mut directives = HashMap::new();
        directives.insert(
            "replace".to_string(),
            vec![
                // Missing replacement argument
                ServerConfigurationDirectiveEntry {
                    args: vec![make_value_string("only-search")],
                    children: None,
                    span: None,
                },
                // Valid rule
                make_replace_entry("valid", "replacement", None),
            ],
        );

        let config = make_config_with_directives(directives);
        let rules = parse_replace_rules(&config);

        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].searched, b"valid");
    }

    #[test]
    fn parses_replace_last_modified_true() {
        let mut directives = HashMap::new();
        directives.insert(
            "replace_last_modified".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_bool(true)],
                children: None,
                span: None,
            }],
        );

        let config = make_config_with_directives(directives);
        assert!(parse_replace_last_modified(&config));
    }

    #[test]
    fn parses_replace_last_modified_false() {
        let mut directives = HashMap::new();
        directives.insert(
            "replace_last_modified".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_bool(false)],
                children: None,
                span: None,
            }],
        );

        let config = make_config_with_directives(directives);
        assert!(!parse_replace_last_modified(&config));
    }

    #[test]
    fn parses_replace_last_modified_default() {
        let directives = HashMap::new();
        let config = make_config_with_directives(directives);
        assert!(!parse_replace_last_modified(&config));
    }

    #[test]
    fn parses_replace_filter_types() {
        let mut directives = HashMap::new();
        directives.insert(
            "replace_filter_types".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![
                    make_value_string("text/html"),
                    make_value_string("text/css"),
                    make_value_string("application/javascript"),
                ],
                children: None,
                span: None,
            }],
        );

        let config = make_config_with_directives(directives);
        let filter_types = parse_replace_filter_types(&config);

        assert_eq!(filter_types.len(), 3);
        assert_eq!(filter_types[0], "text/html");
        assert_eq!(filter_types[1], "text/css");
        assert_eq!(filter_types[2], "application/javascript");
    }

    #[test]
    fn parses_replace_filter_types_default() {
        let directives = HashMap::new();
        let config = make_config_with_directives(directives);
        let filter_types = parse_replace_filter_types(&config);

        assert_eq!(filter_types.len(), 1);
        assert_eq!(filter_types[0], "text/html");
    }

    #[test]
    fn parses_complete_replace_config() {
        let mut directives = HashMap::new();
        directives.insert(
            "replace".to_string(),
            vec![
                make_replace_entry("old", "new", Some(false)),
                make_replace_entry("foo", "bar", Some(true)),
            ],
        );
        directives.insert(
            "replace_last_modified".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_bool(true)],
                children: None,
                span: None,
            }],
        );
        directives.insert(
            "replace_filter_types".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![
                    make_value_string("text/html"),
                    make_value_string("application/json"),
                ],
                children: None,
                span: None,
            }],
        );

        let config = make_config_with_directives(directives);
        let replace_config = ReplaceConfig::from_config(&config);

        assert_eq!(replace_config.rules.len(), 2);
        assert!(replace_config.preserve_last_modified);
        assert_eq!(replace_config.filter_types.len(), 2);
        assert_eq!(replace_config.filter_types[0], "text/html");
        assert_eq!(replace_config.filter_types[1], "application/json");
    }
}
