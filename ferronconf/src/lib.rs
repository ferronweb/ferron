//! A Rust library for parsing `ferron.conf` configuration files.
//!
//! `ferron.conf` is a domain-specific language for custom web server configurations.
//! This crate provides a reference implementation including a lexer, parser, and AST.
//!
//! # Features
//!
//! - **Lexer** — Tokenizes configuration files with support for comments, strings,
//!   numbers, booleans, and interpolation
//! - **Parser** — Builds an AST from tokens with full error reporting
//! - **AST** — Type-safe representation of configuration structures with helper methods
//!
//! # Quick Start
//!
//! ```rust
//! use ferronconf::Config;
//! use std::str::FromStr;
//!
//! let input = r#"
//! example.com {
//!     root /var/www/example
//!     tls true
//! }
//!
//! *.example.com:443 {
//!     proxy http://backend
//! }
//! "#;
//!
//! let config = Config::from_str(input).unwrap();
//!
//! // Find all host blocks
//! for host_block in config.find_host_blocks() {
//!     for host in &host_block.hosts {
//!         println!("Host: {}", host.as_str());
//!     }
//! }
//! ```
//!
//! # See Also
//!
//! - [`Config`] — The root AST node
//! - [`Span`] — Source location for error reporting
//! - [`ParseError`] — Error type for parse failures

mod ast;
mod lexer;
mod parser;
mod tests;

pub use ast::*;
pub use lexer::*;
pub use parser::*;
