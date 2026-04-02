//! Abstract Syntax Tree (AST) types for `ferron.conf` configuration files.
//!
//! This module defines the complete structure of a parsed configuration file,
//! including all statement types, values, and helper methods for navigation.
//!
//! # Example
//!
//! ```rust
//! use ferronconf::Config;
//! use std::str::FromStr;
//!
//! let input = r#"
//! example.com {
//!     root /var/www/example
//! }
//! "#;
//!
//! let config = Config::from_str(input).unwrap();
//! ```

mod display;

use crate::lexer::Span;
use crate::{Lexer, ParseError, Parser};
use std::fmt::Write;
use std::net::IpAddr;
use std::str::FromStr;

/// The root AST node representing a complete `ferron.conf` configuration file.
///
/// A `Config` contains a sequence of [`Statement`]s at the top level.
/// Use the helper methods to find specific directives or blocks.
///
/// # Example
///
/// ```rust
/// use ferronconf::Config;
/// use std::str::FromStr;
///
/// let config = Config::from_str("example.com { root /var/www }").unwrap();
/// let host_blocks = config.find_host_blocks();
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// The top-level statements in the configuration file.
    pub statements: Vec<Statement>,
}

impl Config {
    /// Finds all [`Directive`] statements with the given name.
    ///
    /// # Arguments
    ///
    /// * `name` - The directive name to search for (e.g., `"root"`, `"server_name"`)
    ///
    /// # Returns
    ///
    /// A vector of references to matching directives.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use ferronconf::Config;
    /// # use std::str::FromStr;
    /// let config = Config::from_str("root /var/www\nroot /var/www/other").unwrap();
    /// let roots = config.find_directives("root");
    /// assert_eq!(roots.len(), 2);
    /// ```
    pub fn find_directives(&self, name: &str) -> Vec<&Directive> {
        self.statements
            .iter()
            .filter_map(|stmt| {
                if let Statement::Directive(directive) = stmt {
                    if directive.name == name {
                        Some(directive)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect()
    }

    /// Finds all [`HostBlock`] statements in the configuration.
    ///
    /// # Returns
    ///
    /// A vector of references to all host blocks.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use ferronconf::Config;
    /// # use std::str::FromStr;
    /// let config = Config::from_str("example.com { root /var/www }").unwrap();
    /// let hosts = config.find_host_blocks();
    /// assert_eq!(hosts.len(), 1);
    /// ```
    pub fn find_host_blocks(&self) -> Vec<&HostBlock> {
        self.statements
            .iter()
            .filter_map(|stmt| {
                if let Statement::HostBlock(host_block) = stmt {
                    Some(host_block)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Finds all [`MatchBlock`] statements in the configuration.
    ///
    /// # Returns
    ///
    /// A vector of references to all match blocks.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use ferronconf::Config;
    /// # use std::str::FromStr;
    /// let config = Config::from_str("match rules { path == \"/api\" }").unwrap();
    /// let matchers = config.find_match_blocks();
    /// assert_eq!(matchers.len(), 1);
    /// ```
    pub fn find_match_blocks(&self) -> Vec<&MatchBlock> {
        self.statements
            .iter()
            .filter_map(|stmt| {
                if let Statement::MatchBlock(match_block) = stmt {
                    Some(match_block)
                } else {
                    None
                }
            })
            .collect()
    }
}

impl FromStr for Config {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let lexer = Lexer::new(s);
        let mut parser = Parser::new(lexer.collect());
        parser.parse_config()
    }
}

/// A statement in the configuration file.
///
/// Statements are the top-level building blocks of a `ferron.conf` file.
/// There are five types of statements, each serving a different purpose.
#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    /// A key-value directive with optional arguments and nested block.
    Directive(Directive),
    /// A host-specific configuration block (top-level only).
    HostBlock(HostBlock),
    /// A conditional match block for request attributes.
    MatchBlock(MatchBlock),
    /// A global configuration block (top-level only).
    GlobalBlock(Block),
    /// A reusable configuration snippet definition.
    SnippetBlock(SnippetBlock),
}

impl Statement {
    /// Returns the source span (line and column) of this statement.
    ///
    /// Useful for error reporting and debugging.
    pub fn span(&self) -> Span {
        match self {
            Statement::Directive(d) => d.span,
            Statement::HostBlock(h) => h.span,
            Statement::MatchBlock(m) => m.span,
            Statement::GlobalBlock(g) => g.span,
            Statement::SnippetBlock(s) => s.span,
        }
    }

    /// Returns `true` if this statement is a [`Directive`].
    pub fn is_directive(&self) -> bool {
        matches!(self, Statement::Directive(_))
    }

    /// Returns `true` if this statement is a [`HostBlock`].
    pub fn is_host_block(&self) -> bool {
        matches!(self, Statement::HostBlock(_))
    }

    /// Returns `true` if this statement is a [`MatchBlock`].
    pub fn is_match_block(&self) -> bool {
        matches!(self, Statement::MatchBlock(_))
    }

    /// Returns `true` if this statement is a global block.
    pub fn is_global_block(&self) -> bool {
        matches!(self, Statement::GlobalBlock(_))
    }

    /// Returns `true` if this statement is a [`SnippetBlock`].
    pub fn is_snippet_block(&self) -> bool {
        matches!(self, Statement::SnippetBlock(_))
    }
}

/// A configuration directive with a name, optional arguments, and an optional nested block.
///
/// Directives are the primary way to specify configuration values.
///
/// # Example
///
/// ```ferron
/// server_name example.com
/// max_connections 1000
/// root "{{app.root}}" {
///     index index.html
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Directive {
    /// The directive name (e.g., `"root"`, `"server_name"`).
    pub name: String,
    /// The directive arguments (values).
    pub args: Vec<Value>,
    /// An optional nested block of statements.
    pub block: Option<Block>,
    /// The source span of the directive.
    pub span: Span,
}

impl Directive {
    /// Gets the string argument at the given index.
    ///
    /// # Arguments
    ///
    /// * `index` - The zero-based index of the argument
    ///
    /// # Returns
    ///
    /// `Some(&str)` if the argument exists and is a string, `None` otherwise.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use ferronconf::Config;
    /// # use std::str::FromStr;
    /// let config = Config::from_str("server_name example.com").unwrap();
    /// if let Some(d) = config.find_directives("server_name").first() {
    ///     assert_eq!(d.get_string_arg(0), Some("example.com"));
    /// }
    /// ```
    pub fn get_string_arg(&self, index: usize) -> Option<&str> {
        self.args.get(index).and_then(|arg| {
            if let Value::String(s, _) = arg {
                Some(s.as_str())
            } else {
                None
            }
        })
    }

    /// Gets the integer argument at the given index.
    ///
    /// # Arguments
    ///
    /// * `index` - The zero-based index of the argument
    ///
    /// # Returns
    ///
    /// `Some(i64)` if the argument exists and is an integer, `None` otherwise.
    pub fn get_integer_arg(&self, index: usize) -> Option<i64> {
        self.args.get(index).and_then(|arg| {
            if let Value::Integer(i, _) = arg {
                Some(*i)
            } else {
                None
            }
        })
    }

    /// Gets the boolean argument at the given index.
    ///
    /// # Arguments
    ///
    /// * `index` - The zero-based index of the argument
    ///
    /// # Returns
    ///
    /// `Some(bool)` if the argument exists and is a boolean, `None` otherwise.
    pub fn get_boolean_arg(&self, index: usize) -> Option<bool> {
        self.args.get(index).and_then(|arg| {
            if let Value::Boolean(b, _) = arg {
                Some(*b)
            } else {
                None
            }
        })
    }

    /// Returns `true` if this directive has a nested block.
    pub fn has_block(&self) -> bool {
        self.block.is_some()
    }
}

/// A conditional match block that evaluates request attributes.
///
/// Match blocks contain a list of expressions that are evaluated
/// to determine if certain configuration should apply.
///
/// # Example
///
/// ```ferron
/// match api_rules {
///     {{request.path}} ~ "^/api/"
///     {{request.method}} in "GET,POST"
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct MatchBlock {
    /// The name of this matcher (for reference).
    pub matcher: String,
    /// The list of matcher expressions.
    pub expr: Vec<MatcherExpression>,
    /// The source span of the match block.
    pub span: Span,
}

impl MatchBlock {
    /// Returns `true` if this match block contains any expressions.
    pub fn has_expressions(&self) -> bool {
        !self.expr.is_empty()
    }

    /// Returns a slice of all matcher expressions in this block.
    pub fn get_expressions(&self) -> &[MatcherExpression] {
        &self.expr
    }
}

/// A host-specific configuration block.
///
/// Host blocks apply configuration to specific hosts, protocols, or ports.
/// They are only allowed at the top level.
///
/// # Example
///
/// ```ferron
/// example.com:443 {
///     ssl enabled
/// }
///
/// *.example.com {
///     proxy http://backend
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct HostBlock {
    /// The list of host patterns this block applies to.
    pub hosts: Vec<HostPattern>,
    /// The nested block of statements.
    pub block: Block,
    /// The source span of the host block.
    pub span: Span,
}

impl HostBlock {
    /// Returns all host patterns as full strings (including protocol).
    ///
    /// # Returns
    ///
    /// A vector of strings like `"http example.com"` or `"*.example.com:443"`.
    pub fn get_host_patterns(&self) -> Vec<String> {
        self.hosts.iter().map(|hp| hp.as_full_str()).collect()
    }

    /// Checks if this host block matches a specific host pattern.
    ///
    /// # Arguments
    ///
    /// * `host` - The host string to check (e.g., `"example.com"`)
    ///
    /// # Returns
    ///
    /// `true` if any of the block's hosts match the given host.
    pub fn matches_host(&self, host: &str) -> bool {
        self.hosts.iter().any(|hp| hp.as_str() == host)
    }
}

/// A reusable configuration snippet definition.
///
/// Snippets allow defining configuration fragments that can be
/// included elsewhere (implementation-dependent).
///
/// # Example
///
/// ```ferron
/// snippet ssl_config {
///     ssl_certificate /etc/ssl/cert.pem
///     ssl_key /etc/ssl/key.pem
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct SnippetBlock {
    /// The name of this snippet.
    pub name: String,
    /// The nested block of statements.
    pub block: Block,
    /// The source span of the snippet block.
    pub span: Span,
}

/// A block of nested statements enclosed in braces.
///
/// Blocks can contain any type of statement and can be nested.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    /// The statements contained in this block.
    pub statements: Vec<Statement>,
    /// The source span of the block (position of the opening `{`).
    pub span: Span,
}

impl Block {
    /// Returns the source span of this block.
    pub fn span(&self) -> Span {
        self.span
    }

    /// Finds all [`Directive`] statements with the given name in this block.
    ///
    /// # Arguments
    ///
    /// * `name` - The directive name to search for
    ///
    /// # Returns
    ///
    /// A vector of references to matching directives.
    pub fn find_directives(&self, name: &str) -> Vec<&Directive> {
        self.statements
            .iter()
            .filter_map(|stmt| {
                if let Statement::Directive(directive) = stmt {
                    if directive.name == name {
                        Some(directive)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect()
    }

    /// Finds the first [`Directive`] with the given name in this block.
    ///
    /// # Arguments
    ///
    /// * `name` - The directive name to search for
    ///
    /// # Returns
    ///
    /// `Some(&Directive)` if found, `None` otherwise.
    pub fn find_directive(&self, name: &str) -> Option<&Directive> {
        self.statements.iter().find_map(|stmt| {
            if let Statement::Directive(directive) = stmt {
                if directive.name == name {
                    Some(directive)
                } else {
                    None
                }
            } else {
                None
            }
        })
    }
}

/// A value in a directive argument or expression operand.
///
/// Values can be strings, numbers, booleans, or interpolated strings
/// containing variable references.
///
/// # Example
///
/// ```ferron
/// root /var/www                    # String("/var/www")
/// max_connections 1000             # Integer(1000)
/// ratio 3.14                       # Float(3.14)
/// enabled true                     # Boolean(true)
/// path "{{app.root}}/www"          # InterpolatedString(...)
/// ```
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// A string value (quoted or bare).
    String(String, Span),
    /// An integer value.
    Integer(i64, Span),
    /// A floating-point value.
    Float(f64, Span),
    /// A boolean value (`true` or `false`).
    Boolean(bool, Span),
    /// A string with interpolation expressions.
    InterpolatedString(Vec<StringPart>, Span),
}

impl Value {
    /// Returns the source span of this value.
    pub fn span(&self) -> Span {
        match self {
            Value::String(_, span) => *span,
            Value::Integer(_, span) => *span,
            Value::Float(_, span) => *span,
            Value::Boolean(_, span) => *span,
            Value::InterpolatedString(_, span) => *span,
        }
    }

    /// Attempts to extract this value as a string slice.
    ///
    /// # Returns
    ///
    /// `Some(&str)` if this is a `Value::String`, `None` otherwise.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s, _) => Some(s),
            _ => None,
        }
    }

    /// Attempts to extract this value as an interpolated string slice.
    ///
    /// # Returns
    ///
    /// `Some(&[StringPart])` if this is a `Value::InterpolatedString`, `None` otherwise.
    pub fn as_interpolated_string(&self) -> Option<&[StringPart]> {
        match self {
            Value::InterpolatedString(s, _) => Some(s),
            _ => None,
        }
    }

    /// Attempts to extract this value as an integer.
    ///
    /// # Returns
    ///
    /// `Some(i64)` if this is a `Value::Integer`, `None` otherwise.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Integer(i, _) => Some(*i),
            _ => None,
        }
    }

    /// Attempts to extract this value as a float.
    ///
    /// # Returns
    ///
    /// `Some(f64)` if this is a `Value::Float`, `None` otherwise.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Float(f, _) => Some(*f),
            _ => None,
        }
    }

    /// Attempts to extract this value as a boolean.
    ///
    /// # Returns
    ///
    /// `Some(bool)` if this is a `Value::Boolean`, `None` otherwise.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Boolean(b, _) => Some(*b),
            _ => None,
        }
    }
}

/// A part of an interpolated string.
///
/// Interpolated strings consist of alternating literal text and
/// variable expressions (e.g., `{{app.root}}`).
#[derive(Debug, Clone, PartialEq)]
pub enum StringPart {
    /// A literal string segment.
    Literal(String),
    /// A dotted path expression (e.g., `["app", "root"]` for `{{app.root}}`).
    Expression(Vec<String>),
}

impl StringPart {
    /// Converts this string part back to its source representation.
    ///
    /// # Returns
    ///
    /// For `Literal`, returns the literal text.
    /// For `Expression`, returns the path wrapped in `{{ }}`.
    pub fn as_str(&self) -> String {
        match self {
            StringPart::Literal(s) => s.clone(),
            StringPart::Expression(v) => {
                let mut expr = String::new();
                expr.push_str("{{");
                expr.push_str(&v.join("."));
                expr.push_str("}}");
                expr
            }
        }
    }
}

/// The type of host label in a host pattern.
///
/// Host patterns can be hostnames, IP addresses, or wildcards.
#[derive(Debug, Clone, PartialEq)]
pub enum HostLabels {
    /// A hostname composed of labels (e.g., `["example", "com"]`).
    Hostname(Vec<String>),
    /// An IP address (IPv4 or IPv6).
    IpAddr(IpAddr),
    /// A wildcard (`*`) matching any host.
    Wildcard,
}

impl HostLabels {
    /// Converts these labels to their string representation.
    pub fn as_str(&self) -> String {
        match self {
            HostLabels::Hostname(labels) => labels.join("."),
            HostLabels::IpAddr(ip) => ip.to_string(),
            HostLabels::Wildcard => "*".to_string(),
        }
    }
}

/// A host pattern in a host block.
///
/// Host patterns specify which hosts a configuration block applies to.
/// They can include a protocol, hostname/IP, and optional port.
///
/// # Examples
///
/// - `example.com` - hostname only
/// - `*.example.com:443` - wildcard with port
/// - `http://api.example.com` - with protocol
/// - `[2001:db8::1]:8080` - IPv6 with port
#[derive(Debug, Clone, PartialEq)]
pub struct HostPattern {
    /// The host labels (hostname, IP, or wildcard).
    pub labels: HostLabels,
    /// The optional port number.
    pub port: Option<u16>,
    /// The optional protocol (e.g., `"http"`, `"tcp"`).
    pub protocol: Option<String>,
    /// The source span of the host pattern.
    pub span: Span,
}

impl HostPattern {
    /// Returns the host pattern as a string (without protocol).
    ///
    /// # Example
    ///
    /// ```rust
    /// # use ferronconf::Config;
    /// # use std::str::FromStr;
    /// let config = Config::from_str("example.com:443 { }").unwrap();
    /// if let Some(hb) = config.find_host_blocks().first() {
    ///     assert_eq!(hb.hosts[0].as_str(), "example.com:443");
    /// }
    /// ```
    pub fn as_str(&self) -> String {
        let mut pattern = self.labels.as_str();
        if let Some(port) = self.port {
            pattern.push(':');
            pattern.push_str(&port.to_string());
        }
        pattern
    }

    /// Returns the full host pattern including the protocol.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use ferronconf::Config;
    /// # use std::str::FromStr;
    /// let config = Config::from_str("http example.com { }").unwrap();
    /// if let Some(hb) = config.find_host_blocks().first() {
    ///     assert_eq!(hb.hosts[0].as_full_str(), "http example.com");
    /// }
    /// ```
    pub fn as_full_str(&self) -> String {
        let mut pattern = String::new();
        if let Some(protocol) = &self.protocol {
            pattern.push_str(protocol);
            pattern.push(' ');
        }
        pattern.push_str(&self.as_str());
        pattern
    }
}

/// An expression in a match block that compares two operands.
///
/// # Example
///
/// ```ferron
/// {{request.path}} ~ "^/api/"
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct MatcherExpression {
    /// The left-hand side operand.
    pub left: Operand,
    /// The comparison operator.
    pub op: Operator,
    /// The right-hand side operand.
    pub right: Operand,
    /// The source span of the expression.
    pub span: Span,
}

impl MatcherExpression {
    /// Returns the source span of this expression.
    pub fn span(&self) -> Span {
        self.span
    }

    /// Returns `true` if this is an equality (`==`) expression.
    pub fn is_equality(&self) -> bool {
        matches!(self.op, Operator::Eq)
    }

    /// Returns `true` if this is an inequality (`!=`) expression.
    pub fn is_inequality(&self) -> bool {
        matches!(self.op, Operator::NotEq)
    }

    /// Returns `true` if this is a regex (`~` or `!~`) expression.
    pub fn is_regex(&self) -> bool {
        matches!(self.op, Operator::Regex | Operator::NotRegex)
    }
}

/// An operand in a matcher expression.
///
/// Operands can be identifier paths, strings, or numbers.
#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    /// A dotted identifier path (e.g., `["request", "path"]` for `{{request.path}}`).
    Identifier(Vec<String>, Span),
    /// A string value.
    String(String, Span),
    /// An integer value.
    Integer(i64, Span),
    /// A floating-point value.
    Float(f64, Span),
}

impl Operand {
    /// Returns the source span of this operand.
    pub fn span(&self) -> Span {
        match self {
            Operand::Identifier(_, span) => *span,
            Operand::String(_, span) => *span,
            Operand::Integer(_, span) => *span,
            Operand::Float(_, span) => *span,
        }
    }

    /// Attempts to extract this operand as a string slice.
    ///
    /// # Returns
    ///
    /// `Some(&str)` if this is an `Operand::String`, `None` otherwise.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Operand::String(s, _) => Some(s),
            _ => None,
        }
    }

    /// Attempts to extract this operand as an identifier path.
    ///
    /// # Returns
    ///
    /// `Some(&[String])` if this is an `Operand::Identifier`, `None` otherwise.
    pub fn as_identifier(&self) -> Option<&[String]> {
        match self {
            Operand::Identifier(parts, _) => Some(parts),
            _ => None,
        }
    }

    /// Attempts to extract this operand as an integer.
    ///
    /// # Returns
    ///
    /// `Some(i64)` if this is an `Operand::Integer`, `None` otherwise.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Operand::Integer(i, _) => Some(*i),
            _ => None,
        }
    }

    /// Attempts to extract this operand as a float.
    ///
    /// # Returns
    ///
    /// `Some(f64)` if this is an `Operand::Float`, `None` otherwise.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Operand::Float(f, _) => Some(*f),
            _ => None,
        }
    }
}

/// A comparison operator in a matcher expression.
///
/// # Operators
///
/// | Variant | Syntax | Description |
/// |---------|--------|-------------|
/// | `Eq` | `==` | Equality comparison |
/// | `NotEq` | `!=` | Inequality comparison |
/// | `Regex` | `~` | Regular expression match |
/// | `NotRegex` | `!~` | Negated regex match |
/// | `In` | `in` | Membership test |
#[derive(Debug, Clone, PartialEq)]
pub enum Operator {
    /// Equality (`==`).
    Eq,
    /// Inequality (`!=`).
    NotEq,
    /// Regex match (`~`).
    Regex,
    /// Negated regex match (`!~`).
    NotRegex,
    /// Membership test (`in`).
    In,
}

impl Operator {
    /// Returns the string representation of this operator.
    pub fn as_str(&self) -> &'static str {
        match self {
            Operator::Eq => "==",
            Operator::NotEq => "!=",
            Operator::Regex => "~",
            Operator::NotRegex => "!~",
            Operator::In => "in",
        }
    }

    /// Returns `true` if this is a comparison operator (`==` or `!=`).
    pub fn is_comparison(&self) -> bool {
        matches!(self, Operator::Eq | Operator::NotEq)
    }

    /// Returns `true` if this is a regex operator (`~` or `!~`).
    pub fn is_regex(&self) -> bool {
        matches!(self, Operator::Regex | Operator::NotRegex)
    }
}
