//! Lexer for `ferron.conf` configuration files.
//!
//! The lexer converts raw input text into a sequence of [`Token`]s,
//! discarding whitespace and comments. Tokens are then consumed by
//! the [`Parser`](crate::parser::Parser) to build an AST.
//!
//! The lexer is an internal implementation detail. Users should parse
//! configuration via [`Config::from_str`](crate::Config).

#[derive(Debug, Copy, Clone, PartialEq)]
pub(crate) enum TokenKind {
    /// An identifier (e.g., directive name, hostname label).
    Identifier,
    /// A numeric literal (integer or decimal part).
    Number,
    /// A double-quoted string (e.g., `"hello world"`).
    StringQuoted,
    /// A bare (unquoted) string (e.g., `example.com`).
    StringBare,
    /// A boolean literal (`true` or `false`).
    Boolean,

    /// Left brace `{`.
    LBrace,
    /// Right brace `}`.
    RBrace,
    /// Left bracket `[`.
    LBracket,
    /// Right bracket `]`.
    RBracket,
    /// Colon `:`.
    Colon,
    /// Dot `.`.
    Dot,
    /// Star/wildcard `*`.
    Star,
    /// Comma `,`.
    Comma,
    /// Minus sign `-` (for negative numbers).
    Minus,

    /// Equality operator `==`.
    OpEq,
    /// Inequality operator `!=`.
    OpNeq,
    /// Regex match operator `~`.
    OpRegex,
    /// Negated regex operator `!~`.
    OpNotRegex,
    /// Membership operator `in`.
    OpIn,

    /// The `match` keyword.
    Match,
    /// The `snippet` keyword.
    Snippet,

    /// Interpolation start `{{`.
    InterpStart,
    /// Interpolation end `}}`.
    InterpEnd,

    /// A comment (skipped during parsing).
    Comment,
    /// End of file marker.
    #[allow(clippy::upper_case_acronyms)]
    EOF,
}

/// A source location (line and column) for error reporting.
///
/// Spans are attached to tokens and AST nodes to track their
/// position in the original source file.
#[derive(Copy, Debug, Clone)]
pub struct Span {
    /// The 1-indexed line number.
    pub line: usize,
    /// The 1-indexed column number.
    pub column: usize,
}

impl PartialEq for Span {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

/// A token produced by the lexer.
#[derive(Debug, Clone)]
pub(crate) struct Token {
    /// The type of this token.
    pub kind: TokenKind,
    /// The optional lexeme text (for tokens with values).
    pub lexeme: Option<String>,
    /// The source location of this token.
    pub span: Span,
}

impl Token {
    /// Creates a token without a lexeme (for punctuation and keywords).
    fn bare(kind: TokenKind, span: Span) -> Self {
        Token {
            kind,
            lexeme: None,
            span,
        }
    }

    /// Creates a token with a lexeme value.
    fn with_lexeme(kind: TokenKind, lexeme: String, span: Span) -> Self {
        Token {
            kind,
            lexeme: Some(lexeme),
            span,
        }
    }
}

/// The lexer that converts source text into tokens.
///
/// The lexer implements [`Iterator`] to produce tokens one at a time.
/// It tracks position (line/column) for error reporting and handles:
/// - Whitespace skipping
/// - Comment skipping (lines starting with `#`)
/// - String parsing (quoted and bare)
/// - Number parsing
/// - Identifier and keyword recognition
pub(crate) struct Lexer<'a> {
    chars: std::str::Chars<'a>,
    current: Option<char>,
    next: Option<char>,
    line: usize,
    column: usize,
    prev_token: Option<TokenKind>,
}

impl<'a> Lexer<'a> {
    /// Creates a new lexer for the given input string.
    ///
    /// # Arguments
    ///
    /// * `input` - The source code to tokenize
    pub fn new(input: &'a str) -> Self {
        let mut chars = input.chars();
        let current = chars.next();
        let next = chars.next();

        Lexer {
            chars,
            current,
            next,
            line: 1,
            column: 1,
            prev_token: None,
        }
    }

    fn advance(&mut self) {
        if let Some(c) = self.current {
            if c == '\n' {
                self.line += 1;
                self.column = 1;
            } else {
                self.column += 1;
            }
        }

        self.current = self.next;
        self.next = self.chars.next();
    }

    fn peek(&self) -> Option<char> {
        self.next
    }

    fn skip_whitespace(&mut self) -> bool {
        let mut had_newlines = false;
        while matches!(self.current, Some(c) if c.is_whitespace()) {
            had_newlines = had_newlines || matches!(self.current, Some('\n') | Some('\r'));
            self.advance();
        }

        had_newlines
    }

    fn read_identifier(&mut self) -> String {
        let mut s = String::new();

        while let Some(c) = self.current {
            if c.is_alphanumeric() || c == '_' || c == '-' {
                s.push(c);
                self.advance();
            } else {
                break;
            }
        }

        s
    }

    fn identifier_token(&self, s: &str) -> TokenKind {
        match s {
            "match" => TokenKind::Match,
            "snippet" => TokenKind::Snippet,
            "true" | "false" => TokenKind::Boolean,
            "in" => TokenKind::OpIn,
            _ => TokenKind::Identifier,
        }
    }

    fn read_number(&mut self) -> String {
        let mut s = String::new();

        while let Some(c) = self.current {
            if c.is_ascii_digit() {
                s.push(c);
                self.advance();
            } else {
                break;
            }
        }

        s
    }

    fn read_string(&mut self) -> String {
        let mut s = String::new();

        self.advance(); // skip opening "

        while let Some(c) = self.current {
            if c == '"' {
                break;
            }

            if c == '\\' {
                self.advance();
                if let Some(escaped) = self.current {
                    match escaped {
                        'n' => s.push('\n'), // newline
                        'r' => s.push('\r'), // carriage return
                        't' => s.push('\t'), // tab
                        _ => s.push(escaped),
                    }
                }
            } else {
                s.push(c);
            }

            self.advance();
        }

        self.advance(); // closing "
        s
    }

    fn read_comment(&mut self) {
        while let Some(c) = self.current {
            if c == '\n' {
                break;
            }

            self.advance();
        }
    }

    fn is_bare_char(c: char) -> bool {
        c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':' | '/' | '*' | '+')
    }

    fn read_bare_string(&mut self) -> String {
        let mut s = String::new();

        while let Some(c) = self.current {
            if Self::is_bare_char(c) {
                s.push(c);
                self.advance();
            } else {
                break;
            }
        }

        s
    }

    /// Returns `true` if a bare string is allowed at this position.
    ///
    /// Bare strings are only allowed after certain token types to avoid
    /// ambiguity with host blocks.
    fn allow_bare_string(&self) -> bool {
        matches!(
            self.prev_token,
            Some(TokenKind::Identifier)
                | Some(TokenKind::Number)
                | Some(TokenKind::StringBare)
                | Some(TokenKind::OpRegex)
                | Some(TokenKind::OpNotRegex)
                | Some(TokenKind::OpEq)
                | Some(TokenKind::OpNeq)
                | Some(TokenKind::OpIn)
        )
    }

    /// Tokenizes the next token from the input.
    ///
    /// This is the main tokenization loop that:
    /// 1. Skips whitespace
    /// 2. Matches the next token based on the current character
    /// 3. Updates position tracking
    ///
    /// # Returns
    ///
    /// The next [`Token`], or `EOF` if at the end of input.
    pub fn next_token(&mut self) -> Token {
        loop {
            let had_newlines = self.skip_whitespace();

            let span = Span {
                line: self.line,
                column: self.column,
            };

            let token = match self.current {
                Some('{') if self.peek() == Some('{') => {
                    self.advance();
                    self.advance();
                    Token::bare(TokenKind::InterpStart, span)
                }

                Some('}') if self.peek() == Some('}') => {
                    self.advance();
                    self.advance();
                    Token::bare(TokenKind::InterpEnd, span)
                }

                Some('{') => {
                    self.advance();
                    Token::bare(TokenKind::LBrace, span)
                }

                Some('}') => {
                    self.advance();
                    Token::bare(TokenKind::RBrace, span)
                }

                Some('[') => {
                    self.advance();
                    Token::bare(TokenKind::LBracket, span)
                }

                Some(']') => {
                    self.advance();
                    Token::bare(TokenKind::RBracket, span)
                }

                Some(':') => {
                    self.advance();
                    Token::bare(TokenKind::Colon, span)
                }

                Some('.') => {
                    self.advance();
                    Token::bare(TokenKind::Dot, span)
                }

                Some(',') => {
                    self.advance();
                    Token::bare(TokenKind::Comma, span)
                }

                Some('*') => {
                    self.advance();
                    Token::bare(TokenKind::Star, span)
                }

                Some('"') => {
                    let value = self.read_string();
                    Token::with_lexeme(TokenKind::StringQuoted, value, span)
                }

                Some('#') => {
                    self.read_comment();
                    continue;
                }

                Some('=') if self.peek() == Some('=') => {
                    self.advance();
                    self.advance();
                    Token::bare(TokenKind::OpEq, span)
                }

                Some('!') if self.peek() == Some('=') => {
                    self.advance();
                    self.advance();
                    Token::bare(TokenKind::OpNeq, span)
                }

                Some('~') => {
                    self.advance();
                    Token::bare(TokenKind::OpRegex, span)
                }

                Some('!') if self.peek() == Some('~') => {
                    self.advance();
                    self.advance();
                    Token::bare(TokenKind::OpNotRegex, span)
                }

                Some(c) if c.is_ascii_digit() => {
                    let n = self.read_number();
                    Token::with_lexeme(TokenKind::Number, n, span)
                }

                Some('-') if self.peek().is_some_and(|p| p.is_ascii_digit()) => {
                    self.advance();
                    Token::bare(TokenKind::Minus, span)
                }

                Some(c) if Self::is_bare_char(c) && self.allow_bare_string() && !had_newlines => {
                    let value = self.read_bare_string();
                    let kind = if value == "true" || value == "false" {
                        TokenKind::Boolean
                    } else {
                        TokenKind::StringBare
                    };
                    Token::with_lexeme(kind, value, span)
                }

                Some(c) if c.is_alphabetic() => {
                    let id = self.read_identifier();
                    let kind = self.identifier_token(&id);
                    Token::with_lexeme(kind, id, span)
                }

                None => Token::bare(TokenKind::EOF, span),

                _ => {
                    self.advance();
                    continue;
                }
            };

            self.prev_token = Some(token.kind);
            return token;
        }
    }
}

impl Iterator for Lexer<'_> {
    type Item = Token;

    /// Returns the next token in the input.
    ///
    /// Returns `None` after EOF has been returned once.
    fn next(&mut self) -> Option<Self::Item> {
        if self
            .prev_token
            .as_ref()
            .is_some_and(|k| *k == TokenKind::EOF)
        {
            // There was an EOF token already, so we don't need to return another one.
            return None;
        }

        Some(self.next_token())
    }
}
