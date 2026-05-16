//! Configuration validation for the reverse proxy module.

use std::collections::HashMap;
use std::collections::HashSet;
use std::error::Error;
use std::str::FromStr;

use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::config::{ServerConfigurationBlock, ServerConfigurationDirectiveEntry};
use ferron_core::util::parse_duration;
use http::header::HeaderName;

/// Configuration validator for the reverse proxy module.
pub struct ProxyConfigurationValidator;

impl ConfigurationValidator for ProxyConfigurationValidator {
    fn validate_block(
        &self,
        config: &ServerConfigurationBlock,
        used_directives: &mut HashSet<String>,
        is_global: bool,
    ) -> Result<(), Box<dyn Error>> {
        if is_global {
            // Validate global concurrent_conns directive
            if let Some(entries) = config.directives.get("concurrent_conns") {
                used_directives.insert("concurrent_conns".to_string());
                for e in entries {
                    if let Some(val) = e.args.first().and_then(|v| v.as_number()) {
                        if val < 0 {
                            return Err("Invalid `concurrent_conns` — must be non-negative".into());
                        }
                    } else {
                        return Err("Invalid `concurrent_conns` — expected a number".into());
                    }
                }
            }
        }
        if let Some(entries) = config.directives.get("proxy") {
            used_directives.insert("proxy".to_string());
            validate_proxy_entries(entries, used_directives)?;
        }
        Ok(())
    }
}

fn validate_proxy_entries(
    entries: &[ServerConfigurationDirectiveEntry],
    used_directives: &mut HashSet<String>,
) -> Result<(), Box<dyn Error>> {
    for entry in entries {
        if entry.args.len() > 1 {
            return Err(
                "The `proxy` directive may have at most one shorthand upstream argument".into(),
            );
        }
        for arg in &entry.args {
            if arg.as_string_with_interpolations(&HashMap::new()).is_none() {
                return Err("Invalid proxy upstream URL — expected a string".into());
            }
        }
        if let Some(block) = &entry.children {
            validate_proxy_block(block, used_directives)?;
        }
    }
    Ok(())
}

fn validate_proxy_block(
    block: &ServerConfigurationBlock,
    used_directives: &mut HashSet<String>,
) -> Result<(), Box<dyn Error>> {
    validate_str(block, used_directives, "lb_algorithm")?;
    validate_bool(block, used_directives, "lb_health_check")?;
    validate_number(block, used_directives, "lb_health_check_max_fails", 0)?;
    validate_duration(block, used_directives, "lb_health_check_window")?;
    validate_bool(block, used_directives, "lb_retry_connection")?;
    validate_bool(block, used_directives, "keepalive")?;
    validate_bool(block, used_directives, "http2")?;
    validate_bool(block, used_directives, "http2_only")?;
    validate_bool(block, used_directives, "intercept_errors")?;
    validate_bool(block, used_directives, "no_verification")?;
    validate_enum(block, used_directives, "proxy_header", &["v1", "v2"])?;
    validate_request_header(block, used_directives)?;
    validate_number(block, used_directives, "proxy_concurrent_conns", 0)?;
    validate_upstream_directives(block, used_directives)?;
    #[cfg(feature = "srv-lookup")]
    validate_srv_directives(block, used_directives)?;
    Ok(())
}

fn validate_str(
    block: &ServerConfigurationBlock,
    used: &mut HashSet<String>,
    name: &str,
) -> Result<(), Box<dyn Error>> {
    if let Some(entries) = block.directives.get(name) {
        used.insert(name.to_string());
        for e in entries {
            if e.args.first().and_then(|v| v.as_str()).is_none() {
                return Err(format!("Invalid `{name}` — expected a string").into());
            }
        }
    }
    Ok(())
}

fn validate_interpolated_str(
    block: &ServerConfigurationBlock,
    used: &mut HashSet<String>,
    name: &str,
) -> Result<(), Box<dyn Error>> {
    if let Some(entries) = block.directives.get(name) {
        used.insert(name.to_string());
        for e in entries {
            if e.args
                .first()
                .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
                .is_none()
            {
                return Err(
                    format!("Invalid `{name}` — expected a string (can be interpolated)").into(),
                );
            }
        }
    }
    Ok(())
}

fn validate_bool(
    block: &ServerConfigurationBlock,
    used: &mut HashSet<String>,
    name: &str,
) -> Result<(), Box<dyn Error>> {
    if let Some(entries) = block.directives.get(name) {
        used.insert(name.to_string());
        for e in entries {
            if e.args.is_empty() {
                continue;
            }
            if e.args.first().and_then(|v| v.as_boolean()).is_none() {
                return Err(format!("Invalid `{name}` — expected a boolean").into());
            }
        }
    }
    Ok(())
}

fn validate_number(
    block: &ServerConfigurationBlock,
    used: &mut HashSet<String>,
    name: &str,
    min: i64,
) -> Result<(), Box<dyn Error>> {
    if let Some(entries) = block.directives.get(name) {
        used.insert(name.to_string());
        for e in entries {
            if let Some(val) = e.args.first().and_then(|v| v.as_number()) {
                if val < min {
                    return Err(format!("Invalid `{name}` — must be >= {min}").into());
                }
            } else {
                return Err(format!("Invalid `{name}` — expected a number").into());
            }
        }
    }
    Ok(())
}

fn validate_duration(
    block: &ServerConfigurationBlock,
    used: &mut HashSet<String>,
    name: &str,
) -> Result<(), Box<dyn Error>> {
    if let Some(entries) = block.directives.get(name) {
        used.insert(name.to_string());
        for e in entries {
            if let Some(val) = e.args.first().and_then(|v| v.as_str()) {
                parse_duration(val).map_err(|e| format!("Invalid `{name}` duration: {e}"))?;
            } else {
                return Err(format!("Invalid `{name}` — expected a duration string").into());
            }
        }
    }
    Ok(())
}

fn validate_enum(
    block: &ServerConfigurationBlock,
    used: &mut HashSet<String>,
    name: &str,
    variants: &[&str],
) -> Result<(), Box<dyn Error>> {
    if let Some(entries) = block.directives.get(name) {
        used.insert(name.to_string());
        for e in entries {
            if let Some(val) = e.args.first().and_then(|v| v.as_str()) {
                if !variants.contains(&val) {
                    return Err(format!(
                        "Invalid `{name}` — expected one of: {}",
                        variants.join(", ")
                    )
                    .into());
                }
            } else {
                return Err(format!("Invalid `{name}` — expected a string").into());
            }
        }
    }
    Ok(())
}

fn validate_request_header(
    block: &ServerConfigurationBlock,
    used: &mut HashSet<String>,
) -> Result<(), Box<dyn Error>> {
    if let Some(entries) = block.directives.get("request_header") {
        used.insert("request_header".to_string());
        for e in entries {
            if e.args.is_empty() {
                return Err("request_header requires at least one argument".into());
            }
            let first = e.args[0]
                .as_str()
                .ok_or("The header name must be a string")?;
            let (name, needs_value) = match first.chars().next() {
                Some('+') => (&first[1..], true),
                Some('-') => (&first[1..], false),
                _ => (first, true),
            };
            HeaderName::from_str(name).map_err(|e| format!("Invalid header name '{name}': {e}"))?;
            if needs_value
                && e.args
                    .get(1)
                    .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
                    .is_none()
            {
                return Err("request_header requires a value for add/replace operations".into());
            }
        }
    }
    Ok(())
}

fn validate_upstream_directives(
    block: &ServerConfigurationBlock,
    used: &mut HashSet<String>,
) -> Result<(), Box<dyn Error>> {
    if let Some(entries) = block.directives.get("upstream") {
        used.insert("upstream".to_string());
        for e in entries {
            if e.args
                .first()
                .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
                .is_none()
            {
                return Err("The `upstream` directive requires a URL argument".into());
            }
            if let Some(up_block) = &e.children {
                validate_upstream_block(up_block, used)?;
            }
        }
    }
    Ok(())
}

fn validate_upstream_block(
    block: &ServerConfigurationBlock,
    used: &mut HashSet<String>,
) -> Result<(), Box<dyn Error>> {
    validate_number(block, used, "limit", 1)?;
    validate_duration(block, used, "idle_timeout")?;
    validate_interpolated_str(block, used, "unix")?;
    #[cfg(not(unix))]
    if block.directives.contains_key("unix") {
        return Err("Unix sockets are not supported on this platform".into());
    }
    Ok(())
}

/// Validate SRV upstream directives.
#[cfg(feature = "srv-lookup")]
fn validate_srv_directives(
    block: &ServerConfigurationBlock,
    used: &mut HashSet<String>,
) -> Result<(), Box<dyn Error>> {
    if let Some(entries) = block.directives.get("srv") {
        used.insert("srv".to_string());
        for e in entries {
            if e.args.first().and_then(|v| v.as_str()).is_none() {
                return Err("The `srv` directive requires an SRV record name argument".into());
            }
            if let Some(srv_block) = &e.children {
                validate_srv_block(srv_block, used)?;
            }
        }
    }
    Ok(())
}

#[cfg(feature = "srv-lookup")]
fn validate_srv_block(
    block: &ServerConfigurationBlock,
    used: &mut HashSet<String>,
) -> Result<(), Box<dyn Error>> {
    validate_number(block, used, "limit", 1)?;
    validate_duration(block, used, "idle_timeout")?;
    validate_str(block, used, "dns_servers")?;
    Ok(())
}
