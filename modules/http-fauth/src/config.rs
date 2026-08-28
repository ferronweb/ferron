//! Configuration parsing for the forwarded authentication module.

use std::str::FromStr;
use std::time::Duration;

use ferron_core::config::ServerConfigurationValue;
use http::header::HeaderName;

/// A header action for the `request_header` subdirective: currently only
/// append is supported for `request_header +Name`.
#[derive(Clone, Debug)]
pub enum HeaderAction {
    /// Append the given value to the header.
    Append(HeaderName, String),
}

/// Parsed forwarded authentication configuration.
#[derive(Clone, Debug)]
pub struct ForwardedAuthConfig {
    /// Authentication backend URL
    pub backend_url: String,
    /// Unix socket path (optional)
    pub unix_socket: Option<String>,
    /// Connection limit for this backend
    pub connection_limit: Option<usize>,
    /// Idle timeout for connections
    pub idle_timeout: Duration,
    /// Disable TLS certificate verification
    pub no_verification: bool,
    /// Headers to copy from auth response to original request
    pub copy_headers: Vec<HeaderName>,
    /// Whether to intercept upstream error responses
    pub intercept_errors: bool,
    /// Headers to add to the auth request (`request_header +Name`)
    pub headers_to_add: Vec<HeaderAction>,
    /// Headers to replace on the auth request (`request_header Name`)
    pub headers_to_replace: Vec<(HeaderName, String)>,
    /// Headers to remove from the auth request (`request_header -Name`)
    pub headers_to_remove: Vec<HeaderName>,
}

impl Default for ForwardedAuthConfig {
    fn default() -> Self {
        Self {
            backend_url: String::new(),
            unix_socket: None,
            connection_limit: None,
            idle_timeout: Duration::from_millis(60_000), // 60 seconds
            no_verification: false,
            copy_headers: Vec::new(),
            intercept_errors: false,
            headers_to_add: Vec::new(),
            headers_to_replace: Vec::new(),
            headers_to_remove: Vec::new(),
        }
    }
}

/// Parse a single `request_header` entry into the given config, supporting
/// `+Name` (append), `-Name` (remove), and bare `Name` (replace) forms.
fn parse_request_header_entry(
    entry: &ferron_core::config::ServerConfigurationDirectiveEntry,
    cfg: &mut ForwardedAuthConfig,
    ctx: &ferron_http::HttpContext,
) -> Result<(), Box<dyn std::error::Error>> {
    if entry.args.is_empty() {
        return Err("request_header requires at least one argument".into());
    }

    let first_arg = entry.args[0]
        .as_str()
        .ok_or("request_header name must be a string")?;

    match first_arg.chars().next() {
        Some('+') => {
            let name = &first_arg[1..];
            let value = entry
                .args
                .get(1)
                .and_then(|v| v.as_string_with_interpolations(ctx))
                .ok_or("request_header +Name requires a value")?;
            let header_name = HeaderName::from_str(name)
                .map_err(|e| format!("Invalid header name '{name}': {e}"))?;
            cfg.headers_to_add
                .push(HeaderAction::Append(header_name, value));
        }
        Some('-') => {
            let name = &first_arg[1..];
            let header_name = HeaderName::from_str(name)
                .map_err(|e| format!("Invalid header name '{name}': {e}"))?;
            cfg.headers_to_remove.push(header_name);
        }
        _ => {
            let name = first_arg;
            let value = entry
                .args
                .get(1)
                .and_then(|v| v.as_string_with_interpolations(ctx))
                .ok_or("request_header Name requires a value")?;
            let header_name = HeaderName::from_str(name)
                .map_err(|e| format!("Invalid header name '{name}': {e}"))?;
            cfg.headers_to_replace.push((header_name, value));
        }
    }

    Ok(())
}

/// Parse forwarded authentication configuration from HTTP context.
pub fn parse_forwarded_auth_from_context(
    ctx: &ferron_http::HttpContext,
) -> Result<Vec<ForwardedAuthConfig>, Box<dyn std::error::Error>> {
    let block = &ctx.configuration;

    let auth_to_entries = block.get_entries("auth_to", false);
    let mut parsed_auth_to_entries = Vec::with_capacity(auth_to_entries.len());

    for auth_to_entry in auth_to_entries {
        let mut config = ForwardedAuthConfig::default();

        // Parse backend URL (required)
        let mut backend_url = match &auth_to_entry.args.first() {
            Some(ServerConfigurationValue::String(url, _)) => Some(url.clone()),
            Some(ServerConfigurationValue::InterpolatedString(_, _)) => {
                auth_to_entry.args[0].as_string_with_interpolations(ctx)
            }
            Some(ServerConfigurationValue::Boolean(false, _)) => return Ok(vec![]), // Disabled
            Some(ServerConfigurationValue::Boolean(true, _)) => None,
            None => None,
            _ => return Err("auth_to backend URL must be a string".into()),
        };

        let mut last = false;

        if let Some(children) = &auth_to_entry.children {
            // Parse backend URL (if not set)
            if backend_url.is_none() {
                if let Some(url_entries) = children.directives.get("url") {
                    if let Some(entry) = url_entries.first() {
                        if let Some(url) = entry
                            .args
                            .first()
                            .and_then(|a| a.as_string_with_interpolations(ctx))
                        {
                            backend_url = Some(url);
                        }
                    }
                }
            }

            if let Some(unix_entries) = children.directives.get("unix") {
                if let Some(entry) = unix_entries.first() {
                    if let Some(path) = entry
                        .args
                        .first()
                        .and_then(|a| a.as_string_with_interpolations(ctx))
                    {
                        config.unix_socket = Some(path);
                    }
                }
            }

            if let Some(limit_entries) = children.directives.get("limit") {
                if let Some(entry) = limit_entries.first() {
                    if entry.args.len() == 1 {
                        if let ServerConfigurationValue::Number(limit, _) = &entry.args[0] {
                            config.connection_limit = Some(*limit as usize);
                        } else if let ServerConfigurationValue::Boolean(false, _) = &entry.args[0] {
                            config.connection_limit = None; // Explicitly disabled
                        }
                    }
                }
            }

            if let Some(idle_timeout_entries) = children.directives.get("idle_timeout") {
                if let Some(entry) = idle_timeout_entries.first() {
                    if entry.args.len() == 1 {
                        if let ServerConfigurationValue::Number(timeout_ms, _) = &entry.args[0] {
                            config.idle_timeout = Duration::from_millis(*timeout_ms as u64);
                        } else if let ServerConfigurationValue::String(timeout_str, _) =
                            &entry.args[0]
                        {
                            if let Ok(timeout_ms) = timeout_str.parse::<u64>() {
                                config.idle_timeout = Duration::from_millis(timeout_ms);
                            }
                        } else if let ServerConfigurationValue::Boolean(false, _) = &entry.args[0] {
                            config.idle_timeout = Duration::from_millis(60_000);
                            // Reset to default
                        }
                    }
                }
            }

            if children.get_flag("no_verification") {
                config.no_verification = true;
            }

            if let Some(copy_entries) = children.directives.get("copy") {
                for entry in copy_entries {
                    for arg in &entry.args {
                        if let ServerConfigurationValue::String(header_name, _) = arg {
                            if let Ok(header) = HeaderName::from_str(header_name) {
                                config.copy_headers.push(header);
                            }
                        }
                    }
                }
            }

            if let Some(request_header_entries) = children.directives.get("request_header") {
                for entry in request_header_entries {
                    parse_request_header_entry(entry, &mut config, ctx)?;
                }
            }

            if children.get_flag("last") {
                last = true;
            }

            if children.get_flag("intercept_errors") {
                config.intercept_errors = true;
            }
        }

        if let Some(backend_url) = backend_url {
            config.backend_url = backend_url;
        } else {
            return Err("auth_to directive requires a backend URL".into());
        }

        parsed_auth_to_entries.push(config);

        if last {
            break;
        }
    }

    Ok(parsed_auth_to_entries)
}
