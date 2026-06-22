use std::str::FromStr;

use ferronconf::{
    Block, Config, HostBlock, HostLabels, HostPattern, MatchBlock, Operand,
    SnippetBlock, Statement, StringPart, Value,
};

use crate::config::FormatConfig;
use crate::quoting::format_string_value;

/// Formats a `Config` AST into a formatted string.
pub fn format_config(config: &Config, config_fmt: &FormatConfig) -> String {
    let mut output = String::new();
    let mut formatter = Formatter::new(config_fmt);
    formatter.format_config(config, &mut output);
    output
}

struct Formatter<'a> {
    config: &'a FormatConfig,
    depth: usize,
}

impl<'a> Formatter<'a> {
    fn new(config: &'a FormatConfig) -> Self {
        Self { config, depth: 0 }
    }

    fn indent(&self) -> String {
        self.config.indent_at(self.depth)
    }

    fn format_config(&mut self, config: &Config, output: &mut String) {
        for (i, stmt) in config.statements.iter().enumerate() {
            // Blank lines before
            let blank = config.blank_lines_before.get(&i).copied().unwrap_or(0);
            let blank = blank.min(self.config.max_blank_lines);
            if i > 0 {
                for _ in 0..blank {
                    output.push('\n');
                }
            }
            self.format_statement(stmt, output, &config.trailing_comments, i);
        }
        // Strip trailing newline if configured
        if !self.config.trailing_newline && output.ends_with('\n') {
            output.pop();
        }
    }

    fn format_statement(
        &mut self,
        stmt: &Statement,
        output: &mut String,
        trailing_comments: &std::collections::HashMap<usize, String>,
        idx: usize,
    ) {
        match stmt {
            Statement::Directive(d) => self.format_directive(d, output, trailing_comments, idx),
            Statement::HostBlock(h) => self.format_host_block(h, output, trailing_comments, idx),
            Statement::MatchBlock(m) => self.format_match_block(m, output, trailing_comments, idx),
            Statement::GlobalBlock(b) => self.format_global_block(b, output),
            Statement::SnippetBlock(s) => self.format_snippet_block(s, output, trailing_comments, idx),
            Statement::Comment(text, _) => {
                output.push_str(&self.indent());
                output.push_str(text);
                output.push('\n');
            }
        }
    }

    fn format_directive(
        &mut self,
        d: &ferronconf::Directive,
        output: &mut String,
        trailing_comments: &std::collections::HashMap<usize, String>,
        idx: usize,
    ) {
        output.push_str(&self.indent());
        output.push_str(&d.name);
        for arg in &d.args {
            output.push(' ');
            self.format_value(arg, output);
        }
        if let Some(block) = &d.block {
            output.push_str(" {\n");
            self.depth += 1;
            self.format_block_contents(block, output);
            self.depth -= 1;
            output.push_str(&self.indent());
            output.push('}');
        }
        self.format_trailing_comment(trailing_comments, idx, output);
        output.push('\n');
    }

    fn format_host_block(
        &mut self,
        h: &HostBlock,
        output: &mut String,
        trailing_comments: &std::collections::HashMap<usize, String>,
        idx: usize,
    ) {
        output.push_str(&self.indent());
        for (i, host) in h.hosts.iter().enumerate() {
            if i > 0 {
                output.push_str(", ");
            }
            self.format_host_pattern(host, output);
        }
        output.push_str(" {\n");
        self.depth += 1;
        self.format_block_contents(&h.block, output);
        self.depth -= 1;
        output.push_str(&self.indent());
        output.push('}');
        self.format_trailing_comment(trailing_comments, idx, output);
        output.push('\n');
    }

    fn format_match_block(
        &mut self,
        m: &MatchBlock,
        output: &mut String,
        trailing_comments: &std::collections::HashMap<usize, String>,
        idx: usize,
    ) {
        output.push_str(&self.indent());
        output.push_str("match ");
        output.push_str(&m.matcher);
        output.push_str(" {\n");
        self.depth += 1;
        for expr in &m.expr {
            output.push_str(&self.indent());
            self.format_operand(&expr.left, output);
            output.push(' ');
            output.push_str(expr.op.as_str());
            output.push(' ');
            self.format_operand(&expr.right, output);
            output.push('\n');
        }
        self.depth -= 1;
        output.push_str(&self.indent());
        output.push('}');
        self.format_trailing_comment(trailing_comments, idx, output);
        output.push('\n');
    }

    fn format_global_block(&mut self, block: &Block, output: &mut String) {
        output.push_str(&self.indent());
        output.push_str("{\n");
        self.depth += 1;
        self.format_block_contents(block, output);
        self.depth -= 1;
        output.push_str(&self.indent());
        output.push_str("}\n");
    }

    fn format_snippet_block(
        &mut self,
        s: &SnippetBlock,
        output: &mut String,
        trailing_comments: &std::collections::HashMap<usize, String>,
        idx: usize,
    ) {
        output.push_str(&self.indent());
        output.push_str("snippet ");
        output.push_str(&s.name);
        output.push_str(" {\n");
        self.depth += 1;
        self.format_block_contents(&s.block, output);
        self.depth -= 1;
        output.push_str(&self.indent());
        output.push('}');
        self.format_trailing_comment(trailing_comments, idx, output);
        output.push('\n');
    }

    fn format_block_contents(&mut self, block: &Block, output: &mut String) {
        let statements: Vec<(usize, &Statement)> = if self.config.sort_directives {
            let mut indexed: Vec<(usize, &Statement)> = block.statements.iter().enumerate().collect();
            indexed.sort_by(|a, b| {
                let name_a = Self::statement_name(a.1);
                let name_b = Self::statement_name(b.1);
                name_a.cmp(name_b)
            });
            indexed
        } else {
            block.statements.iter().enumerate().collect()
        };

        for (i, stmt) in statements {
            // Blank lines before (inside block)
            let blank = block.blank_lines_before.get(&i).copied().unwrap_or(0);
            let blank = blank.min(self.config.max_blank_lines);
            if i > 0 {
                for _ in 0..blank {
                    output.push('\n');
                }
            }
            self.format_statement(stmt, output, &block.trailing_comments, i);
        }
    }

    fn statement_name(stmt: &Statement) -> &str {
        match stmt {
            Statement::Directive(d) => &d.name,
            Statement::HostBlock(_) => "",
            Statement::MatchBlock(m) => &m.matcher,
            Statement::GlobalBlock(_) => "",
            Statement::SnippetBlock(s) => &s.name,
            Statement::Comment(_, _) => "",
        }
    }

    fn format_trailing_comment(
        &self,
        trailing_comments: &std::collections::HashMap<usize, String>,
        idx: usize,
        output: &mut String,
    ) {
        if let Some(comment) = trailing_comments.get(&idx) {
            output.push(' ');
            output.push_str(comment);
        }
    }

    fn format_value(&self, value: &Value, output: &mut String) {
        match value {
            Value::String(s, _) => {
                if self.config.normalize_quotes {
                    output.push_str(&format_string_value(s, self.config.quote_style));
                } else {
                    // Preserve original quoting style
                    output.push('"');
                    output.push_str(&crate::quoting::escape_quoted_string(s));
                    output.push('"');
                }
            }
            Value::Integer(i, _) => {
                output.push_str(&i.to_string());
            }
            Value::Float(f, _) => {
                output.push_str(&f.to_string());
            }
            Value::Boolean(b, _) => {
                output.push_str(if *b { "true" } else { "false" });
            }
            Value::InterpolatedString(parts, _) => {
                output.push('"');
                for part in parts {
                    match part {
                        StringPart::Literal(s) => {
                            output.push_str(&crate::quoting::escape_quoted_string(s));
                        }
                        StringPart::Expression(path) => {
                            output.push_str("{{");
                            output.push_str(&path.join("."));
                            output.push_str("}}");
                        }
                    }
                }
                output.push('"');
            }
        }
    }

    fn format_host_pattern(&self, pattern: &HostPattern, output: &mut String) {
        if let Some(protocol) = &pattern.protocol {
            output.push_str(protocol);
            output.push(' ');
        }
        self.format_host_labels(&pattern.labels, output);
        if let Some(port) = pattern.port {
            output.push(':');
            output.push_str(&port.to_string());
        }
    }

    fn format_host_labels(&self, labels: &HostLabels, output: &mut String) {
        match labels {
            HostLabels::Hostname(parts) => {
                output.push_str(&parts.join("."));
            }
            HostLabels::IpAddr(ip) => {
                match ip {
                    std::net::IpAddr::V4(v4) => {
                        output.push_str(&v4.to_string());
                    }
                    std::net::IpAddr::V6(v6) => {
                        output.push('[');
                        output.push_str(&v6.to_string());
                        output.push(']');
                    }
                }
            }
            HostLabels::Wildcard => {
                output.push('*');
            }
        }
    }

    fn format_operand(&self, operand: &Operand, output: &mut String) {
        match operand {
            Operand::Identifier(parts, _) => {
                output.push_str(&parts.join("."));
            }
            Operand::String(s, _) => {
                if self.config.normalize_quotes {
                    output.push_str(&format_string_value(s, self.config.quote_style));
                } else {
                    output.push('"');
                    output.push_str(&crate::quoting::escape_quoted_string(s));
                    output.push('"');
                }
            }
            Operand::Integer(i, _) => {
                output.push_str(&i.to_string());
            }
            Operand::Float(f, _) => {
                output.push_str(&f.to_string());
            }
        }
    }
}

/// Checks if two configs produce the same formatted output (idempotency test helper).
#[allow(dead_code)]
pub fn format_is_idempotent(input: &str, config_fmt: &FormatConfig) -> bool {
    let config1 = match Config::from_str(input) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let formatted1 = format_config(&config1, config_fmt);
    let config2 = match Config::from_str(&formatted1) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let formatted2 = format_config(&config2, config_fmt);
    formatted1 == formatted2
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{FormatConfig, IndentStyle, QuoteStyle};
    use std::str::FromStr;

    fn default_config() -> FormatConfig {
        FormatConfig::default()
    }

    #[test]
    fn test_format_simple_directive() {
        let input = "root /var/www";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &default_config());
        assert_eq!(output, "root /var/www\n");
    }

    #[test]
    fn test_format_host_block() {
        let input = "example.com {\n    root /var/www\n}";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &default_config());
        assert_eq!(
            output,
            "example.com {\n    root /var/www\n}\n"
        );
    }

    #[test]
    fn test_format_global_block() {
        let input = "{\n    default_http_port 8080\n}";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &default_config());
        assert_eq!(
            output,
            "{\n    default_http_port 8080\n}\n"
        );
    }

    #[test]
    fn test_format_match_block() {
        let input = "match curl_client {\n    request.header.user_agent ~ \"curl\"\n}";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &default_config());
        // Auto mode: "curl" is a valid bare string
        assert_eq!(
            output,
            "match curl_client {\n    request.header.user_agent ~ curl\n}\n"
        );
    }

    #[test]
    fn test_format_snippet_block() {
        let input = "snippet tls_acme {\n    tls {\n        provider \"acme\"\n    }\n}";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &default_config());
        assert_eq!(
            output,
            "snippet tls_acme {\n    tls {\n        provider acme\n    }\n}\n"
        );
    }

    #[test]
    fn test_format_standalone_comment() {
        let input = "# This is a comment\nroot /var/www";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &default_config());
        assert_eq!(
            output,
            "# This is a comment\nroot /var/www\n"
        );
    }

    #[test]
    fn test_format_trailing_comment() {
        let input = "root /var/www # main site";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &default_config());
        assert_eq!(
            output,
            "root /var/www # main site\n"
        );
    }

    #[test]
    fn test_format_comment_in_block() {
        let input = "example.com {\n    # Root directory\n    root /var/www\n}";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &default_config());
        assert_eq!(
            output,
            "example.com {\n    # Root directory\n    root /var/www\n}\n"
        );
    }

    #[test]
    fn test_format_multiple_comments() {
        let input = "# Comment 1\n# Comment 2\nroot /var/www";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &default_config());
        assert_eq!(
            output,
            "# Comment 1\n# Comment 2\nroot /var/www\n"
        );
    }

    #[test]
    fn test_format_nested_blocks() {
        let input = "example.com {\n    tls {\n        provider \"acme\"\n        challenge http-01\n    }\n}";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &default_config());
        assert_eq!(
            output,
            "example.com {\n    tls {\n        provider acme\n        challenge http-01\n    }\n}\n"
        );
    }

    #[test]
    fn test_format_host_with_port() {
        let input = "example.com:443 {\n    root /var/www\n}";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &default_config());
        assert_eq!(
            output,
            "example.com:443 {\n    root /var/www\n}\n"
        );
    }

    #[test]
    fn test_format_host_with_protocol() {
        let input = "http example.com {\n    root /var/www\n}";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &default_config());
        assert_eq!(
            output,
            "http example.com {\n    root /var/www\n}\n"
        );
    }

    #[test]
    fn test_format_ipv6() {
        let input = "[2001:db8::1]:8080 {\n    root /var/www\n}";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &default_config());
        assert_eq!(
            output,
            "[2001:db8::1]:8080 {\n    root /var/www\n}\n"
        );
    }

    #[test]
    fn test_format_wildcard_host() {
        let input = "*.example.com {\n    root /var/www\n}";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &default_config());
        assert_eq!(
            output,
            "*.example.com {\n    root /var/www\n}\n"
        );
    }

    #[test]
    fn test_format_comma_separated_hosts() {
        let input = "a.com, b.com:8080 {\n    root /var/www\n}";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &default_config());
        assert_eq!(
            output,
            "a.com, b.com:8080 {\n    root /var/www\n}\n"
        );
    }

    #[test]
    fn test_format_directive_with_block() {
        let input = "tls {\n    provider \"acme\"\n}";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &default_config());
        // Auto mode: "acme" is a valid bare string, so quotes are removed
        assert_eq!(
            output,
            "tls {\n    provider acme\n}\n"
        );
    }

    #[test]
    fn test_format_directive_with_trailing_comment() {
        let input = "tls {\n    provider \"acme\" # auto TLS\n}";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &default_config());
        assert_eq!(
            output,
            "tls {\n    provider acme # auto TLS\n}\n"
        );
    }

    #[test]
    fn test_format_boolean_values() {
        let input = "enabled true\ndisabled false";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &default_config());
        assert_eq!(
            output,
            "enabled true\ndisabled false\n"
        );
    }

    #[test]
    fn test_format_number_values() {
        let input = "port 80\nratio 3.14";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &default_config());
        assert_eq!(
            output,
            "port 80\nratio 3.14\n"
        );
    }

    #[test]
    fn test_format_interpolation() {
        let input = "cert \"{{env.TLS_CERT}}\"";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &default_config());
        assert_eq!(
            output,
            "cert \"{{env.TLS_CERT}}\"\n"
        );
    }

    #[test]
    fn test_format_match_operators() {
        let input = "match test {\n    a == b\n    c != d\n    e ~ f\n    g !~ h\n    i in j\n}";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &default_config());
        assert_eq!(
            output,
            "match test {\n    a == b\n    c != d\n    e ~ f\n    g !~ h\n    i in j\n}\n"
        );
    }

    #[test]
    fn test_format_empty_config() {
        let config = Config::from_str("").unwrap();
        let output = format_config(&config, &default_config());
        assert_eq!(output, "");
    }

    #[test]
    fn test_format_empty_block() {
        let input = "example.com { }";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &default_config());
        assert_eq!(output, "example.com {\n}\n");
    }

    #[test]
    fn test_format_2_spaces_indent() {
        let mut fmt = default_config();
        fmt.indent_width = 2;
        let input = "example.com {\n    root /var/www\n}";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &fmt);
        assert_eq!(
            output,
            "example.com {\n  root /var/www\n}\n"
        );
    }

    #[test]
    fn test_format_tabs_indent() {
        let mut fmt = default_config();
        fmt.indent_style = IndentStyle::Tabs;
        let input = "example.com {\n    root /var/www\n}";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &fmt);
        assert_eq!(
            output,
            "example.com {\n\troot /var/www\n}\n"
        );
    }

    #[test]
    fn test_format_always_double_quotes() {
        let mut fmt = default_config();
        fmt.quote_style = QuoteStyle::AlwaysDouble;
        let input = "root /var/www";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &fmt);
        assert_eq!(
            output,
            "root \"/var/www\"\n"
        );
    }

    #[test]
    fn test_format_quote_normalization_auto() {
        // "true" as a bare string should be quoted (would be parsed as boolean)
        let input = "directive \"true\"";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &default_config());
        assert_eq!(
            output,
            "directive \"true\"\n"
        );
    }

    #[test]
    fn test_format_idempotent() {
        let input = "example.com {\n    root /var/www\n    tls {\n        provider acme\n    }\n}\n";
        assert!(format_is_idempotent(input, &default_config()));
    }

    #[test]
    fn test_format_idempotent_with_comments() {
        let input = "# Global\nexample.com {\n    # Root\n    root /var/www\n}\n";
        assert!(format_is_idempotent(input, &default_config()));
    }

    #[test]
    fn test_format_idempotent_with_trailing_comments() {
        let input = "root /var/www # main site\n";
        assert!(format_is_idempotent(input, &default_config()));
    }

    #[test]
    fn test_format_preserves_blank_lines() {
        let input = "a 1\n\nb 2\n";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &default_config());
        assert_eq!(output, "a 1\n\nb 2\n");
    }

    #[test]
    fn test_format_max_blank_lines() {
        let mut fmt = default_config();
        fmt.max_blank_lines = 1;
        let input = "a 1\n\n\n\nb 2\n";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &fmt);
        // The parser tracks blank lines. With 3 blank lines (4 newlines), max_blank_lines=1 trims to 1
        assert_eq!(output, "a 1\n\nb 2\n");
    }

    #[test]
    fn test_format_no_trailing_newline() {
        let mut fmt = default_config();
        fmt.trailing_newline = false;
        let input = "root /var/www\n";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &fmt);
        assert_eq!(output, "root /var/www");
    }

    #[test]
    fn test_format_complex_config() {
        let input = r#"# Global settings
{
    default_http_port 8080
}

# Snippet
snippet tls_acme {
    tls {
        provider "acme"
        challenge http-01
    }
}

# Main site
example.com:443 {
    use tls_acme
    root /var/www/example

    # Proxy
    proxy http://localhost:3000 {
        keepalive true
    }
}
"#;
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &default_config());
        // Verify structure
        assert!(output.contains("# Global settings"));
        assert!(output.contains("snippet tls_acme"));
        assert!(output.contains("example.com:443"));
        assert!(output.contains("proxy http://localhost:3000"));
        // Verify it round-trips (format the output again should be same)
        assert!(format_is_idempotent(&output, &default_config()));
    }
}
