//! Configuration system for server setup with hierarchical structures and value interpolation.
//!
//! This module provides:
//! - Hierarchical configuration blocks (global, per-port, per-host)
//! - Type-safe configuration values with interpolation support
//! - Configuration matching and filtering
//! - Builder patterns for constructing configurations
//! - Validation and adaptation frameworks
//! - Configuration watching for reload detection
//!
//! # Configuration Hierarchy
//!
//! ```text
//! Global Configuration
//!  |
//!  +-- Port/IP (TCP/HTTP/etc.)
//!      |
//!      +-- Host/SNI Filter
//!          |
//!          +-- Matchers and Directives
//!               |
//!               +-- Error Handling
//! ```

pub mod adapter;
mod builder;
pub mod layer;
pub mod macros;
pub mod validator;

pub use builder::*;

use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    net::IpAddr,
    sync::Arc,
};

/// Source location information for configuration elements.
///
/// Stores line, column, and file information for error reporting and debugging.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ServerConfigurationSpan {
    /// Line number (1-indexed)
    pub line: usize,
    /// Column number (1-indexed)
    pub column: usize,
    /// Source file path
    pub file: Option<String>,
}

/// Top-level server configuration containing global and per-protocol settings.
///
/// The configuration is organized hierarchically:
/// - Global configuration applies to all protocols
/// - Per-protocol ports and their associated hosts/SNI filters
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ServerConfiguration {
    /// Global configuration block applying to all protocols
    pub global_config: Arc<ServerConfigurationBlock>,
    /// Port configurations indexed by protocol name (e.g., "http", "https", "tcp")
    pub ports: BTreeMap<String, Vec<ServerConfigurationPort>>,
}

/// Configuration for a specific port and its associated hosts/SNI filters.
///
/// Allows multiple host configurations on the same port with different filters.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ServerConfigurationPort {
    /// Port number (optional, may be inherited from protocol defaults)
    pub port: Option<u16>,
    /// Host configurations with filters for SNI hostname and IP address matching
    pub hosts: Vec<(ServerConfigurationHostFilters, ServerConfigurationBlock)>,
}

/// Filters for matching port configurations to specific hosts or IPs.
///
/// Used for SNI (Server Name Indication) hostname matching and IP-based routing.
#[derive(Debug, Default, Eq, PartialEq, Clone, Serialize, Deserialize)]
pub struct ServerConfigurationHostFilters {
    /// IP address to match (for multi-homed servers)
    pub ip: Option<IpAddr>,
    /// Host/domain name to match (for SNI)
    pub host: Option<String>,
}

/// A block of configuration directives with optional nested structure.
///
/// Directives are organized by name, with support for:
/// - Multiple values per directive
/// - Nested child blocks
/// - Source location tracking for error reporting
/// - String interpolation support
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ServerConfigurationBlock {
    /// All directives in this block, indexed by name
    pub directives: Arc<HashMap<String, Vec<ServerConfigurationDirectiveEntry>>>,
    /// Named matcher expressions for conditional directives
    pub matchers: HashMap<String, ServerConfigurationMatcher>,
    /// Source location of this block
    pub span: Option<ServerConfigurationSpan>,
}

impl ServerConfigurationBlock {
    /// Get the first value for a directive.
    ///
    /// Returns the first argument of the first entry for the directive, or None if not found.
    #[inline]
    pub fn get_value(&self, directive: &str) -> Option<&ServerConfigurationValue> {
        self.directives
            .get(directive)
            .and_then(|entries| entries.first())
            .and_then(|entry| entry.args.first())
    }

    /// Get a directive as a boolean flag.
    ///
    /// Returns the boolean value if present, or true as default for flag-style directives.
    #[inline]
    pub fn get_flag(&self, directive: &str) -> bool {
        if let Some(e) = self
            .directives
            .get(directive)
            .and_then(|entries| entries.first())
        {
            e.get_flag()
        } else {
            false
        }
    }
}

/// A single directive entry with arguments and optional nested configuration.
///
/// Represents one occurrence of a directive within a configuration block.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ServerConfigurationDirectiveEntry {
    /// Arguments provided to this directive
    pub args: Vec<ServerConfigurationValue>,
    /// Optional nested configuration block
    pub children: Option<ServerConfigurationBlock>,
    /// Source location of this directive
    pub span: Option<ServerConfigurationSpan>,
}

impl ServerConfigurationDirectiveEntry {
    /// Get the first argument value.
    #[inline]
    pub fn get_value(&self) -> Option<&ServerConfigurationValue> {
        self.args.first()
    }

    /// Get this directive as a boolean flag.
    ///
    /// Returns the boolean value if present, or true for flag-style directives.
    #[inline]
    pub fn get_flag(&self) -> bool {
        if let Some(ServerConfigurationValue::Boolean(value, _)) = self.args.first() {
            *value
        } else {
            true
        }
    }
}

/// A typed configuration value with optional source location.
///
/// Supports strings (with optional interpolation), numbers, floats, and booleans.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerConfigurationValue {
    /// Plain string value
    String(String, Option<ServerConfigurationSpan>),
    /// Integer value
    Number(i64, Option<ServerConfigurationSpan>),
    /// Floating-point value
    Float(f64, Option<ServerConfigurationSpan>),
    /// Boolean value
    Boolean(bool, Option<ServerConfigurationSpan>),
    /// String with variable interpolation support
    InterpolatedString(
        Vec<ServerConfigurationInterpolatedStringPart>,
        Option<ServerConfigurationSpan>,
    ),
}

impl ServerConfigurationValue {
    /// Get this value as a string reference, if it is a string.
    #[inline]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            ServerConfigurationValue::String(s, _) => Some(s),
            _ => None,
        }
    }

    /// Get this value as a string with variable interpolation applied.
    ///
    /// Supports two types of variables:
    /// - `env.NAME` - Resolved from environment variables
    /// - `NAME` - Resolved from the provided variables map
    ///
    /// Unresolved variables are left as `{{NAME}}` in the output.
    pub fn as_string_with_interpolations(&self, variables: &impl Variables) -> Option<String> {
        match self {
            ServerConfigurationValue::String(s, _) => Some(s.clone()),
            ServerConfigurationValue::InterpolatedString(parts, _) => {
                let mut result = String::new();
                for part in parts {
                    match part {
                        ServerConfigurationInterpolatedStringPart::String(s) => result.push_str(s),
                        ServerConfigurationInterpolatedStringPart::Variable(var) => {
                            if let Some(env_var) = var.strip_prefix("env.") {
                                let env_var_name = &env_var;
                                if let Ok(env_value) = std::env::var(env_var_name) {
                                    result.push_str(&env_value);
                                } else {
                                    result.push_str(&format!("{{{{{}}}}}", var));
                                }
                            } else if let Some(value) = variables.resolve(var) {
                                result.push_str(&value);
                            } else {
                                result.push_str(&format!("{{{{{}}}}}", var));
                            }
                        }
                    }
                }
                Some(result)
            }
            _ => None,
        }
    }

    /// Get this value as an integer, if it is a number.
    #[inline]
    pub fn as_number(&self) -> Option<i64> {
        if let ServerConfigurationValue::Number(n, _) = self {
            Some(*n)
        } else {
            None
        }
    }

    /// Get this value as a float, if it is a float.
    #[inline]
    pub fn as_float(&self) -> Option<f64> {
        if let ServerConfigurationValue::Float(f, _) = self {
            Some(*f)
        } else {
            None
        }
    }

    /// Get this value as a boolean, if it is a boolean.
    #[inline]
    pub fn as_boolean(&self) -> Option<bool> {
        if let ServerConfigurationValue::Boolean(b, _) = self {
            Some(*b)
        } else {
            None
        }
    }
}

/// Part of an interpolated string: either literal text or a variable reference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerConfigurationInterpolatedStringPart {
    /// Literal string content
    String(String),
    /// Variable reference to be resolved
    Variable(String),
}

/// A matcher for conditional configuration directives.
///
/// Used to evaluate expressions like `$request_method == "GET"` for conditional routing.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ServerConfigurationMatcher {
    /// List of expressions to evaluate
    pub exprs: Vec<ServerConfigurationMatcherExpr>,
    /// Source location
    pub span: Option<ServerConfigurationSpan>,
}

/// A single matcher expression: `left op right`.
#[derive(Debug, Eq, PartialEq, Ord, PartialOrd, Clone, Serialize, Deserialize)]
pub struct ServerConfigurationMatcherExpr {
    /// Left operand
    pub left: ServerConfigurationMatcherOperand,
    /// Right operand
    pub right: ServerConfigurationMatcherOperand,
    /// Comparison operator
    pub op: ServerConfigurationMatcherOperator,
}

/// An operand in a matcher expression: identifier, string, integer, or float.
#[allow(clippy::derive_ord_xor_partial_ord)]
#[derive(Debug, PartialEq, PartialOrd, Clone, Serialize, Deserialize)]
pub enum ServerConfigurationMatcherOperand {
    /// Variable/identifier reference (e.g., `$request_method`)
    Identifier(String),
    /// String literal value
    String(String),
    /// Integer literal value
    Integer(i64),
    /// Float literal value
    Float(f64),
}

impl Eq for ServerConfigurationMatcherOperand {}

impl Ord for ServerConfigurationMatcherOperand {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use ServerConfigurationMatcherOperand::*;
        match (self, other) {
            (Identifier(a), Identifier(b)) => a.cmp(b),
            (String(a), String(b)) => a.cmp(b),
            (Integer(a), Integer(b)) => a.cmp(b),
            // For floats, we need to handle NaN values which do not have a total ordering
            (Float(a), Float(b)) => a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal),

            // Define an arbitrary but consistent ordering between different types
            (Identifier(_), _) => std::cmp::Ordering::Less,
            (String(_), Identifier(_)) => std::cmp::Ordering::Greater,
            (String(_), _) => std::cmp::Ordering::Less,
            (Integer(_), Identifier(_) | String(_)) => std::cmp::Ordering::Greater,
            (Integer(_), Float(_)) => std::cmp::Ordering::Less,
            (Float(_), Identifier(_) | String(_) | Integer(_)) => std::cmp::Ordering::Greater,
        }
    }
}

/// Comparison operators for matcher expressions.
#[derive(Debug, Eq, PartialEq, Ord, PartialOrd, Clone, Serialize, Deserialize)]
pub enum ServerConfigurationMatcherOperator {
    /// Equal: `==`
    Eq,
    /// Not equal: `!=`
    NotEq,
    /// Regular expression match: `~`
    Regex,
    /// Regular expression non-match: `!~`
    NotRegex,
    /// Membership: `in`
    In,
}

/// Trait for resolving variables in configuration values.
///
/// Implementations can resolve configuration variables by name.
pub trait Variables {
    /// Resolve a variable by name, returning its string value if found.
    fn resolve(&self, name: &str) -> Option<String>;
}

impl Variables for HashMap<String, String> {
    /// Resolve variables from a HashMap by direct lookup.
    #[inline]
    fn resolve(&self, name: &str) -> Option<String> {
        self.get(name).cloned()
    }
}
