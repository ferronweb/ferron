//! Parser for `ferron.conf` configuration files.
//!
//! The parser consumes tokens from the [`Lexer`](crate::lexer::Lexer) and builds
//! an abstract syntax tree (AST) representing the configuration structure.
//!
//! # Error Handling
//!
//! Parse errors include source location information via [`Span`](crate::lexer::Span)
//! for precise error reporting.
//!
//! The parser is an internal implementation detail. Users should parse
//! configuration via [`Config::from_str`](crate::Config).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use crate::ast::*;
use crate::lexer::{Span, Token, TokenKind};

/// A parse error with message and source location.
///
/// Errors are returned when the parser encounters invalid syntax
/// or unexpected tokens.
#[derive(Debug, Clone)]
pub struct ParseError {
    /// A human-readable error message.
    pub message: String,
    /// The source location where the error occurred.
    pub span: Span,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Parse error at line {}, column {}: {}",
            self.span.line, self.span.column, self.message
        )
    }
}

impl std::error::Error for ParseError {}

/// The parser that builds an AST from a token stream.
///
/// The parser implements a recursive descent parsing strategy,
/// where each grammar rule is implemented as a separate method.
pub(crate) struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

/// Internal enum for tracking parsed number types.
enum ParsedNumber {
    Integer(i64),
    Float(f64),
}

impl Parser {
    /// Creates a new parser for the given tokens.
    ///
    /// # Arguments
    ///
    /// * `tokens` - The token stream from the lexer
    pub fn new(tokens: Vec<Token>) -> Parser {
        Parser { tokens, pos: 0 }
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn advance(&mut self) -> &Token {
        self.pos += 1;
        &self.tokens[self.pos - 1]
    }

    fn advance_owned(&mut self) -> Token {
        let span = self.tokens[self.pos].span;
        let token = std::mem::replace(
            &mut self.tokens[self.pos],
            Token {
                kind: TokenKind::EOF,
                lexeme: None,
                span,
            },
        );
        self.pos += 1;
        token
    }

    fn skip_ignorable_tokens(&mut self) {
        while self.peek().kind == TokenKind::Comment {
            self.advance();
        }
    }

    fn check(&self, kind: TokenKind) -> bool {
        self.peek().kind == kind
    }

    fn token_text(token: &Token) -> Result<&str, ParseError> {
        token.lexeme.as_deref().ok_or(ParseError {
            message: format!("Missing token text for {:?}", token.kind),
            span: token.span,
        })
    }

    fn parse_integer(text: &str, span: Span) -> Result<i64, ParseError> {
        text.parse::<i64>().map_err(|_| ParseError {
            message: "Invalid number".into(),
            span,
        })
    }

    fn parse_decimal(
        integer: i64,
        decimal_text: &str,
        negative: bool,
        span: Span,
    ) -> Result<f64, ParseError> {
        let decimal = Self::parse_integer(decimal_text, span)?;
        let number = integer as f64 + decimal as f64 / 10.0_f64.powi(decimal_text.len() as i32);
        Ok(if negative { -number } else { number })
    }

    fn parse_number_literal(
        &mut self,
        integer_text: &str,
        span: Span,
        negative: bool,
    ) -> Result<ParsedNumber, ParseError> {
        let integer = Self::parse_integer(integer_text, span)?;

        if self.check(TokenKind::Dot) {
            self.advance();
            let token = self.advance_owned();
            if token.kind != TokenKind::Number {
                return Err(ParseError {
                    message: "Invalid number".into(),
                    span: token.span,
                });
            }

            let decimal_text = Self::token_text(&token)?;
            let number = Self::parse_decimal(integer, decimal_text, negative, token.span)?;
            Ok(ParsedNumber::Float(number))
        } else {
            Ok(ParsedNumber::Integer(if negative {
                -integer
            } else {
                integer
            }))
        }
    }

    fn is_value_start(&self) -> bool {
        matches!(
            self.peek().kind,
            TokenKind::StringBare
                | TokenKind::StringQuoted
                | TokenKind::Number
                | TokenKind::Boolean
                | TokenKind::InterpStart
                | TokenKind::Minus
        )
    }

    fn expect(&mut self, kind: TokenKind) -> Result<(), ParseError> {
        if self.check(kind) {
            self.advance();
            Ok(())
        } else {
            Err(ParseError {
                message: format!("Expected {:?}, got {:?}", kind, self.peek().kind),
                span: self.peek().span,
            })
        }
    }

    fn expect_identifier(&mut self) -> Result<String, ParseError> {
        let token = self.advance_owned();

        if token.kind == TokenKind::Identifier {
            Ok(token.lexeme.ok_or(ParseError {
                message: "Missing token text for Identifier".into(),
                span: token.span,
            })?)
        } else {
            Err(ParseError {
                message: format!("Expected identifier, got {:?}", token.kind),
                span: token.span,
            })
        }
    }

    fn parse_operator(&mut self) -> Result<Operator, ParseError> {
        let token = self.advance();

        match token.kind {
            TokenKind::OpEq => Ok(Operator::Eq),
            TokenKind::OpNeq => Ok(Operator::NotEq),
            TokenKind::OpRegex => Ok(Operator::Regex),
            TokenKind::OpNotRegex => Ok(Operator::NotRegex),
            TokenKind::OpIn => Ok(Operator::In),

            _ => Err(ParseError {
                message: "Invalid operator".into(),
                span: token.span,
            }),
        }
    }

    fn parse_operand(&mut self) -> Result<Operand, ParseError> {
        let token = self.advance_owned();
        let span = token.span;

        match token.kind {
            TokenKind::Identifier => {
                let mut group = Vec::new();
                group.push(token.lexeme.ok_or(ParseError {
                    message: "Missing token text for Identifier".into(),
                    span,
                })?);

                while self.check(TokenKind::Dot) {
                    self.advance();
                    let token = self.advance_owned();
                    if token.kind == TokenKind::Identifier {
                        group.push(token.lexeme.ok_or(ParseError {
                            message: "Missing token text for Identifier".into(),
                            span: token.span,
                        })?);
                    } else {
                        return Err(ParseError {
                            message: "Invalid identifier".into(),
                            span: token.span,
                        });
                    }
                }

                Ok(Operand::Identifier(group, span))
            }

            TokenKind::StringBare | TokenKind::StringQuoted => Ok(Operand::String(
                token.lexeme.ok_or(ParseError {
                    message: format!("Missing token text for {:?}", token.kind),
                    span,
                })?,
                span,
            )),

            TokenKind::Number => {
                let integer_text = token.lexeme.as_deref().ok_or(ParseError {
                    message: "Missing token text for Number".into(),
                    span,
                })?;
                match self.parse_number_literal(integer_text, span, false)? {
                    ParsedNumber::Integer(integer) => Ok(Operand::Integer(integer, span)),
                    ParsedNumber::Float(number) => Ok(Operand::Float(number, span)),
                }
            }

            _ => Err(ParseError {
                message: "Invalid operand".into(),
                span: token.span,
            }),
        }
    }

    fn parse_match_expression(&mut self) -> Result<MatcherExpression, ParseError> {
        let left = self.parse_operand()?;
        let op = self.parse_operator()?;
        let right = self.parse_operand()?;
        let span = left.span();

        Ok(MatcherExpression {
            left,
            op,
            right,
            span,
        })
    }

    fn parse_match_block(&mut self) -> Result<MatchBlock, ParseError> {
        let start_span = self.peek().span; // Span of the 'match' keyword
        self.expect(TokenKind::Match)?;

        let matcher = self.expect_identifier()?;
        let mut expr = Vec::new();

        self.expect(TokenKind::LBrace)?;

        while !self.check(TokenKind::RBrace) {
            expr.push(self.parse_match_expression()?);
        }

        self.expect(TokenKind::RBrace)?;

        Ok(MatchBlock {
            matcher,
            expr,
            span: start_span,
        })
    }

    fn parse_snippet_block(&mut self) -> Result<SnippetBlock, ParseError> {
        let start_span = self.peek().span; // Span of the 'snippet' keyword
        self.expect(TokenKind::Snippet)?;

        let name = self.expect_identifier()?;
        let block = self.parse_block()?;

        Ok(SnippetBlock {
            name,
            block,
            span: start_span,
        })
    }

    fn parse_block(&mut self) -> Result<Block, ParseError> {
        let start_span = self.peek().span; // Span of the '{'
        self.expect(TokenKind::LBrace)?;

        let mut statements = Vec::new();

        while !self.check(TokenKind::RBrace) {
            statements.push(self.parse_statement(false)?);
        }

        self.expect(TokenKind::RBrace)?;

        Ok(Block {
            statements,
            span: start_span,
        })
    }

    fn parse_string_with_interpolation(input: &str, span: Span) -> Result<Value, ParseError> {
        let mut parts = Vec::new();
        let bytes = input.as_bytes();
        let mut cursor = 0;
        let mut literal_start = 0;

        while cursor + 1 < bytes.len() {
            if bytes[cursor] == b'{' && bytes[cursor + 1] == b'{' {
                if literal_start < cursor {
                    parts.push(StringPart::Literal(
                        input[literal_start..cursor].to_string(),
                    ));
                }

                let expr_start = cursor + 2;
                cursor = expr_start;

                while cursor + 1 < bytes.len()
                    && !(bytes[cursor] == b'}' && bytes[cursor + 1] == b'}')
                {
                    cursor += 1;
                }

                if cursor + 1 >= bytes.len() {
                    return Err(ParseError {
                        message: format!(
                            "Expected {{ ... }} pair, got {:?}",
                            &input[expr_start - 2..]
                        ),
                        span,
                    });
                }

                let expr = input[expr_start..cursor]
                    .trim()
                    .split('.')
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>();
                parts.push(StringPart::Expression(expr));

                cursor += 2;
                literal_start = cursor;
            } else {
                cursor += 1;
            }
        }

        if parts.is_empty() {
            return Ok(Value::String(input.to_string(), span));
        }

        if literal_start < input.len() {
            parts.push(StringPart::Literal(input[literal_start..].to_string()));
        }

        Ok(Value::InterpolatedString(parts, span))
    }

    fn parse_value(&mut self) -> Result<Value, ParseError> {
        let token = self.advance_owned();
        let span = token.span;

        match token.kind {
            TokenKind::StringBare | TokenKind::StringQuoted => {
                let text = token.lexeme.as_deref().ok_or(ParseError {
                    message: format!("Missing token text for {:?}", token.kind),
                    span,
                })?;
                Self::parse_string_with_interpolation(text, token.span)
            }

            TokenKind::InterpStart => {
                let mut parts = Vec::new();

                loop {
                    let advanced = self.advance_owned();
                    if advanced.kind == TokenKind::InterpEnd {
                        break;
                    } else if advanced.kind == TokenKind::Identifier {
                        parts.push(advanced.lexeme.ok_or(ParseError {
                            message: "Missing token text for Identifier".into(),
                            span: advanced.span,
                        })?);
                    } else if advanced.kind != TokenKind::Dot {
                        return Err(ParseError {
                            message: "Invalid interpolation".into(),
                            span: advanced.span,
                        });
                    }
                }

                Ok(Value::InterpolatedString(
                    vec![StringPart::Expression(parts)],
                    span,
                ))
            }

            TokenKind::Minus => {
                let token = self.advance_owned();
                if token.kind != TokenKind::Number {
                    return Err(ParseError {
                        message: "Invalid number".into(),
                        span: token.span,
                    });
                }
                let integer_text = token.lexeme.as_deref().ok_or(ParseError {
                    message: "Missing token text for Number".into(),
                    span: token.span,
                })?;
                match self.parse_number_literal(integer_text, span, true)? {
                    ParsedNumber::Integer(integer) => Ok(Value::Integer(integer, span)),
                    ParsedNumber::Float(number) => Ok(Value::Float(number, span)),
                }
            }

            TokenKind::Number => {
                let integer_text = token.lexeme.as_deref().ok_or(ParseError {
                    message: "Missing token text for Number".into(),
                    span,
                })?;
                match self.parse_number_literal(integer_text, span, false)? {
                    ParsedNumber::Integer(integer) => Ok(Value::Integer(integer, span)),
                    ParsedNumber::Float(number) => Ok(Value::Float(number, span)),
                }
            }

            TokenKind::Boolean => Ok(Value::Boolean(
                token.lexeme.as_deref().ok_or(ParseError {
                    message: "Missing token text for Boolean".into(),
                    span,
                })? == "true",
                span,
            )),

            _ => Err(ParseError {
                message: "Invalid value".into(),
                span: token.span,
            }),
        }
    }

    fn parse_directive(&mut self) -> Result<Directive, ParseError> {
        let name_token = self.advance_owned();
        let span = name_token.span;
        let name = name_token.lexeme.ok_or(ParseError {
            message: "Missing token text for Identifier".into(),
            span,
        })?;
        let mut args = Vec::new();

        while self.is_value_start() {
            args.push(self.parse_value()?);
        }

        let block = if self.check(TokenKind::LBrace) {
            Some(self.parse_block()?)
        } else {
            None
        };

        Ok(Directive {
            name,
            args,
            block,
            span,
        })
    }

    fn parse_host_pattern(&mut self) -> Result<HostPattern, ParseError> {
        let start_span = self.peek().span; // Span of the first token in the host pattern
        let mut labels = Vec::new();
        let mut port = None;
        let mut protocol = None;
        let mut is_ipv6 = false;

        loop {
            let token = self.advance_owned();

            match &token.kind {
                TokenKind::LBracket => {
                    is_ipv6 = true;
                }

                TokenKind::RBracket if is_ipv6 => {
                    labels = vec![labels.join("")];
                    is_ipv6 = false;
                }

                TokenKind::Identifier
                | TokenKind::Number
                | TokenKind::Match
                | TokenKind::Snippet
                | TokenKind::Boolean => {
                    labels.push(token.lexeme.ok_or(ParseError {
                        message: format!("Missing token text for {:?}", token.kind),
                        span: token.span,
                    })?);
                }

                TokenKind::Star if !is_ipv6 => {
                    labels.push("*".into());
                }

                TokenKind::Colon if is_ipv6 => {
                    labels.push(":".into());
                }

                TokenKind::StringBare if is_ipv6 => {
                    labels.push(token.lexeme.ok_or(ParseError {
                        message: "Missing token text for StringBare".into(),
                        span: token.span,
                    })?);
                }

                TokenKind::StringBare => {
                    let lexeme = token.lexeme.ok_or(ParseError {
                        message: "Missing token text for StringBare".into(),
                        span: token.span,
                    })?;
                    labels.extend(lexeme.split('.').map(|s| s.to_string()));
                    if let Some(popped) = labels.pop() {
                        if popped.contains(':') {
                            let (host, port_str) =
                                popped.split_once(':').ok_or_else(|| ParseError {
                                    message: "Invalid host:port format".into(),
                                    span: token.span,
                                })?;
                            labels.push(host.into());
                            port = Some(port_str.parse::<u16>().map_err(|_| ParseError {
                                message: "Invalid port number".into(),
                                span: self.peek().span,
                            })?);
                        } else {
                            labels.push(popped);
                        }
                    }
                }

                _ => {
                    return Err(ParseError {
                        message: "Invalid host label".into(),
                        span: token.span,
                    });
                }
            }

            if is_ipv6 {
                continue;
            }

            if self.check(TokenKind::Dot) {
                self.advance();
            } else if self.check(TokenKind::Colon) {
                if !is_ipv6 {
                    self.advance();
                    let port_token = self.advance_owned();
                    port = Some(match port_token.kind {
                        TokenKind::Number => Self::token_text(&port_token)?
                            .parse::<u16>()
                            .map_err(|_| ParseError {
                                message: "Invalid port number".into(),
                                span: port_token.span,
                            })?,
                        _ => {
                            return Err(ParseError {
                                message: "Invalid port number".into(),
                                span: port_token.span,
                            });
                        }
                    });
                    break;
                }
            } else if self.check(TokenKind::StringBare)
                || self.check(TokenKind::Number)
                || self.check(TokenKind::Star)
            {
                if !is_ipv6 {
                    if protocol.is_none() && labels.len() == 1 {
                        protocol = Some(labels.pop().ok_or_else(|| ParseError {
                            message: "Invalid host label".into(),
                            span: self.peek().span,
                        })?);
                    } else {
                        return Err(ParseError {
                            message: "Invalid host label".into(),
                            span: self.peek().span,
                        });
                    }
                }
            } else if !is_ipv6 && self.check(TokenKind::LBracket) {
                if protocol.is_none() && labels.len() == 1 {
                    protocol = Some(labels.pop().ok_or_else(|| ParseError {
                        message: "Invalid host label".into(),
                        span: self.peek().span,
                    })?);
                } else {
                    return Err(ParseError {
                        message: "Invalid host label".into(),
                        span: self.peek().span,
                    });
                }
            } else if is_ipv6 && self.check(TokenKind::RBracket) {
                // IPv6 addresses are enclosed in brackets
            } else {
                break;
            }
        }

        let labels = if labels.len() == 1 && labels[0].contains(":") {
            let label = labels.pop().ok_or_else(|| ParseError {
                message: "Invalid host format".into(),
                span: self.peek().span,
            })?;
            let Ok(addr) = label.parse::<Ipv6Addr>() else {
                return Err(ParseError {
                    message: "Invalid IPv6 address".into(),
                    span: self.peek().span,
                });
            };
            HostLabels::IpAddr(IpAddr::V6(addr))
        } else if labels.len() == 4 {
            let mut octets = [0; 4];
            let mut is_ipv4 = true;
            for (i, label) in labels.iter().enumerate() {
                match label.parse::<u8>() {
                    Ok(octet) => octets[i] = octet,
                    Err(_) => {
                        is_ipv4 = false;
                        break;
                    }
                }
            }

            if is_ipv4 {
                HostLabels::IpAddr(IpAddr::V4(Ipv4Addr::from(octets)))
            } else {
                HostLabels::Hostname(labels)
            }
        } else if labels.len() == 1 && labels[0] == "*" {
            HostLabels::Wildcard
        } else {
            HostLabels::Hostname(labels)
        };

        Ok(HostPattern {
            labels,
            port,
            protocol,
            span: start_span,
        })
    }

    fn parse_host_block(&mut self) -> Result<HostBlock, ParseError> {
        let start_span = self.peek().span; // Span of the first host pattern or token that begins the host block
        let mut hosts = Vec::new();
        loop {
            let host = self.parse_host_pattern()?;
            hosts.push(host);

            if self.check(TokenKind::Comma) {
                self.advance();
            } else {
                break;
            }
        }
        let block = self.parse_block()?;

        Ok(HostBlock {
            hosts,
            block,
            span: start_span,
        })
    }

    /// Performs a lookahead scan to determine if tokens form a host block.
    ///
    /// This is needed to disambiguate between directives and host blocks
    /// at the top level.
    fn scans_as_host_block(&self) -> bool {
        let mut i = self.pos;
        let mut in_ipv6 = false;
        let mut suspect_not = false;

        while i < self.tokens.len() {
            let token = &self.tokens[i];
            match token.kind {
                TokenKind::LBrace => return true,
                TokenKind::LBracket => {
                    in_ipv6 = true;
                    suspect_not = false;
                }
                TokenKind::RBracket => {
                    in_ipv6 = false;
                    i += 1;
                    continue;
                }
                TokenKind::StringQuoted
                | TokenKind::Match
                | TokenKind::Snippet
                | TokenKind::Boolean
                    if !in_ipv6 =>
                {
                    if suspect_not {
                        return false;
                    }
                    suspect_not = true;
                }
                _ => {}
            }

            if in_ipv6 {
                i += 1;
                continue;
            }

            // Adjacency checks (outside IPv6)
            if i > self.pos {
                let prev = &self.tokens[i - 1];
                // If prev was a label part
                match prev.kind {
                    TokenKind::Identifier
                    | TokenKind::Match
                    | TokenKind::Snippet
                    | TokenKind::Boolean
                    | TokenKind::Number
                    | TokenKind::StringBare
                    | TokenKind::Star
                    | TokenKind::RBracket => {
                        // Current must be separator or valid Protocol transition
                        match token.kind {
                            TokenKind::Dot
                            | TokenKind::Colon
                            | TokenKind::LBracket
                            | TokenKind::Comma
                            | TokenKind::LBrace => {} // OK
                            TokenKind::StringBare | TokenKind::Number | TokenKind::Star => {} // Protocol transition OK
                            _ => return false,
                        }
                        suspect_not = false;
                    }
                    _ => {}
                }
            }
            i += 1;
        }
        false
    }

    /// Convenience wrapper for [`scans_as_host_block`](Self::scans_as_host_block).
    fn looks_like_host(&self) -> bool {
        self.scans_as_host_block()
    }

    /// Parses a single statement at the current position.
    ///
    /// # Arguments
    ///
    /// * `top_level` - If `true`, host blocks are allowed; if `false`, only
    ///   directives, match blocks, snippet blocks, and nested blocks are allowed
    ///
    /// # Returns
    ///
    /// A [`Statement`] on success, or a [`ParseError`] if parsing fails.
    fn parse_statement(&mut self, top_level: bool) -> Result<Statement, ParseError> {
        self.skip_ignorable_tokens(); // Skip comments before parsing a statement

        match self.peek().kind {
            TokenKind::Number
            | TokenKind::Star
            | TokenKind::LBracket
            | TokenKind::Identifier
            | TokenKind::Match
            | TokenKind::Snippet
            | TokenKind::Boolean
                if top_level && self.looks_like_host() =>
            {
                Ok(Statement::HostBlock(self.parse_host_block()?))
            }

            TokenKind::Match => Ok(Statement::MatchBlock(self.parse_match_block()?)),

            TokenKind::Identifier => Ok(Statement::Directive(self.parse_directive()?)),

            TokenKind::LBrace => Ok(Statement::GlobalBlock(self.parse_block()?)),

            TokenKind::Snippet => Ok(Statement::SnippetBlock(self.parse_snippet_block()?)),

            _ => Err(ParseError {
                message: format!(
                    "Unexpected token {:?}, expected 'match', 'identifier', '[', '@', 'number' or '*'",
                    self.peek().kind
                ),
                span: self.peek().span,
            }),
        }
    }

    /// Parses a complete configuration file into a [`Config`] AST.
    ///
    /// This is the main entry point for parsing. It handles:
    /// - Leading comments
    /// - Multiple top-level statements
    /// - EOF validation
    ///
    /// # Returns
    ///
    /// A [`Config`] containing all parsed statements, or a [`ParseError`]
    /// if parsing fails.
    pub fn parse_config(&mut self) -> Result<Config, ParseError> {
        let mut statements = Vec::new();
        self.skip_ignorable_tokens(); // Skip leading comments

        while !self.check(TokenKind::EOF) {
            self.skip_ignorable_tokens(); // Skip comments between statements
            statements.push(self.parse_statement(true)?);
        }

        Ok(Config { statements })
    }
}
