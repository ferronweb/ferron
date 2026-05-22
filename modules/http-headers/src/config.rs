//! Configuration parsing for the HTTP headers module.

use std::error::Error;
use std::str::FromStr;

use ferron_core::config::{ServerConfigurationBlock, ServerConfigurationDirectiveEntry};
use ferron_http::HttpContext;
use http::header::HeaderName;

/// A response header action.
#[derive(Clone)]
pub enum HeaderAction {
    /// Append the given value to the header (allows duplicates).
    Append(HeaderName, String),
    /// Replace the header value (removes existing, sets new value).
    Replace(HeaderName, String),
    /// Remove all instances of the header.
    Remove(HeaderName),
}

/// CORS configuration.
#[derive(Clone, Default)]
pub struct CorsConfig {
    /// Allowed origins (empty means disabled, `["*"]` means any origin).
    pub origins: Vec<String>,
    /// Allowed HTTP methods.
    pub methods: Vec<String>,
    /// Allowed request headers.
    pub headers: Vec<String>,
    /// Whether credentials (cookies, auth) are allowed.
    pub credentials: bool,
    /// Preflight cache duration in seconds.
    pub max_age: Option<u32>,
    /// Headers exposed to the browser.
    pub expose_headers: Vec<String>,
}

/// Parsed HTTP headers configuration.
#[derive(Clone, Default)]
pub struct HeadersConfig {
    pub header_actions: Vec<HeaderAction>,
    pub cors: Option<CorsConfig>,
}

/// Parse header actions from a directive entry.
fn parse_header_entry(
    entry: &ServerConfigurationDirectiveEntry,
    cfg: &mut HeadersConfig,
    ctx: &HttpContext,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    if entry.args.is_empty() {
        return Err("header requires at least one argument".into());
    }

    let first_arg = entry.args[0]
        .as_str()
        .ok_or("header name must be a string")?;

    match first_arg.chars().next() {
        Some('+') => {
            let name = &first_arg[1..];
            let value = entry
                .args
                .get(1)
                .and_then(|v| v.as_string_with_interpolations(ctx))
                .ok_or("header +Name requires a value")?;
            let header_name = HeaderName::from_str(name)
                .map_err(|e| format!("Invalid header name '{name}': {e}"))?;
            cfg.header_actions
                .push(HeaderAction::Append(header_name, value));
        }
        Some('-') => {
            let name = &first_arg[1..];
            let header_name = HeaderName::from_str(name)
                .map_err(|e| format!("Invalid header name '{name}': {e}"))?;
            cfg.header_actions.push(HeaderAction::Remove(header_name));
        }
        _ => {
            let name = first_arg;
            let value = entry
                .args
                .get(1)
                .and_then(|v| v.as_string_with_interpolations(ctx))
                .ok_or("header Name requires a value")?;
            let header_name = HeaderName::from_str(name)
                .map_err(|e| format!("Invalid header name '{name}': {e}"))?;
            cfg.header_actions
                .push(HeaderAction::Replace(header_name, value));
        }
    }

    Ok(())
}

/// Parse a single CORS directive block.
fn parse_cors_block(
    block: &ServerConfigurationBlock,
    cors: &mut CorsConfig,
    ctx: &HttpContext,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    for (name, entries) in block.directives.iter() {
        match name.as_str() {
            "origins" => {
                for entry in entries {
                    for arg in &entry.args {
                        if let Some(val) = arg.as_string_with_interpolations(ctx) {
                            cors.origins.push(val);
                        }
                    }
                }
            }
            "methods" => {
                for entry in entries {
                    for arg in &entry.args {
                        if let Some(val) = arg.as_str() {
                            cors.methods.push(val.to_string());
                        }
                    }
                }
            }
            "headers" => {
                for entry in entries {
                    for arg in &entry.args {
                        if let Some(val) = arg.as_str() {
                            cors.headers.push(val.to_string());
                        }
                    }
                }
            }
            "credentials" => {
                if let Some(val) = entries.first().map(|e| e.get_flag()) {
                    cors.credentials = val;
                }
            }
            "max_age" => {
                if let Some(val) = entries
                    .first()
                    .and_then(|e| e.args.first())
                    .and_then(|v| v.as_duration())
                    .map(|d| d.as_secs())
                {
                    cors.max_age = Some(val as u32);
                }
            }
            "expose_headers" => {
                for entry in entries {
                    for arg in &entry.args {
                        if let Some(val) = arg.as_str() {
                            cors.expose_headers.push(val.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Parse headers configuration from an HttpContext.
pub fn parse_headers_config(
    ctx: &ferron_http::HttpContext,
) -> Result<Option<HeadersConfig>, Box<dyn Error + Send + Sync>> {
    let header_entries = ctx.configuration.get_entries("header", false);
    let cors_entries = ctx.configuration.get_entries("cors", false);

    if header_entries.is_empty() && cors_entries.is_empty() {
        return Ok(None);
    }

    let mut cfg = HeadersConfig::default();

    for entry in &header_entries {
        parse_header_entry(entry, &mut cfg, ctx)?;
    }

    for entry in &cors_entries {
        let mut cors = CorsConfig::default();
        if let Some(block) = &entry.children {
            parse_cors_block(block, &mut cors, ctx)?;
        }
        cfg.cors = Some(cors);
    }

    Ok(Some(cfg))
}
