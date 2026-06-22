use crate::config::QuoteStyle;

/// Returns true if the given character is valid in a bare string.
///
/// Bare strings can contain: alphanumeric, `_`, `-`, `.`, `:`, `/`, `*`, `+`
fn is_bare_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':' | '/' | '*' | '+')
}

/// Returns true if the string is a valid bare string (no quoting needed).
pub fn is_valid_bare_string(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    // "Bare" strings starting with a number, a colon, a dot, an asterisk, a dash
    // would cause parse error with `ferronconf` parser
    if let Some(first) = s.chars().next() {
        if first.is_ascii_digit() || matches!(first, ':' | '.' | '*' | '-') {
            return false;
        }
    }
    // Must start with a letter or special char (not a digit, for host patterns)
    // Actually, per the spec, bare strings can start with any bare char
    // but identifiers start with letters. For values, bare strings are more flexible.
    s.chars().all(is_bare_char)
}

/// Returns true if the string contains interpolation syntax.
fn has_interpolation(s: &str) -> bool {
    s.contains("{{")
}

/// Determines whether a value should be quoted based on the quote style.
pub fn should_quote(s: &str, style: QuoteStyle) -> bool {
    match style {
        QuoteStyle::AlwaysDouble => true,
        QuoteStyle::AlwaysBare => false,
        QuoteStyle::Auto => {
            // Must quote if:
            // - Empty string
            // - Contains interpolation
            // - Contains characters not valid in bare strings
            // - Would be ambiguous with other token types (true, false, numbers)
            if s.is_empty() || has_interpolation(s) || !is_valid_bare_string(s) {
                return true;
            }
            // Would be parsed as a different token type
            if s == "true" || s == "false" || s == "in" {
                return true;
            }
            // Would be parsed as a number
            if s.parse::<f64>().is_ok() {
                return true;
            }
            false
        }
    }
}

/// Escapes a string for use in a double-quoted ferron.conf string.
///
/// Handles: `\n`, `\r`, `\t`, `\\`, `\"`
pub fn escape_quoted_string(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            '\\' => result.push_str("\\\\"),
            '"' => result.push_str("\\\""),
            _ => result.push(c),
        }
    }
    result
}

/// Unescapes a ferron.conf quoted string back to its literal value.
#[allow(dead_code)]
pub fn unescape_quoted_string(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('r') => result.push('\r'),
                Some('t') => result.push('\t'),
                Some('\\') => result.push('\\'),
                Some('"') => result.push('"'),
                Some(other) => {
                    result.push('\\');
                    result.push(other);
                }
                None => result.push('\\'),
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// Formats a string value according to the quote style.
///
/// Returns the formatted string (with or without quotes).
pub fn format_string_value(s: &str, style: QuoteStyle) -> String {
    if should_quote(s, style) {
        format!("\"{}\"", escape_quoted_string(s))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_valid_bare_string() {
        assert!(is_valid_bare_string("hello"));
        assert!(is_valid_bare_string("example.com"));
        assert!(is_valid_bare_string("/var/www/html"));
        assert!(is_valid_bare_string("localhost:8080"));
        assert!(is_valid_bare_string("path/to/file.txt"));
        assert!(is_valid_bare_string("key-value_name"));

        assert!(!is_valid_bare_string(""));
        assert!(!is_valid_bare_string("hello world"));
        assert!(!is_valid_bare_string("key=value"));
        assert!(!is_valid_bare_string("hello\"world"));
    }

    #[test]
    fn test_should_quote_auto() {
        // Valid bare strings should not be quoted
        assert!(!should_quote("hello", QuoteStyle::Auto));
        assert!(!should_quote("example.com", QuoteStyle::Auto));
        assert!(!should_quote("/var/www", QuoteStyle::Auto));

        // Invalid bare strings should be quoted
        assert!(should_quote("", QuoteStyle::Auto));
        assert!(should_quote("hello world", QuoteStyle::Auto));
        assert!(should_quote("key=value", QuoteStyle::Auto));

        // Keywords should be quoted
        assert!(should_quote("true", QuoteStyle::Auto));
        assert!(should_quote("false", QuoteStyle::Auto));
        assert!(should_quote("in", QuoteStyle::Auto));

        // Numbers should be quoted (to distinguish from Number tokens)
        assert!(should_quote("42", QuoteStyle::Auto));
        assert!(should_quote("3.14", QuoteStyle::Auto));

        // Interpolation should be quoted
        assert!(should_quote("{{var}}", QuoteStyle::Auto));
        assert!(should_quote("prefix {{var}} suffix", QuoteStyle::Auto));
    }

    #[test]
    fn test_should_quote_always_double() {
        assert!(should_quote("hello", QuoteStyle::AlwaysDouble));
        assert!(should_quote("example.com", QuoteStyle::AlwaysDouble));
    }

    #[test]
    fn test_should_quote_always_bare() {
        assert!(!should_quote("hello", QuoteStyle::AlwaysBare));
        assert!(!should_quote("example.com", QuoteStyle::AlwaysBare));
    }

    #[test]
    fn test_escape_unescape_roundtrip() {
        let cases = vec![
            "hello world",
            "line1\nline2",
            "tab\there",
            "back\\slash",
            "quote\"inside",
            "",
            "multi\n\r\t\\\"escape",
        ];
        for case in cases {
            let escaped = escape_quoted_string(case);
            let unescaped = unescape_quoted_string(&escaped);
            assert_eq!(unescaped, case, "Roundtrip failed for: {:?}", case);
        }
    }

    #[test]
    fn test_format_string_value() {
        assert_eq!(format_string_value("hello", QuoteStyle::Auto), "hello");
        assert_eq!(
            format_string_value("hello world", QuoteStyle::Auto),
            "\"hello world\""
        );
        assert_eq!(format_string_value("true", QuoteStyle::Auto), "\"true\"");
        assert_eq!(format_string_value("42", QuoteStyle::Auto), "\"42\"");
        assert_eq!(
            format_string_value("hello", QuoteStyle::AlwaysDouble),
            "\"hello\""
        );
    }
}
