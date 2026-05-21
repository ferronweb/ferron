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
