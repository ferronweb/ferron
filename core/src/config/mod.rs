//! Configuration system for server setup with hierarchical structures and value interpolation.
//!
//! This module provides the data model for Ferron's configuration. It is
//! protocol-agnostic: the types here represent the parsed configuration
//! without interpreting any specific directives.
//!
//! # Configuration hierarchy
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
//!
//! # Key types
//!
//! | Type | Purpose |
//! |---|---|
//! | [`ServerConfiguration`] | Top-level configuration: global block + per-port entries |
//! | [`ServerConfigurationBlock`] | A block of directives with optional nested children |
//! | [`ServerConfigurationDirectiveEntry`] | One occurrence of a directive (args + children) |
//! | [`ServerConfigurationValue`] | A typed value: string, number, float, bool, or interpolated |
//! | [`LayeredConfiguration`](layer::LayeredConfiguration) | Multiple blocks merged with override semantics |
//!
//! # Module authors
//!
//! Modules typically read configuration through [`LayeredConfiguration`](layer::LayeredConfiguration)
//! in their [`Stage::run`](crate::pipeline::Stage::run) implementation. Configuration
//! adapters and validators are registered via [`ModuleLoader`](crate::loader::ModuleLoader).

pub mod adapter;
mod duration;
pub mod layer;
mod macros;
pub mod validator;

pub use duration::*;
//pub use macros::*;

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

/// Source location of a configuration element for error reporting.
///
/// Tracks line, column, and file path so that validation errors and
/// diagnostics can point back to the exact position in the configuration
/// file.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ServerConfigurationSpan {
    /// Line number (1-indexed)
    pub line: usize,
    /// Column number (1-indexed)
    pub column: usize,
    /// Source file path
    pub file: Option<String>,
}

/// Top-level server configuration containing global and per-port settings.
///
/// The configuration is organized hierarchically:
///
/// - [`global_config`](Self::global_config) -- applies to all protocols.
/// - [`ports`](Self::ports) -- maps protocol names to lists of
///   [`ServerConfigurationPort`], each binding a port to one or more host
///   blocks with optional SNI/IP filters.
///
/// Modules receive an `Arc<ServerConfiguration>` in
/// [`ModuleLoader::register_modules`](crate::loader::ModuleLoader::register_modules)
/// and typically read from `global_config` or per-host blocks.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ServerConfiguration {
    /// Global configuration block applying to all protocols
    pub global_config: Arc<ServerConfigurationBlock>,
    /// Port configurations indexed by protocol name (e.g., "http", "https", "tcp")
    pub ports: BTreeMap<String, Vec<ServerConfigurationPort>>,
}

/// Configuration for a specific port and its associated host blocks.
///
/// Each port entry binds a port number (or inherits the protocol default)
/// and maps it to a list of
/// ([`ServerConfigurationHostFilters`], [`ServerConfigurationBlock`])
/// pairs. The filters determine which incoming connections match the host
/// block (by SNI hostname or local IP address).
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ServerConfigurationPort {
    /// Port number (optional, may be inherited from protocol defaults)
    pub port: Option<u16>,
    /// Host configurations with filters for SNI hostname and IP address matching
    pub hosts: Vec<(ServerConfigurationHostFilters, ServerConfigurationBlock)>,
}

/// Filters for matching incoming connections to a specific host block.
///
/// When a connection arrives, the server evaluates these filters to
/// determine which [`ServerConfigurationBlock`] applies:
///
/// - `ip` matches the local IP address the connection was accepted on.
/// - `host` matches the SNI hostname (TLS) or the `Host` header (HTTP).
#[derive(Debug, Default, Eq, PartialEq, Clone, Serialize, Deserialize)]
pub struct ServerConfigurationHostFilters {
    /// IP address to match (for multi-homed servers)
    pub ip: Option<IpAddr>,
    /// Host/domain name to match (for SNI)
    pub host: Option<String>,
}

/// A block of configuration directives with optional nested children.
///
/// This is the primary unit of configuration in Ferron. Directives are
/// organized by name, with support for:
///
/// - Multiple values per directive (each value is a
///   [`ServerConfigurationValue`]).
/// - Nested child blocks (e.g. `runtime { io_uring true }`).
/// - Named matchers for conditional directives.
/// - Source location tracking for error reporting.
///
/// Modules typically read directives via [`get_value`](Self::get_value),
/// [`get_flag`](Self::get_flag), or [`has_directive`](Self::has_directive).
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ServerConfigurationBlock {
    /// All directives in this block, indexed by name
    pub directives: Arc<FxHashMap<String, Vec<ServerConfigurationDirectiveEntry>>>,
    /// Named matcher expressions for conditional directives
    pub matchers: FxHashMap<String, ServerConfigurationMatcher>,
    /// Source location of this block
    pub span: Option<ServerConfigurationSpan>,
}

impl ServerConfigurationBlock {
    /// Get the first value for a directive.
    ///
    /// Returns the first argument of the first entry for the given directive
    /// name, or `None` if the directive does not exist.
    ///
    /// # Example
    ///
    /// ```ignore
    /// if let Some(port) = block.get_value("port") {
    ///     if let Some(n) = port.as_number() {
    ///         // use port number
    ///     }
    /// }
    /// ```
    #[inline]
    pub fn get_value(&self, directive: &str) -> Option<&ServerConfigurationValue> {
        self.directives
            .get(directive)
            .and_then(|entries| entries.first())
            .and_then(|entry| entry.args.first())
    }

    /// Get a directive as a boolean flag.
    ///
    /// Returns `true` if the directive is present and its first argument is
    /// a boolean with value `true`, or if the directive is present with no
    /// arguments (flag-style). Returns `false` if the directive is absent.
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

    /// Check if a directive exists anywhere in this block tree (recursively).
    ///
    /// Returns `true` if the directive is present at this level or in any
    /// nested child block. Useful for `is_applicable` checks in stages.
    pub fn has_directive(&self, directive: &str) -> bool {
        if self.directives.contains_key(directive) {
            return true;
        }
        self.directives.values().any(|entries| {
            entries.iter().any(|e| {
                e.children
                    .as_ref()
                    .is_some_and(|c| c.has_directive(directive))
            })
        })
    }

    /// Build a merged block containing the union of all directive keys from
    /// multiple configuration blocks.
    ///
    /// This is useful for [`is_applicable`](crate::pipeline::Stage::is_applicable)
    /// checks at server initialization: if any host block (or the global config)
    /// uses a directive, the corresponding stage should be included in the
    /// pipeline.
    ///
    /// The merged block has empty directive entries (only keys matter), since
    /// `has_directive` only checks key presence.
    pub fn merge_from<'a>(blocks: impl IntoIterator<Item = &'a Self>) -> Self {
        let mut all_keys = FxHashMap::default();
        for block in blocks {
            for key in block.directives.keys() {
                all_keys.entry(key.clone()).or_insert_with(Vec::new);
            }
        }
        ServerConfigurationBlock {
            directives: Arc::new(all_keys),
            matchers: FxHashMap::default(),
            span: None,
        }
    }
}

/// A single directive entry with arguments and optional nested configuration.
///
/// A configuration block may contain multiple entries for the same directive
/// name (e.g. multiple `listen` directives). Each entry holds its own
/// arguments and optional child block.
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
/// Supports four base types (string, integer, float, boolean) plus an
/// interpolated string variant that contains `{{variable}}` references.
/// Use the `as_*` methods to extract typed values:
///
/// - [`as_str`](Self::as_str) -- plain string reference
/// - [`as_string_with_interpolations`](Self::as_string_with_interpolations) -- string with variable substitution
/// - [`as_number`](Self::as_number) -- integer
/// - [`as_float`](Self::as_float) -- floating-point
/// - [`as_boolean`](Self::as_boolean) -- boolean
/// - [`as_duration`](Self::as_duration) -- duration (parses `"12h"`, `"30m"`, etc.)
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

    /// Get this value as a duration, if it is a duration.
    #[inline]
    pub fn as_duration(&self) -> Option<Duration> {
        match self {
            ServerConfigurationValue::Number(n, _) => Some(Duration::from_secs(*n as u64)),
            ServerConfigurationValue::Float(f, _) => Some(Duration::from_secs_f64(*f)),
            ServerConfigurationValue::String(s, _) => parse_duration(s).ok(),
            _ => None,
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
/// Matchers evaluate expressions like `$request_method == "GET"` to
/// conditionally apply directives. The server evaluates all expressions
/// in the matcher; if all pass, the associated directives are applied.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ServerConfigurationMatcher {
    /// List of expressions to evaluate (all must pass).
    pub exprs: Vec<ServerConfigurationMatcherExpr>,
    /// Source location.
    pub span: Option<ServerConfigurationSpan>,
}

/// A single matcher expression: `left op right`.
#[derive(Debug, Eq, PartialEq, Ord, PartialOrd, Clone, Serialize, Deserialize)]
pub struct ServerConfigurationMatcherExpr {
    /// Left operand (typically a variable like `$request_method`).
    pub left: ServerConfigurationMatcherOperand,
    /// Right operand (typically a literal value).
    pub right: ServerConfigurationMatcherOperand,
    /// Comparison operator.
    pub op: ServerConfigurationMatcherOperator,
}

/// An operand in a matcher expression: identifier, string, integer, or float.
///
/// Identifiers (e.g. `$request_method`) are resolved at runtime from the
/// request context. Literals are compared directly.
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
///
/// Used in [`ServerConfigurationMatcherExpr`] to compare left and right
/// operands.
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
    /// Membership test: `in`
    In,
}

/// Trait for resolving variables in configuration values.
///
/// Implement this trait to provide custom variable resolution for
/// interpolated strings. The default implementation for
/// [`HashMap<String, String>`](std::collections::HashMap) does direct
/// key lookup.
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
