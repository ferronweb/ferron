use std::collections::HashSet;
use std::str::FromStr;

use ferronconf::{
    Block, Config, HostBlock, HostLabels, HostPattern, MatchBlock, Operand, SnippetBlock,
    Statement, StringPart, Value,
};

use crate::config::FormatConfig;
use crate::quoting::{format_string_value, format_string_value_raw};
use crate::source_analysis::SourceAnalysis;

/// Formats a `Config` AST into a formatted string.
///
/// If `source` is provided, line continuations from the original input are preserved.
/// If `source_analysis` is provided, raw strings (`r"..."`) are preserved.
pub fn format_config_with_analysis(
    config: &Config,
    config_fmt: &FormatConfig,
    source_analysis: Option<&SourceAnalysis>,
    source: Option<&str>,
) -> String {
    let mut output = String::new();
    let mut formatter = Formatter::new(config_fmt, source_analysis, source);
    formatter.format_config(config, &mut output);
    output
}

/// Formats a `Config` AST into a formatted string (backward-compatible).
#[allow(dead_code)]
pub fn format_config(config: &Config, config_fmt: &FormatConfig) -> String {
    format_config_with_analysis(config, config_fmt, None, None)
}

struct Formatter<'a> {
    config: &'a FormatConfig,
    source_analysis: Option<&'a SourceAnalysis>,
    source: Option<&'a str>,
    depth: usize,
}

impl<'a> Formatter<'a> {
    fn new(
        config: &'a FormatConfig,
        source_analysis: Option<&'a SourceAnalysis>,
        source: Option<&'a str>,
    ) -> Self {
        Self {
            config,
            source_analysis,
            source,
            depth: 0,
        }
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
            Statement::SnippetBlock(s) => {
                self.format_snippet_block(s, output, trailing_comments, idx)
            }
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
        let gaps_with_continuations = self.find_directive_continuation_gaps(d);

        output.push_str(&self.indent());
        output.push_str(&d.name);

        for (arg_idx, arg) in d.args.iter().enumerate() {
            if gaps_with_continuations.contains(&arg_idx) {
                output.push_str(" \\\n");
                output.push_str(&self.indent());
            } else {
                output.push(' ');
            }
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

    /// Finds which argument gaps in a directive had line continuations in the original source.
    ///
    /// Returns a set of argument indices: if `arg_idx` is in the set, a `\` should be
    /// inserted after argument `arg_idx` (i.e., between `arg_idx` and `arg_idx + 1`).
    fn find_directive_continuation_gaps(&self, d: &ferronconf::Directive) -> HashSet<usize> {
        let mut gaps = HashSet::new();

        let (Some(source), Some(analysis)) = (self.source, self.source_analysis) else {
            return gaps;
        };

        let span = d.span;
        let start_line = span.line;
        let start_col = span.column;

        // Find the byte offset of the directive start in the source
        let source_lines: Vec<&str> = source.lines().collect();
        if start_line == 0 || start_line > source_lines.len() {
            return gaps;
        }

        let directive_line = &source_lines[start_line - 1];
        if start_col == 0 || start_col > directive_line.len() {
            return gaps;
        }
        let directive_start_byte = directive_line[..start_col - 1].len();

        // Scan the directive region for continuations
        let mut byte_offset = directive_start_byte;
        let mut current_line = start_line;
        let mut paren_depth: i32 = 0;
        let mut in_string = false;

        while byte_offset < source.len() && current_line <= source_lines.len() {
            let line = &source_lines[current_line - 1];
            let col_offset = if current_line == start_line {
                start_col - 1
            } else {
                0
            };

            let bytes = line.as_bytes();
            let mut i = col_offset;

            while i < bytes.len() {
                let b = bytes[i];

                if in_string {
                    if b == b'\\' && i + 1 < bytes.len() {
                        i += 2;
                        continue;
                    }
                    if b == b'"' {
                        in_string = false;
                    }
                    i += 1;
                    continue;
                }

                match b {
                    b'"' => {
                        in_string = true;
                    }
                    b'{' => {
                        paren_depth += 1;
                    }
                    b'}' => {
                        paren_depth -= 1;
                        if paren_depth < 0 {
                            // Exited the directive
                            return gaps;
                        }
                    }
                    b'\\' => {
                        // Check for line continuation
                        let mut j = i + 1;
                        // Skip whitespace after `\`
                        while j < bytes.len()
                            && bytes[j].is_ascii_whitespace()
                            && bytes[j] != b'\n'
                            && bytes[j] != b'\r'
                        {
                            j += 1;
                        }
                        // Skip optional comment
                        if j < bytes.len() && bytes[j] == b'#' {
                            while j < bytes.len() && bytes[j] != b'\n' && bytes[j] != b'\r' {
                                j += 1;
                            }
                        }
                        // If we reached end of line, it's a continuation
                        if j >= bytes.len() || bytes[j] == b'\n' || bytes[j] == b'\r' {
                            // Found a continuation at (current_line, i + 1)
                            // Map to argument gap by counting tokens before this position
                            let col = i + 1; // 1-indexed
                            if analysis.has_continuation(current_line, col) {
                                let gap = self.count_tokens_before(d, current_line, col);
                                gaps.insert(gap);
                            }
                        }
                    }
                    _ => {}
                }
                i += 1;
            }

            // Move to next line
            byte_offset += line.len() + 1; // +1 for newline
            current_line += 1;
        }

        gaps
    }

    /// Counts how many of the directive's tokens (name + args) appear before
    /// the given (line, column) position in the source.
    fn count_tokens_before(&self, d: &ferronconf::Directive, line: usize, column: usize) -> usize {
        let source = match self.source {
            Some(s) => s,
            None => return 0,
        };

        let name_span = d.span;

        let source_lines: Vec<&str> = source.lines().collect();
        if name_span.line == 0 || name_span.line > source_lines.len() {
            return 0;
        }

        let name_line = &source_lines[name_span.line - 1];
        if name_span.column == 0 || name_span.column > name_line.len() {
            return 0;
        }

        let start_byte = {
            let line_offset: usize = source_lines[..name_span.line - 1]
                .iter()
                .map(|l| l.len() + 1)
                .sum();
            line_offset + name_span.column - 1
        };

        let end_byte = {
            let line_offset: usize = source_lines[..line - 1].iter().map(|l| l.len() + 1).sum();
            line_offset + column - 1
        };

        if end_byte <= start_byte {
            return 0;
        }

        let region = &source[start_byte..end_byte.min(source.len())];

        // Count tokens in this region (skip whitespace, skip continuation backslashes)
        let mut token_count = 0;
        let mut chars = region.chars().peekable();
        let mut in_string = false;

        while let Some(c) = chars.next() {
            if in_string {
                if c == '\\' {
                    chars.next();
                    continue;
                }
                if c == '"' {
                    in_string = false;
                }
                continue;
            }

            if c == '"' {
                in_string = true;
                token_count += 1;
            } else if c.is_whitespace()
                || c == '\\'
                || c == '{'
                || c == '}'
                || c == ','
                || c == ';'
                || c == '#'
            {
                continue;
            } else {
                token_count += 1;
                while let Some(&next) = chars.peek() {
                    if next.is_whitespace()
                        || next == '{'
                        || next == '}'
                        || next == ','
                        || next == ';'
                        || next == '#'
                        || next == '"'
                        || next == '\\'
                    {
                        break;
                    }
                    chars.next();
                }
            }
        }

        // token_count is the number of tokens before the continuation position.
        // Token 0 = directive name, token 1 = first arg, etc.
        // Gap after arg[i] = between token i+1 and token i+2.
        if token_count == 0 {
            0
        } else if token_count <= d.args.len() {
            token_count - 1
        } else {
            d.args.len().saturating_sub(1)
        }
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
            let mut indexed: Vec<(usize, &Statement)> =
                block.statements.iter().enumerate().collect();
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
            Value::String(s, span) => {
                let is_raw = self
                    .source_analysis
                    .map(|a| a.is_raw(span.line, span.column))
                    .unwrap_or(false);
                if is_raw {
                    output.push_str(&format_string_value_raw(s, self.config.quote_style, true));
                } else if self.config.normalize_quotes {
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
            HostLabels::IpAddr(ip) => match ip {
                std::net::IpAddr::V4(v4) => {
                    output.push_str(&v4.to_string());
                }
                std::net::IpAddr::V6(v6) => {
                    output.push('[');
                    output.push_str(&v6.to_string());
                    output.push(']');
                }
            },
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
            Operand::String(s, span) => {
                let is_raw = self
                    .source_analysis
                    .map(|a| a.is_raw(span.line, span.column))
                    .unwrap_or(false);
                if is_raw {
                    output.push_str(&format_string_value_raw(s, self.config.quote_style, true));
                } else if self.config.normalize_quotes {
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
    let analysis = crate::source_analysis::analyze_input(input);
    let config1 = match Config::from_str(input) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let formatted1 =
        format_config_with_analysis(&config1, config_fmt, Some(&analysis), Some(input));
    let analysis2 = crate::source_analysis::analyze_input(&formatted1);
    let config2 = match Config::from_str(&formatted1) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let formatted2 =
        format_config_with_analysis(&config2, config_fmt, Some(&analysis2), Some(&formatted1));
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
        assert_eq!(output, "example.com {\n    root /var/www\n}\n");
    }

    #[test]
    fn test_format_global_block() {
        let input = "{\n    default_http_port 8080\n}";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &default_config());
        assert_eq!(output, "{\n    default_http_port 8080\n}\n");
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
        assert_eq!(output, "# This is a comment\nroot /var/www\n");
    }

    #[test]
    fn test_format_trailing_comment() {
        let input = "root /var/www # main site";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &default_config());
        assert_eq!(output, "root /var/www # main site\n");
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
        assert_eq!(output, "# Comment 1\n# Comment 2\nroot /var/www\n");
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
        assert_eq!(output, "example.com:443 {\n    root /var/www\n}\n");
    }

    #[test]
    fn test_format_host_with_protocol() {
        let input = "http example.com {\n    root /var/www\n}";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &default_config());
        assert_eq!(output, "http example.com {\n    root /var/www\n}\n");
    }

    #[test]
    fn test_format_ipv6() {
        let input = "[2001:db8::1]:8080 {\n    root /var/www\n}";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &default_config());
        assert_eq!(output, "[2001:db8::1]:8080 {\n    root /var/www\n}\n");
    }

    #[test]
    fn test_format_wildcard_host() {
        let input = "*.example.com {\n    root /var/www\n}";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &default_config());
        assert_eq!(output, "*.example.com {\n    root /var/www\n}\n");
    }

    #[test]
    fn test_format_comma_separated_hosts() {
        let input = "a.com, b.com:8080 {\n    root /var/www\n}";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &default_config());
        assert_eq!(output, "a.com, b.com:8080 {\n    root /var/www\n}\n");
    }

    #[test]
    fn test_format_directive_with_block() {
        let input = "tls {\n    provider \"acme\"\n}";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &default_config());
        // Auto mode: "acme" is a valid bare string, so quotes are removed
        assert_eq!(output, "tls {\n    provider acme\n}\n");
    }

    #[test]
    fn test_format_directive_with_trailing_comment() {
        let input = "tls {\n    provider \"acme\" # auto TLS\n}";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &default_config());
        assert_eq!(output, "tls {\n    provider acme # auto TLS\n}\n");
    }

    #[test]
    fn test_format_boolean_values() {
        let input = "enabled true\ndisabled false";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &default_config());
        assert_eq!(output, "enabled true\ndisabled false\n");
    }

    #[test]
    fn test_format_number_values() {
        let input = "port 80\nratio 3.14";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &default_config());
        assert_eq!(output, "port 80\nratio 3.14\n");
    }

    #[test]
    fn test_format_interpolation() {
        let input = "cert \"{{env.TLS_CERT}}\"";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &default_config());
        assert_eq!(output, "cert \"{{env.TLS_CERT}}\"\n");
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
        assert_eq!(output, "example.com {\n  root /var/www\n}\n");
    }

    #[test]
    fn test_format_tabs_indent() {
        let mut fmt = default_config();
        fmt.indent_style = IndentStyle::Tabs;
        let input = "example.com {\n    root /var/www\n}";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &fmt);
        assert_eq!(output, "example.com {\n\troot /var/www\n}\n");
    }

    #[test]
    fn test_format_always_double_quotes() {
        let mut fmt = default_config();
        fmt.quote_style = QuoteStyle::AlwaysDouble;
        let input = "root /var/www";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &fmt);
        assert_eq!(output, "root \"/var/www\"\n");
    }

    #[test]
    fn test_format_quote_normalization_auto() {
        // "true" as a bare string should be quoted (would be parsed as boolean)
        let input = "directive \"true\"";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &default_config());
        assert_eq!(output, "directive \"true\"\n");
    }

    #[test]
    fn test_format_idempotent() {
        let input =
            "example.com {\n    root /var/www\n    tls {\n        provider acme\n    }\n}\n";
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

    #[test]
    fn test_format_raw_string_preserved() {
        let input = "match test {\n    request.uri.path ~ r\"^/api/v1$\"\n}";
        let config = Config::from_str(input).unwrap();
        let analysis = crate::source_analysis::analyze_input(input);
        let output =
            format_config_with_analysis(&config, &default_config(), Some(&analysis), Some(input));
        assert!(output.contains("r\"^/api/v1$\""));
        assert!(!output.contains("\"\\^/api/v1\\$\""));
    }

    #[test]
    fn test_format_raw_string_idempotent() {
        let input = "match test {\n    request.uri.path ~ r\"^/api/v1$\"\n}\n";
        assert!(format_is_idempotent(input, &default_config()));
    }

    #[test]
    fn test_format_raw_string_not_confused_with_quoted() {
        // A regular quoted string should NOT become raw
        let input = "match test {\n    request.uri.path ~ \"hello world\"\n}";
        let config = Config::from_str(input).unwrap();
        let analysis = crate::source_analysis::analyze_input(input);
        let output =
            format_config_with_analysis(&config, &default_config(), Some(&analysis), Some(input));
        // "hello world" contains a space so it stays quoted, but should NOT become raw
        assert!(output.contains("\"hello world\""));
        assert!(!output.contains("r\""));
    }

    #[test]
    fn test_format_line_continuation_preserved() {
        let input = "proxy http://localhost:3000 \\\n    http://localhost:3001\n";
        let config = Config::from_str(input).unwrap();
        let analysis = crate::source_analysis::analyze_input(input);
        let output =
            format_config_with_analysis(&config, &default_config(), Some(&analysis), Some(input));
        assert!(output.contains("\\\n"));
        assert!(output.contains("http://localhost:3000 \\"));
    }

    #[test]
    fn test_format_line_continuation_with_comment() {
        let input = "proxy http://localhost:3000 \\ # first\n    http://localhost:3001\n";
        let config = Config::from_str(input).unwrap();
        let analysis = crate::source_analysis::analyze_input(input);
        let output =
            format_config_with_analysis(&config, &default_config(), Some(&analysis), Some(input));
        assert!(output.contains("\\"));
    }

    #[test]
    fn test_format_no_continuation_without_source() {
        // Without source text, no continuations are preserved
        let input = "proxy http://localhost:3000 \\\n    http://localhost:3001\n";
        let config = Config::from_str(input).unwrap();
        let output = format_config(&config, &default_config());
        assert!(!output.contains("\\\n"));
        assert!(output.contains("proxy http://localhost:3000 http://localhost:3001"));
    }

    #[test]
    fn test_format_raw_string_in_directive() {
        let input = "proxy http://localhost:3000 r\"^/api\"\n";
        let config = Config::from_str(input).unwrap();
        let analysis = crate::source_analysis::analyze_input(input);
        let output =
            format_config_with_analysis(&config, &default_config(), Some(&analysis), Some(input));
        assert!(output.contains("r\"^/api\""));
    }
}
