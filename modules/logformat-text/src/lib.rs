use std::collections::HashMap;
use std::sync::Arc;

use chrono::Local;
use ferron_core::{loader::ModuleLoader, providers::Provider};
use ferron_observability::{AccessVisitor, LogFormatterContext};
use once_cell::sync::Lazy;

// Default Combined Log Format pattern
const DEFAULT_CLF_PATTERN: &str =
    "%client_ip - %auth_user [%t] \"%method %path_and_query %version\" %status %content_length \"%{Referer}i\" \"%{User-Agent}i\"";

/// Represents a parsed format token
#[derive(Debug, Clone, PartialEq)]
enum FormatToken {
    /// %field_name - Access log field
    Field(String),
    /// %{Header-Name}i - Request header
    Header(String),
    /// %{format}t or %t - Timestamp
    Timestamp(Option<String>),
    /// Literal text (including escaped %%)
    Literal(String),
}

/// Parsed and compiled format pattern
#[derive(Debug, Clone)]
struct FormatPattern {
    tokens: Vec<FormatToken>,
}

impl FormatPattern {
    /// Parse a format pattern string into tokens
    fn parse(pattern: &str) -> Self {
        let tokens = Self::tokenize(pattern);
        Self { tokens }
    }

    fn tokenize(pattern: &str) -> Vec<FormatToken> {
        let mut tokens = Vec::new();
        let mut chars = pattern.chars().peekable();
        let mut literal_buf = String::new();

        while let Some(c) = chars.next() {
            if c == '%' {
                // Flush literal buffer
                if !literal_buf.is_empty() {
                    tokens.push(FormatToken::Literal(std::mem::take(&mut literal_buf)));
                }

                match chars.peek() {
                    Some(&'%') => {
                        // Escaped percent
                        chars.next();
                        literal_buf.push('%');
                    }
                    Some(&'{') => {
                        // %{...}x style token
                        chars.next(); // consume '{'
                        let mut inner = String::new();
                        loop {
                            match chars.next() {
                                Some('}') => break,
                                Some(c) => inner.push(c),
                                None => {
                                    // Unterminated, treat as literal
                                    literal_buf.push_str("%{");
                                    literal_buf.push_str(&inner);
                                    break;
                                }
                            }
                        }

                        match chars.peek() {
                            Some(&'i') => {
                                // Header: %{Header-Name}i
                                chars.next();
                                tokens.push(FormatToken::Header(inner));
                            }
                            Some(&'t') => {
                                // Timestamp with format: %{format}t
                                chars.next();
                                tokens.push(FormatToken::Timestamp(Some(inner)));
                            }
                            _ => {
                                // Unknown, treat as literal
                                literal_buf.push_str("%{");
                                literal_buf.push_str(&inner);
                            }
                        }
                    }
                    Some(&'t') => {
                        // Timestamp without format: %t
                        chars.next();
                        tokens.push(FormatToken::Timestamp(None));
                    }
                    Some(&c) if c.is_alphabetic() || c == '_' => {
                        // Field name: %field_name
                        let mut field_name = String::new();
                        field_name.push(c);
                        chars.next();
                        while let Some(&nc) = chars.peek() {
                            if nc.is_alphanumeric() || nc == '_' {
                                field_name.push(nc);
                                chars.next();
                            } else {
                                break;
                            }
                        }
                        tokens.push(FormatToken::Field(field_name));
                    }
                    _ => {
                        // Unknown or end, treat as literal
                        literal_buf.push('%');
                    }
                }
            } else {
                literal_buf.push(c);
            }
        }

        // Flush remaining literal
        if !literal_buf.is_empty() {
            tokens.push(FormatToken::Literal(literal_buf));
        }

        tokens
    }

    /// Format the pattern using the provided field values
    fn format(&self, fields: &HashMap<String, String>, timestamp_format: Option<&str>) -> String {
        let mut output = String::with_capacity(256);

        for token in &self.tokens {
            match token {
                FormatToken::Field(name) => {
                    let value = fields.get(name.as_str()).map(|s| s.as_str()).unwrap_or("-");
                    output.push_str(value);
                }
                FormatToken::Header(name) => {
                    let key = format!("header_{}", name.to_ascii_lowercase().replace("-", "_"));
                    let value = fields.get(key.as_str()).map(|s| s.as_str()).unwrap_or("-");
                    output.push_str(value);
                }
                FormatToken::Timestamp(Some(format)) => {
                    let now = Local::now();
                    output.push_str(&now.format(format).to_string());
                }
                FormatToken::Timestamp(None) => {
                    // Use configured timestamp_format or CLF default
                    let fmt = timestamp_format.unwrap_or("%d/%b/%Y:%H:%M:%S %z");
                    let now = Local::now();
                    output.push_str(&now.format(fmt).to_string());
                }
                FormatToken::Literal(text) => {
                    output.push_str(text);
                }
            }
        }

        output
    }
}

/// Parse configuration values from the log config block
fn parse_config(
    log_config: &ferron_core::config::ServerConfigurationBlock,
) -> (FormatPattern, Option<String>, Arc<Vec<String>>) {
    // Parse access_pattern or use default CLF
    let pattern_str = log_config
        .directives
        .get("access_pattern")
        .and_then(|entries| entries.first())
        .and_then(|entry| entry.args.first())
        .and_then(|arg| arg.as_string_with_interpolations(&std::collections::HashMap::new()))
        .unwrap_or_else(|| DEFAULT_CLF_PATTERN.to_string());

    // Parse timestamp_format
    let timestamp_format = log_config
        .directives
        .get("timestamp_format")
        .and_then(|entries| entries.first())
        .and_then(|entry| entry.args.first())
        .and_then(|arg| arg.as_string_with_interpolations(&std::collections::HashMap::new()));

    // Parse enabled fields (optional field filtering like JSON module)
    let enabled_fields: Arc<Vec<String>> = log_config
        .directives
        .get("fields")
        .map(|entries| {
            entries
                .iter()
                .flat_map(|entry| entry.args.iter())
                .filter_map(|arg| {
                    arg.as_string_with_interpolations(&std::collections::HashMap::new())
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
        .into();

    let pattern = get_or_compile_pattern(&pattern_str);
    (pattern, timestamp_format, enabled_fields)
}

/// Cache for compiled format patterns to avoid re-parsing
static PATTERN_CACHE: Lazy<std::sync::Mutex<HashMap<String, FormatPattern>>> =
    Lazy::new(|| std::sync::Mutex::new(HashMap::new()));

fn get_or_compile_pattern(pattern_str: &str) -> FormatPattern {
    let mut cache = PATTERN_CACHE.lock().unwrap();
    if let Some(pattern) = cache.get(pattern_str) {
        return pattern.clone();
    }
    let pattern = FormatPattern::parse(pattern_str);
    cache.insert(pattern_str.to_string(), pattern.clone());
    pattern
}

/// Text visitor that collects fields into a HashMap
struct TextVisitor {
    fields: HashMap<String, String>,
    enabled_fields: Arc<Vec<String>>,
}

impl TextVisitor {
    fn new(enabled_fields: Arc<Vec<String>>) -> Self {
        Self {
            fields: HashMap::new(),
            enabled_fields,
        }
    }

    fn is_enabled(&self, name: &str) -> bool {
        self.enabled_fields.is_empty() || self.enabled_fields.iter().any(|f| f == name)
    }
}

impl AccessVisitor for TextVisitor {
    fn field_string(&mut self, name: &str, value: &str) {
        if self.is_enabled(name) {
            self.fields.insert(name.to_string(), value.to_string());
        }
    }

    fn field_u64(&mut self, name: &str, value: u64) {
        if self.is_enabled(name) {
            self.fields.insert(name.to_string(), value.to_string());
        }
    }

    fn field_f64(&mut self, name: &str, value: f64) {
        if self.is_enabled(name) {
            self.fields.insert(name.to_string(), value.to_string());
        }
    }

    fn field_bool(&mut self, name: &str, value: bool) {
        if self.is_enabled(name) {
            self.fields.insert(name.to_string(), value.to_string());
        }
    }
}

struct TextFormatObservabilityProvider;

impl Provider<LogFormatterContext> for TextFormatObservabilityProvider {
    fn name(&self) -> &str {
        "text"
    }

    fn execute(&self, ctx: &mut LogFormatterContext) -> Result<(), Box<dyn std::error::Error>> {
        let (pattern, timestamp_format, enabled_fields) = parse_config(&ctx.log_config);
        let mut visitor = TextVisitor::new(enabled_fields);
        ctx.access_event.visit(&mut visitor);

        let output = pattern.format(&visitor.fields, timestamp_format.as_deref());

        ctx.output = Some(output);
        Ok(())
    }
}

pub struct TextFormatObservabilityModuleLoader;

impl ModuleLoader for TextFormatObservabilityModuleLoader {
    fn register_providers(
        &mut self,
        registry: ferron_core::registry::RegistryBuilder,
    ) -> ferron_core::registry::RegistryBuilder {
        registry
            .with_provider::<LogFormatterContext, _>(|| Arc::new(TextFormatObservabilityProvider))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_field_token() {
        let pattern = FormatPattern::parse("%client_ip");
        assert_eq!(
            pattern.tokens,
            vec![FormatToken::Field("client_ip".to_string())]
        );
    }

    #[test]
    fn parses_header_token() {
        let pattern = FormatPattern::parse("%{Referer}i");
        assert_eq!(
            pattern.tokens,
            vec![FormatToken::Header("Referer".to_string())]
        );
    }

    #[test]
    fn parses_timestamp_token_with_format() {
        let pattern = FormatPattern::parse("%{%Y-%m-%d}t");
        assert_eq!(
            pattern.tokens,
            vec![FormatToken::Timestamp(Some("%Y-%m-%d".to_string()))]
        );
    }

    #[test]
    fn parses_timestamp_token_without_format() {
        let pattern = FormatPattern::parse("%t");
        assert_eq!(pattern.tokens, vec![FormatToken::Timestamp(None)]);
    }

    #[test]
    fn parses_escaped_percent() {
        let pattern = FormatPattern::parse("%%test");
        assert_eq!(
            pattern.tokens,
            vec![FormatToken::Literal("%test".to_string())]
        );
    }

    #[test]
    fn parses_mixed_tokens() {
        let pattern = FormatPattern::parse("%client_ip - %method \"%{User-Agent}i\"");
        assert_eq!(
            pattern.tokens,
            vec![
                FormatToken::Field("client_ip".to_string()),
                FormatToken::Literal(" - ".to_string()),
                FormatToken::Field("method".to_string()),
                FormatToken::Literal(" \"".to_string()),
                FormatToken::Header("User-Agent".to_string()),
                FormatToken::Literal("\"".to_string()),
            ]
        );
    }

    #[test]
    fn parses_clf_default() {
        let pattern = FormatPattern::parse(DEFAULT_CLF_PATTERN);
        // Should contain fields for client_ip, auth_user, t, method, path_and_query, version,
        // status, content_length, and headers for Referer and User-Agent
        let has_client_ip = pattern
            .tokens
            .iter()
            .any(|t| matches!(t, FormatToken::Field(f) if f == "client_ip"));
        let has_referer = pattern
            .tokens
            .iter()
            .any(|t| matches!(t, FormatToken::Header(h) if h == "Referer"));
        let has_timestamp = pattern
            .tokens
            .iter()
            .any(|t| matches!(t, FormatToken::Timestamp(None)));
        assert!(has_client_ip);
        assert!(has_referer);
        assert!(has_timestamp);
    }

    #[test]
    fn formats_fields_from_map() {
        let pattern = FormatPattern::parse("%client_ip %method %status");
        let mut fields = HashMap::new();
        fields.insert("client_ip".to_string(), "192.168.1.1".to_string());
        fields.insert("method".to_string(), "GET".to_string());
        fields.insert("status".to_string(), "200".to_string());

        let output = pattern.format(&fields, None);
        assert_eq!(output, "192.168.1.1 GET 200");
    }

    #[test]
    fn formats_missing_field_as_dash() {
        let pattern = FormatPattern::parse("%client_ip %missing %status");
        let mut fields = HashMap::new();
        fields.insert("client_ip".to_string(), "10.0.0.1".to_string());
        fields.insert("status".to_string(), "404".to_string());

        let output = pattern.format(&fields, None);
        assert_eq!(output, "10.0.0.1 - 404");
    }

    #[test]
    fn formats_header_from_map() {
        let pattern = FormatPattern::parse("%{Referer}i");
        let mut fields = HashMap::new();
        fields.insert(
            "header_referer".to_string(),
            "http://example.com".to_string(),
        );

        let output = pattern.format(&fields, None);
        assert_eq!(output, "http://example.com");
    }

    #[test]
    fn formats_missing_header_as_dash() {
        let pattern = FormatPattern::parse("%{Referer}i");
        let fields = HashMap::new();

        let output = pattern.format(&fields, None);
        assert_eq!(output, "-");
    }

    #[test]
    fn formats_timestamp_with_custom_format() {
        let pattern = FormatPattern::parse("%{%Y-%m-%d %H:%M:%S}t");
        let fields = HashMap::new();

        let output = pattern.format(&fields, None);
        // Should match format like "2026-04-05 12:34:56"
        assert!(output.len() == 19); // Fixed length for this format
        assert!(output.contains('-'));
        assert!(output.contains(':'));
    }

    #[test]
    fn formats_literal_text() {
        let pattern = FormatPattern::parse("GET /path HTTP/1.1");
        let fields = HashMap::new();

        let output = pattern.format(&fields, None);
        assert_eq!(output, "GET /path HTTP/1.1");
    }

    #[test]
    fn visitor_collects_fields() {
        let mut visitor = TextVisitor::new(Arc::new(Vec::new()));
        visitor.field_string("method", "POST");
        visitor.field_u64("status", 201);
        visitor.field_f64("duration_secs", 0.123);
        visitor.field_bool("is_secure", true);

        assert_eq!(visitor.fields.get("method").unwrap(), "POST");
        assert_eq!(visitor.fields.get("status").unwrap(), "201");
        assert!(visitor
            .fields
            .get("duration_secs")
            .unwrap()
            .starts_with("0.123"));
        assert_eq!(visitor.fields.get("is_secure").unwrap(), "true");
    }

    #[test]
    fn visitor_respects_enabled_fields() {
        let enabled = Arc::new(vec!["method".to_string(), "status".to_string()]);
        let mut visitor = TextVisitor::new(enabled);
        visitor.field_string("method", "GET");
        visitor.field_string("path", "/secret");
        visitor.field_u64("status", 200);

        assert_eq!(visitor.fields.len(), 2);
        assert!(visitor.fields.contains_key("method"));
        assert!(visitor.fields.contains_key("status"));
        assert!(!visitor.fields.contains_key("path"));
    }
}
