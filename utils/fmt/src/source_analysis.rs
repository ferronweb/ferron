use std::collections::HashSet;

use ferronconf::{Lexer, TokenKind};

/// Results of analyzing the original source text.
#[derive(Debug, Clone)]
pub struct SourceAnalysis {
    /// Positions `(line, column)` of `StringRaw` token starts (1-indexed).
    raw_positions: HashSet<(usize, usize)>,
    /// Positions `(line, column)` of line-continuation backslashes (1-indexed).
    continuation_positions: HashSet<(usize, usize)>,
}

impl SourceAnalysis {
    /// Returns `true` if the token at the given span was a raw string.
    pub fn is_raw(&self, line: usize, column: usize) -> bool {
        self.raw_positions.contains(&(line, column))
    }

    /// Returns `true` if there was a line continuation at the given position.
    pub fn has_continuation(&self, line: usize, column: usize) -> bool {
        self.continuation_positions.contains(&(line, column))
    }

    /// Returns the set of continuation positions.
    #[allow(dead_code)]
    pub fn continuation_positions(&self) -> &HashSet<(usize, usize)> {
        &self.continuation_positions
    }
}

/// Analyzes the input text to identify raw string positions and line continuations.
pub fn analyze_input(input: &str) -> SourceAnalysis {
    let raw_positions = analyze_raw_positions(input);
    let continuation_positions = analyze_continuation_positions(input);

    SourceAnalysis {
        raw_positions,
        continuation_positions,
    }
}

/// Scans the input with the lexer to find all `StringRaw` token positions.
fn analyze_raw_positions(input: &str) -> HashSet<(usize, usize)> {
    let mut positions = HashSet::new();
    let mut lexer = Lexer::new(input);

    loop {
        match lexer.next_or_error() {
            Ok(Some(token)) => {
                if token.kind == TokenKind::StringRaw {
                    positions.insert((token.span.line, token.span.column));
                }
                if token.kind == TokenKind::EOF {
                    break;
                }
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }

    positions
}

/// Scans the input character-by-character to find line continuation backslashes.
///
/// A line continuation is `\` at end of line, optionally followed by whitespace
/// and/or a comment (`# ...`), then a newline. Must not be inside a quoted string.
fn analyze_continuation_positions(input: &str) -> HashSet<(usize, usize)> {
    let mut positions = HashSet::new();
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut i = 0;
    let mut line: usize = 1;
    let mut column: usize = 1;
    let mut in_quoted = false;

    while i < len {
        let c = chars[i];

        if in_quoted {
            if c == '\\' {
                // Skip escaped character inside quoted string
                i += 2;
                column += 2;
                continue;
            }
            if c == '"' {
                in_quoted = false;
            }
            if c == '\n' {
                line += 1;
                column = 1;
            } else {
                column += 1;
            }
            i += 1;
            continue;
        }

        if c == '"' {
            if i > 0 && chars[i - 1] == 'r' {
                // This is the `"` of a raw string — skip to closing `"`
                in_quoted = true; // reuse flag to skip content
            } else {
                in_quoted = true;
            }
            i += 1;
            column += 1;
            continue;
        }

        if c == '\\' {
            // Check if this is a line continuation
            let saved_i = i;
            let saved_line = line;
            let saved_column = column;

            i += 1;
            column += 1;

            // Skip whitespace after `\` (but not newlines)
            while i < len && chars[i].is_whitespace() && chars[i] != '\n' && chars[i] != '\r' {
                i += 1;
                column += 1;
            }

            // Skip optional comment
            if i < len && chars[i] == '#' {
                while i < len && chars[i] != '\n' && chars[i] != '\r' {
                    i += 1;
                    column += 1;
                }
            }

            // If we reached a newline, this is a valid continuation
            if i < len && (chars[i] == '\n' || chars[i] == '\r') {
                positions.insert((saved_line, saved_column));
                // Don't advance past the newline here — the main loop will handle it
                continue;
            }

            // Not a continuation — restore state
            i = saved_i;
            line = saved_line;
            column = saved_column;
        }

        if c == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
        i += 1;
    }

    positions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_analyze_raw_positions() {
        let input = "root \"/var/www\"\nproxy ~ r\"^/api/v1$\"\n";
        let analysis = analyze_input(input);
        // proxy is on line 2, "r" of r"..." starts at column 9
        assert!(analysis.is_raw(2, 9));
        assert!(!analysis.is_raw(1, 6));
    }

    #[test]
    fn test_analyze_continuation_basic() {
        let input = "proxy http://localhost:3000 \\\n    http://localhost:3001\n";
        let analysis = analyze_input(input);
        assert!(analysis.has_continuation(1, 29));
    }

    #[test]
    fn test_analyze_continuation_with_comment() {
        let input = "proxy http://localhost:3000 \\ # first backend\n    http://localhost:3001\n";
        let analysis = analyze_input(input);
        assert!(analysis.has_continuation(1, 29));
    }

    #[test]
    fn test_no_continuation_inside_quoted_string() {
        let input = "header \"value\\\\\\n\"\n";
        let analysis = analyze_input(input);
        assert!(analysis.continuation_positions().is_empty());
    }

    #[test]
    fn test_analyze_multiple_continuations() {
        let input = "proxy \\\n    http://a.com \\\n    http://b.com\n";
        let analysis = analyze_input(input);
        assert!(analysis.has_continuation(1, 7));
        assert!(analysis.has_continuation(2, 18));
    }

    #[test]
    fn test_no_continuation_backslash_not_at_eol() {
        let input = "root /var/www\\html\n";
        let analysis = analyze_input(input);
        // `\` followed by `h`, not a newline — not a continuation
        assert!(analysis.continuation_positions().is_empty());
    }
}
