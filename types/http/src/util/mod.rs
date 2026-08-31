//! HTTP utility functions and types.
//!
//! This module provides common helpers for XSS protection, HTML page
//! formatting, and HTTP quality-value header parsing.

/// Common CSS styles embedded in default error and directory listing pages.
pub const CSS_COMMON: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/common.css"));

/// HTML entity escaping for XSS protection.
pub mod anti_xss;
mod default_html_page;
/// HTTP `Accept`-style quality-value header parsing.
pub mod parse_q_value_header;
/// HTTP `Accept`-style quality-value header parsing with q-value grouping.
pub mod parse_q_value_header_grouped;
