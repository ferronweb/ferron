//! Configuration parsing for the forwarded authentication module.

use std::str::FromStr;
use std::time::Duration;

use ferron_core::config::{layer::LayeredConfiguration, ServerConfigurationValue};
use http::header::HeaderName;

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
        }
    }
}

/// Parse forwarded authentication configuration from HTTP context.
pub fn parse_forwarded_auth_from_layered_config(
    block: &LayeredConfiguration,
) -> Result<Option<ForwardedAuthConfig>, Box<dyn std::error::Error>> {
    let mut config = ForwardedAuthConfig::default();

    // Get auth_to directive
    let auth_to_entries = block.get_entries("auth_to", false);
    let auth_to_entry = if let Some(entry) = auth_to_entries.first() {
        entry
    } else {
        return Ok(None); // No auth_to configuration
    };

    // Parse backend URL (required)
    if auth_to_entry.args.len() != 1 {
        return Err("auth_to directive requires exactly one argument (backend URL)".into());
    }

    let backend_url = match &auth_to_entry.args[0] {
        ServerConfigurationValue::String(url, _) => Some(url.clone()),
        ServerConfigurationValue::Boolean(false, _) => return Ok(None), // Disabled
        ServerConfigurationValue::Boolean(true, _) => None,
        _ => return Err("auth_to backend URL must be a string".into()),
    };

    // Parse nested directives
    if let Some(children) = &auth_to_entry.children {
        // Parse backend URL (if not set)
        if backend_url.is_none() {
            if let Some(url_entries) = children.directives.get("url") {
                if let Some(entry) = url_entries.first() {
                    if entry.args.len() == 1 {
                        if let ServerConfigurationValue::String(url, _) = &entry.args[0] {
                            config.backend_url = url.clone();
                        }
                    }
                }
            }
        }

        // Parse unix socket
        if let Some(unix_entries) = children.directives.get("unix") {
            if let Some(entry) = unix_entries.first() {
                if entry.args.len() == 1 {
                    if let ServerConfigurationValue::String(path, _) = &entry.args[0] {
                        config.unix_socket = Some(path.clone());
                    }
                }
            }
        }

        // Parse connection limit
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

        // Parse idle timeout
        if let Some(idle_timeout_entries) = children.directives.get("idle_timeout") {
            if let Some(entry) = idle_timeout_entries.first() {
                if entry.args.len() == 1 {
                    if let ServerConfigurationValue::Number(timeout_ms, _) = &entry.args[0] {
                        config.idle_timeout = Duration::from_millis(*timeout_ms as u64);
                    } else if let ServerConfigurationValue::String(timeout_str, _) = &entry.args[0]
                    {
                        // Try to parse as duration string
                        if let Ok(timeout_ms) = timeout_str.parse::<u64>() {
                            config.idle_timeout = Duration::from_millis(timeout_ms);
                        }
                    } else if let ServerConfigurationValue::Boolean(false, _) = &entry.args[0] {
                        config.idle_timeout = Duration::from_millis(60_000); // Reset to default
                    }
                }
            }
        }

        // Parse no_verification
        if let Some(no_verif_entries) = children.directives.get("no_verification") {
            if let Some(entry) = no_verif_entries.first() {
                if entry.args.len() == 1 {
                    if let ServerConfigurationValue::Boolean(no_verif, _) = &entry.args[0] {
                        config.no_verification = *no_verif;
                    }
                }
            }
        }

        // Parse copy headers
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
    }

    if let Some(backend_url) = backend_url {
        config.backend_url = backend_url;
    } else {
        return Err("auth_to directive requires a backend URL".into());
    }

    Ok(Some(config))
}

/// Parse forwarded authentication configuration from HTTP context.
pub fn parse_forwarded_auth_from_context(
    ctx: &ferron_http::HttpContext,
) -> Result<Option<ForwardedAuthConfig>, Box<dyn std::error::Error>> {
    parse_forwarded_auth_from_layered_config(&ctx.configuration)
}
