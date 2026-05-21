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
