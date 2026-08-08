use crate::config::QuoteStyle;

/// Returns true if the given character is valid in a bare string.
fn is_bare_char(c: char) -> bool {
    !c.is_whitespace() && !matches!(c, '{' | '}' | '"' | '#' | ',' | ';')
}

/// Returns true if the string is a valid bare string (no quoting needed).
pub fn is_valid_bare_string(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }

    // Bare string edge cases...
    // 1. "[" and "]" may appear (for IPv6 addresses), but only "[" once, and "]" once;
    //    also, "]" should come after "[", not before (that's `ferronconf` crate parsing limitation)
    // 2. "-123", "+123", "123", "-12.3", "12.3", "+12.3" would be interpreted as numbers
    // 3. "true" and "false" would be interpreted as booleans
    // 4. "==", "!=", "~", "~=", "in" would be interpreted as operators inside `match` blocks
    // 5. "snippet", "match" would be interpreted as keywords
    //
    // Some of the edge cases are handled by `should_quote`...

    // So check if "[" and "]" either both appear exactly once, or neither appear at all
    if s.contains('[') != s.contains(']')
        || (s.contains('[')
            && s.contains(']')
            && s.chars().position(|c| c == '[') != s.chars().position(|c| c == ']'))
    {
        return false;
    }

    s.chars().all(is_bare_char)
}

fn would_parse_as_number(s: &str) -> bool {
    // "-123", "+123", "123", "-12.3", "12.3", "+12.3" would be interpreted as numbers
    // Also "-123.0-123.0" (jammed tokens) would cause this function to return "true", since
    // the parser would error out with jammed tokens without quotes...

    // Scan if the string begins with a number literal (including sign and decimal point)
    // First character: "." (decimal point), "-", "+" or a digit
    // Followed by zero or more digits, optionally followed by a decimal point and more digits
    let first = s.chars().next().unwrap_or(' ');
    if !first.is_ascii_digit() && first != '.' && first != '-' && first != '+' {
        return false;
    }

    // Scan for more digits
    //
    // -12.3
    //  ^
    let index = if first == '.' {
        // Decimal point found, scan for more digits
        1
    } else {
        let Some(index) = s[1..].chars().position(|c| !c.is_ascii_digit()) else {
            return true;
        };
        index + 1
    };

    // Check if IP address
    if s[index..].chars().find(|c| !c.is_ascii_digit()) == Some('.') {
        return false;
    }

    // At this point, we know the string starts with a digit or decimal point
    //
    // 12.3  123abc 123.abc
    //   ^      ^      ^
    //
    // BUT:
    //
    // .abc
    //  ^
    //
    // ...is not a valid number literal
    index > 1 || first != '.'
}

/// Determines whether a value should be quoted based on the quote style.
pub fn should_quote(s: &str, style: QuoteStyle) -> bool {
    match style {
        QuoteStyle::AlwaysDouble => true,
        QuoteStyle::Auto => {
            // Must quote if:
            // - Empty string
            // - Contains interpolation
            // - Contains characters not valid in bare strings
            // - Would be ambiguous with other token types (true, false, numbers)
            if s.is_empty() || s.contains("{{") || !is_valid_bare_string(s) {
                return true;
            }
            // Would be parsed as a different token type
            if matches!(
                s,
                "true" | "false" | "in" | "==" | "!=" | "~" | "~=" | "snippet" | "match"
            ) {
                return true;
            }
            // Would be parsed as a number
            if would_parse_as_number(s) {
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

/// Formats a string value, using raw string syntax if `is_raw` is true.
///
/// Raw strings are output as `r"..."` with no escape processing.
/// Otherwise, delegates to [`format_string_value`].
pub fn format_string_value_raw(s: &str, style: QuoteStyle, is_raw: bool) -> String {
    if is_raw {
        format!("r\"{}\"", s)
    } else {
        format_string_value(s, style)
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
        assert!(is_valid_bare_string("key=value"));

        assert!(!is_valid_bare_string(""));
        assert!(!is_valid_bare_string("hello world"));
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

        // Keywords should be quoted
        assert!(should_quote("true", QuoteStyle::Auto));
        assert!(should_quote("false", QuoteStyle::Auto));
        assert!(should_quote("in", QuoteStyle::Auto));

        // Numbers should be quoted (to distinguish from Number tokens)
        assert!(should_quote("42", QuoteStyle::Auto));
        // "3.14" is split by the lexer into Number("3") + StringBare(".14"),
        // so as a single value it doesn't need quoting (would parse as a
        // Number token followed by a bare string continuation)

        // Interpolation should be quoted
        assert!(should_quote("{{var}}", QuoteStyle::Auto));
        assert!(should_quote("prefix {{var}} suffix", QuoteStyle::Auto));

        // Jammed numbers should be quoted
        assert!(should_quote("12ab", QuoteStyle::Auto));
        assert!(should_quote("10s", QuoteStyle::Auto));
        assert!(should_quote("1s", QuoteStyle::Auto));
    }

    #[test]
    fn test_should_quote_always_double() {
        assert!(should_quote("hello", QuoteStyle::AlwaysDouble));
        assert!(should_quote("example.com", QuoteStyle::AlwaysDouble));
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
