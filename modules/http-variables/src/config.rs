//! Configuration parsing for the `set_var` and `log_field` directives.

use fancy_regex::{Regex, RegexBuilder};
use ferron_core::config::layer::LayeredConfiguration;
use ferron_core::config::{
    ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationValue,
    Variables,
};

/// A compiled `set_var` rule from configuration.
#[derive(Debug, Clone)]
pub struct SetVarRule {
    /// The source variable name (e.g., `request.uri.path`).
    pub source: String,
    /// Pre-compiled regex pattern to match against the source value.
    pub pattern: Regex,
    /// The destination variable name to set on match.
    pub variable: String,
    /// Value to assign when the pattern matches (default: `"1"`).
    pub value: String,
    /// When `true`, sets the variable when the pattern does **not** match.
    pub negate: bool,
}

/// A parsed `log_field` rule from configuration.
#[derive(Debug, Clone)]
pub struct LogFieldRule {
    /// The custom access log field name.
    pub field_name: String,
    /// The source value — either a plain variable name or an interpolated string.
    pub source: LogFieldSource,
}

/// How the source value for a `log_field` directive is resolved.
#[derive(Debug, Clone)]
pub enum LogFieldSource {
    /// Resolve a variable by name (e.g., `network_type`).
    Variable(String),
    /// Resolve an interpolated string at runtime (e.g., `"{{request.header.x_custom_header}}"`).
    Interpolated(Vec<ferron_core::config::ServerConfigurationInterpolatedStringPart>),
}

/// Parse all `set_var` directives from the layered configuration.
pub fn parse_set_var_rules(config: &LayeredConfiguration) -> Vec<SetVarRule> {
    let mut rules = Vec::new();
    for entry in config.get_entries("set_var", true) {
        if let Some(rule) = parse_set_var_entry(entry) {
            rules.push(rule);
        }
    }
    rules
}

/// Parse all `log_field` directives from the layered configuration.
pub fn parse_log_field_rules(config: &LayeredConfiguration) -> Vec<LogFieldRule> {
    let mut rules = Vec::new();
    for entry in config.get_entries("log_field", true) {
        if let Some(rule) = parse_log_field_entry(entry) {
            rules.push(rule);
        }
    }
    rules
}

/// Parse a single `log_field` entry into a `LogFieldRule`.
fn parse_log_field_entry(entry: &ServerConfigurationDirectiveEntry) -> Option<LogFieldRule> {
    if entry.args.len() != 2 {
        return None;
    }

    let field_name = entry.args[0].as_str()?.to_string();
    let source = match &entry.args[1] {
        ServerConfigurationValue::String(s, _) => LogFieldSource::Variable(s.clone()),
        ServerConfigurationValue::InterpolatedString(parts, _) => {
            LogFieldSource::Interpolated(parts.clone())
        }
        _ => return None,
    };

    Some(LogFieldRule { field_name, source })
}

/// Parse a single `set_var` entry into a `SetVarRule`.
fn parse_set_var_entry(entry: &ServerConfigurationDirectiveEntry) -> Option<SetVarRule> {
    // Inline form: set_var source regex variable
    // Block form:  set_var source regex variable { ... }
    if entry.args.len() != 3 {
        return None;
    }

    let source = entry.args[0].as_str()?.to_string();
    let pattern_str = entry.args[1].as_str()?.to_string();
    let variable = entry.args[2].as_str()?.to_string();

    // Parse optional block subdirectives
    let mut value = "1".to_string();
    let mut case_insensitive = false;
    let mut negate = false;

    if let Some(children) = &entry.children {
        parse_set_var_block(children, &mut value, &mut case_insensitive, &mut negate);
    }

    let pattern = RegexBuilder::new(&pattern_str)
        .case_insensitive(case_insensitive)
        .build()
        .ok()?;

    Some(SetVarRule {
        source,
        pattern,
        variable,
        value,
        negate,
    })
}

/// Parse the optional block inside a `set_var` directive.
fn parse_set_var_block(
    block: &ServerConfigurationBlock,
    value: &mut String,
    case_insensitive: &mut bool,
    negate: &mut bool,
) {
    if let Some(entries) = block.directives.get("value") {
        if let Some(entry) = entries.first() {
            if let Some(v) = entry.args.first().and_then(|v| v.as_str()) {
                *value = v.to_string();
            }
        }
    }
    if let Some(entries) = block.directives.get("case_insensitive") {
        if let Some(entry) = entries.first() {
            *case_insensitive = entry
                .args
                .first()
                .and_then(|v| v.as_boolean())
                .unwrap_or(false);
        }
    }
    if let Some(entries) = block.directives.get("negate") {
        if let Some(entry) = entries.first() {
            *negate = entry
                .args
                .first()
                .and_then(|v| v.as_boolean())
                .unwrap_or(false);
        }
    }
}

/// Evaluate all `set_var` rules against the given context, returning
/// `(variable_name, value)` pairs for variables that should be set.
pub fn evaluate_set_var_rules(
    rules: &[SetVarRule],
    variables: &impl Variables,
) -> Vec<(String, String)> {
    let mut results = Vec::new();
    for rule in rules {
        let source_value = variables.resolve(&rule.source).unwrap_or_default();
        let matched = rule.pattern.is_match(&source_value).unwrap_or(false);
        if rule.negate != matched {
            results.push((rule.variable.clone(), rule.value.clone()));
        }
    }
    results
}
