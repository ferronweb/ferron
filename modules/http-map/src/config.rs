//! Configuration parsing and evaluation for the `map` directive.
//!
//! Parses `map <source> <destination> { ... }` entries from layered
//! configuration into typed `MapRule` structures and evaluates them
//! at request time to set destination variables.

use fancy_regex::{Regex, RegexBuilder};
use ferron_core::config::layer::LayeredConfiguration;
use ferron_core::config::{ServerConfigurationBlock, ServerConfigurationDirectiveEntry, Variables};

/// A compiled mapping rule from configuration.
#[derive(Debug, Clone)]
pub struct MapRule {
    /// The source variable name (e.g., `request.uri.path`).
    pub source: String,
    /// The destination variable name (e.g., `category`).
    pub destination: String,
    /// Ordered mapping entries — evaluated in priority order at runtime.
    pub entries: Vec<MapEntry>,
    /// Fallback value when no entry matches.
    pub default: Option<String>,
}

/// A single mapping entry within a `map` block.
#[derive(Debug, Clone)]
pub enum MapEntry {
    /// Exact string match (no wildcards).
    Exact { key: String, value: String },
    /// Wildcard match — the pattern contains `*` converted to regex.
    Wildcard { regex: Regex, value: String },
    /// Regex match — compiled at parse time.
    Regex { regex: Regex, value: String },
}

/// Parse all `map` directives from the layered configuration and evaluate them
/// against the given context, populating destination variables.
///
/// Returns `true` if at least one map was evaluated, `false` otherwise.
pub fn evaluate_map_directives(
    config: &LayeredConfiguration,
    variables: &impl Variables,
) -> Vec<(String, String)> {
    let rules = parse_map_config(config);
    if rules.is_empty() {
        return Vec::new();
    }

    let mut results = Vec::new();

    for rule in rules {
        let source_value: String = resolve_source(&rule.source, variables).unwrap_or_default();

        let result_value = evaluate_entries(&source_value, &rule.entries, &rule.default);
        results.push((rule.destination, result_value));
    }

    results
}

/// Resolve the source variable from the context.
fn resolve_source(source: &str, variables: &impl Variables) -> Option<String> {
    variables.resolve(source)
}

/// Evaluate mapping entries against a source value, returning the matched result.
///
/// Priority order: exact match → wildcard match → regex match → default.
fn evaluate_entries(source: &str, entries: &[MapEntry], default: &Option<String>) -> String {
    // First pass: exact matches
    for entry in entries {
        if let MapEntry::Exact { key, value } = entry {
            if source == key {
                return value.clone();
            }
        }
    }

    // Second pass: wildcard matches (longest match wins)
    let mut best_wildcard: Option<&str> = None;
    let mut best_wildcard_len = 0usize;
    for entry in entries {
        if let MapEntry::Wildcard { regex, .. } = entry {
            if let Ok(true) = regex.is_match(source) {
                // Prefer the longest-matching wildcard
                let pattern_str = regex.as_str();
                // Approximate: use regex pattern length as proxy for specificity
                if pattern_str.len() > best_wildcard_len {
                    best_wildcard_len = pattern_str.len();
                    if let MapEntry::Wildcard { value, .. } = entry {
                        best_wildcard = Some(value);
                    }
                }
            }
        }
    }
    if let Some(value) = best_wildcard {
        return value.to_string();
    }

    // Third pass: regex matches (first match in declaration order wins)
    for entry in entries {
        if let MapEntry::Regex { regex, value } = entry {
            if let Ok(Some(captures)) = regex.captures(source) {
                let resolved = resolve_captures(value, &captures);
                return resolved;
            }
        }
    }

    // Fallback to default
    default.clone().unwrap_or_default()
}

/// Resolve capture group references ($1, $2, etc.) in the result value.
fn resolve_captures(value: &str, captures: &fancy_regex::Captures) -> String {
    let mut result = String::new();
    let mut chars = value.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '$' {
            let mut num_str = String::new();
            while let Some(&d) = chars.peek() {
                if d.is_ascii_digit() {
                    num_str.push(d);
                    chars.next();
                } else {
                    break;
                }
            }
            if let Ok(idx) = num_str.parse::<usize>() {
                if let Some(m) = captures.get(idx) {
                    result.push_str(m.as_str());
                } else {
                    // Capture group doesn't exist — keep reference literally
                    result.push('$');
                    result.push_str(&num_str);
                }
            } else {
                result.push('$');
                result.push_str(&num_str);
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// Parse all `map` directives from the layered configuration.
fn parse_map_config(config: &LayeredConfiguration) -> Vec<MapRule> {
    let mut rules = Vec::new();
    let entries = config.get_entries("map", true);

    for entry in entries {
        if let Some(rule) = parse_map_entry(entry) {
            rules.push(rule);
        }
    }

    rules
}

/// Parse a single `map` directive entry into a `MapRule`.
fn parse_map_entry(entry: &ServerConfigurationDirectiveEntry) -> Option<MapRule> {
    if entry.args.len() != 2 {
        return None;
    }

    let source = entry.args[0].as_str()?.to_string();
    let destination = entry.args[1].as_str()?.to_string();

    let block = entry.children.as_ref()?;
    let (entries, default) = parse_map_block(block);

    Some(MapRule {
        source,
        destination,
        entries,
        default,
    })
}

/// Parse the contents of a `map { ... }` block.
fn parse_map_block(block: &ServerConfigurationBlock) -> (Vec<MapEntry>, Option<String>) {
    let mut entries = Vec::new();
    let mut default = None;

    // Parse `default` directive
    if let Some(default_entries) = block.directives.get("default") {
        if let Some(entry) = default_entries.first() {
            if let Some(value) = entry.args.first().and_then(|v| v.as_str()) {
                default = Some(value.to_string());
            }
        }
    }

    // Parse `exact` directives
    if let Some(exact_entries) = block.directives.get("exact") {
        for entry in exact_entries {
            if entry.args.len() == 2 {
                if let (Some(key), Some(value)) = (entry.args[0].as_str(), entry.args[1].as_str()) {
                    if key.contains('*') {
                        // Treat as wildcard
                        if let Some(wildcard_regex) = wildcard_to_regex(key) {
                            if let Ok(regex) = Regex::new(&wildcard_regex) {
                                entries.push(MapEntry::Wildcard {
                                    regex,
                                    value: value.to_string(),
                                });
                            }
                        }
                    } else {
                        entries.push(MapEntry::Exact {
                            key: key.to_string(),
                            value: value.to_string(),
                        });
                    }
                }
            }
        }
    }

    // Parse `regex` directives
    if let Some(regex_entries) = block.directives.get("regex") {
        for entry in regex_entries {
            if entry.args.len() >= 2 {
                if let (Some(pattern), Some(value)) =
                    (entry.args[0].as_str(), entry.args[1].as_str())
                {
                    let case_insensitive = if let Some(ref children) = entry.children {
                        children
                            .directives
                            .get("case_insensitive")
                            .and_then(|e| e.first())
                            .and_then(|e| e.args.first())
                            .and_then(|v| v.as_boolean())
                            .unwrap_or(false)
                    } else {
                        false
                    };

                    let regex_result = RegexBuilder::new(pattern)
                        .case_insensitive(case_insensitive)
                        .build();

                    if let Ok(regex) = regex_result {
                        entries.push(MapEntry::Regex {
                            regex,
                            value: value.to_string(),
                        });
                    }
                }
            }
        }
    }

    (entries, default)
}

/// Convert a wildcard pattern (with `*`) to a regex string.
///
/// `*` is treated as "match any characters" (equivalent to `.*` in regex).
fn wildcard_to_regex(pattern: &str) -> Option<String> {
    if !pattern.contains('*') {
        return None;
    }

    // Escape regex special chars except `*`, then replace `*` with `.*`
    let mut result = String::new();
    for c in pattern.chars() {
        match c {
            '*' => result.push_str(".*"),
            '\\' | '.' | '+' | '?' | '^' | '$' | '(' | ')' | '[' | ']' | '{' | '}' | '|' => {
                result.push('\\');
                result.push(c);
            }
            _ => result.push(c),
        }
    }

    // Anchor to full match
    Some(format!("^{}$", result))
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

    fn make_default_entry(value: &str) -> ServerConfigurationDirectiveEntry {
        make_map_entry(vec![make_value_string(value)], None)
    }

    fn make_exact_entry(key: &str, value: &str) -> ServerConfigurationDirectiveEntry {
        make_map_entry(vec![make_value_string(key), make_value_string(value)], None)
    }

    fn make_regex_entry(pattern: &str, value: &str) -> ServerConfigurationDirectiveEntry {
        make_map_entry(
            vec![make_value_string(pattern), make_value_string(value)],
            None,
        )
    }

    fn make_regex_entry_case_insensitive(
        pattern: &str,
        value: &str,
    ) -> ServerConfigurationDirectiveEntry {
        let mut opts = HashMap::new();
        opts.insert(
            "case_insensitive".to_string(),
            vec![make_map_entry(vec![make_value_bool(true)], None)],
        );
        make_map_entry(
            vec![make_value_string(pattern), make_value_string(value)],
            Some(ServerConfigurationBlock {
                directives: Arc::new(opts),
                matchers: HashMap::new(),
                span: None,
            }),
        )
    }

    fn make_map_block(
        source: &str,
        destination: &str,
        default: Option<&str>,
        exact_entries: Vec<ServerConfigurationDirectiveEntry>,
        regex_entries: Vec<ServerConfigurationDirectiveEntry>,
    ) -> (LayeredConfiguration, String) {
        let mut directives = HashMap::new();

        if let Some(d) = default {
            directives.insert("default".to_string(), vec![make_default_entry(d)]);
        }

        if !exact_entries.is_empty() {
            directives.insert("exact".to_string(), exact_entries);
        }

        if !regex_entries.is_empty() {
            directives.insert("regex".to_string(), regex_entries);
        }

        let map_block = ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: HashMap::new(),
            span: None,
        };

        let mut top_directives = HashMap::new();
        top_directives.insert(
            "map".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_string(source), make_value_string(destination)],
                children: Some(map_block),
                span: None,
            }],
        );

        let mut config = LayeredConfiguration::new();
        config.layers.push(Arc::new(ServerConfigurationBlock {
            directives: Arc::new(top_directives),
            matchers: HashMap::new(),
            span: None,
        }));

        (config, destination.to_string())
    }

    fn make_variables(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn parses_exact_match() {
        let (config, _dest) = make_map_block(
            "request.uri.path",
            "category",
            Some("uncategorized"),
            vec![make_exact_entry("/api/users", "api")],
            vec![],
        );
        let rules = parse_map_config(&config);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].source, "request.uri.path");
        assert_eq!(rules[0].destination, "category");
        assert_eq!(rules[0].default.as_deref(), Some("uncategorized"));
        assert!(matches!(rules[0].entries[0], MapEntry::Exact { .. }));
    }

    #[test]
    fn parses_wildcard_match() {
        let (config, _) = make_map_block(
            "request.uri.path",
            "category",
            Some("uncategorized"),
            vec![make_exact_entry("/api/*", "api")],
            vec![],
        );
        let rules = parse_map_config(&config);
        assert!(matches!(rules[0].entries[0], MapEntry::Wildcard { .. }));
    }

    #[test]
    fn parses_regex_match() {
        let (config, _) = make_map_block(
            "request.uri.path",
            "user_id",
            None,
            vec![],
            vec![make_regex_entry("^/users/([0-9]+)", "$1")],
        );
        let rules = parse_map_config(&config);
        assert!(matches!(rules[0].entries[0], MapEntry::Regex { .. }));
    }

    #[test]
    fn evaluates_exact_match() {
        let (config, _) = make_map_block(
            "test_var",
            "result",
            Some("default_val"),
            vec![make_exact_entry("/api/users", "api")],
            vec![],
        );
        let vars = make_variables(&[("test_var", "/api/users")]);
        let results = evaluate_map_directives(&config, &vars);
        assert_eq!(results, vec![("result".to_string(), "api".to_string())]);
    }

    #[test]
    fn evaluates_wildcard_match() {
        let (config, _) = make_map_block(
            "test_var",
            "result",
            Some("default_val"),
            vec![make_exact_entry("/api/*", "api")],
            vec![],
        );
        let vars = make_variables(&[("test_var", "/api/users/123")]);
        let results = evaluate_map_directives(&config, &vars);
        assert_eq!(results, vec![("result".to_string(), "api".to_string())]);
    }

    #[test]
    fn evaluates_regex_match_with_captures() {
        let (config, _) = make_map_block(
            "test_var",
            "user_id",
            Some(""),
            vec![],
            vec![make_regex_entry("^/users/([0-9]+)", "$1")],
        );
        let vars = make_variables(&[("test_var", "/users/42")]);
        let results = evaluate_map_directives(&config, &vars);
        assert_eq!(results, vec![("user_id".to_string(), "42".to_string())]);
    }

    #[test]
    fn evaluates_default_fallback() {
        let (config, _) = make_map_block(
            "test_var",
            "result",
            Some("default_val"),
            vec![make_exact_entry("/api", "api")],
            vec![],
        );
        let vars = make_variables(&[("test_var", "/unknown")]);
        let results = evaluate_map_directives(&config, &vars);
        assert_eq!(
            results,
            vec![("result".to_string(), "default_val".to_string())]
        );
    }

    #[test]
    fn evaluates_default_empty_when_not_set() {
        let (config, _) = make_map_block(
            "test_var",
            "result",
            None,
            vec![make_exact_entry("/api", "api")],
            vec![],
        );
        let vars = make_variables(&[("test_var", "/unknown")]);
        let results = evaluate_map_directives(&config, &vars);
        assert_eq!(results, vec![("result".to_string(), String::new())]);
    }

    #[test]
    fn exact_match_beats_wildcard() {
        let (config, _) = make_map_block(
            "test_var",
            "result",
            Some("default_val"),
            vec![
                make_exact_entry("/api/*", "api_wildcard"),
                make_exact_entry("/api/users", "api_exact"),
            ],
            vec![],
        );
        let vars = make_variables(&[("test_var", "/api/users")]);
        let results = evaluate_map_directives(&config, &vars);
        assert_eq!(
            results,
            vec![("result".to_string(), "api_exact".to_string())]
        );
    }

    #[test]
    fn wildcard_beats_default() {
        let (config, _) = make_map_block(
            "test_var",
            "result",
            Some("default_val"),
            vec![make_exact_entry("/api/*", "api")],
            vec![],
        );
        let vars = make_variables(&[("test_var", "/api/test")]);
        let results = evaluate_map_directives(&config, &vars);
        assert_eq!(results, vec![("result".to_string(), "api".to_string())]);
    }

    #[test]
    fn regex_case_insensitive() {
        let (config, _) = make_map_block(
            "test_var",
            "result",
            Some("default_val"),
            vec![],
            vec![make_regex_entry_case_insensitive("^/API/.*", "api")],
        );
        let vars = make_variables(&[("test_var", "/api/users")]);
        let results = evaluate_map_directives(&config, &vars);
        assert_eq!(results, vec![("result".to_string(), "api".to_string())]);
    }

    #[test]
    fn resolve_captures_basic() {
        let re = Regex::new("^/users/([0-9]+)/posts/([0-9]+)$").unwrap();
        let captures = re.captures("/users/42/posts/7").unwrap().unwrap();
        assert_eq!(resolve_captures("$1", &captures), "42");
        assert_eq!(resolve_captures("$2", &captures), "7");
        assert_eq!(
            resolve_captures("user_$1_post_$2", &captures),
            "user_42_post_7"
        );
    }

    #[test]
    fn resolve_captures_missing_group() {
        let re = Regex::new("^/test$").unwrap();
        let captures = re.captures("/test").unwrap().unwrap();
        assert_eq!(resolve_captures("$1", &captures), "$1");
    }

    #[test]
    fn no_map_directives_returns_empty() {
        let config = LayeredConfiguration::new();
        let vars = make_variables(&[]);
        let results = evaluate_map_directives(&config, &vars);
        assert!(results.is_empty());
    }

    #[test]
    fn empty_source_variable_uses_default() {
        let (config, _) = make_map_block(
            "nonexistent_var",
            "result",
            Some("default_val"),
            vec![],
            vec![],
        );
        let vars = make_variables(&[]);
        let results = evaluate_map_directives(&config, &vars);
        assert_eq!(
            results,
            vec![("result".to_string(), "default_val".to_string())]
        );
    }

    #[test]
    fn multiple_map_entries_same_rule() {
        let (config, _) = make_map_block(
            "test_var",
            "result",
            Some("default_val"),
            vec![
                make_exact_entry("/api", "api"),
                make_exact_entry("/blog", "blog"),
            ],
            vec![],
        );
        let vars = make_variables(&[("test_var", "/blog")]);
        let results = evaluate_map_directives(&config, &vars);
        assert_eq!(results, vec![("result".to_string(), "blog".to_string())]);
    }
}
