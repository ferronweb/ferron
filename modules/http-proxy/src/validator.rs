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
            validate_proxy_entries(entries)?;
        }
        Ok(())
    }
}

fn validate_proxy_entries(
    entries: &[ServerConfigurationDirectiveEntry],
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
            validate_proxy_block(block)?;
        }
    }
    Ok(())
}

fn validate_proxy_block(block: &ServerConfigurationBlock) -> Result<(), Box<dyn Error>> {
    validate_str(block, "algorithm")?;
    validate_passive_check_directives(block)?;
    validate_circuit_breaker_directives(block)?;
    validate_bool(block, "retry_connection")?;
    validate_bool(block, "keepalive")?;
    validate_bool(block, "http2")?;
    validate_bool(block, "http2_only")?;
    validate_bool(block, "intercept_errors")?;
    validate_bool(block, "no_verification")?;
    validate_enum(block, "proxy_header", &["v1", "v2"])?;
    validate_request_header(block)?;
    validate_number(block, "proxy_concurrent_conns", 0)?;
    validate_upstream_directives(block)?;
    #[cfg(feature = "srv-lookup")]
    validate_srv_directives(block)?;
    Ok(())
}

fn validate_str(block: &ServerConfigurationBlock, name: &str) -> Result<(), Box<dyn Error>> {
    if let Some(entries) = block.directives.get(name) {
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
    name: &str,
) -> Result<(), Box<dyn Error>> {
    if let Some(entries) = block.directives.get(name) {
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

fn validate_bool(block: &ServerConfigurationBlock, name: &str) -> Result<(), Box<dyn Error>> {
    if let Some(entries) = block.directives.get(name) {
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
    name: &str,
    min: i64,
) -> Result<(), Box<dyn Error>> {
    if let Some(entries) = block.directives.get(name) {
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

fn validate_duration(block: &ServerConfigurationBlock, name: &str) -> Result<(), Box<dyn Error>> {
    if let Some(entries) = block.directives.get(name) {
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
    name: &str,
    variants: &[&str],
) -> Result<(), Box<dyn Error>> {
    if let Some(entries) = block.directives.get(name) {
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

fn validate_request_header(block: &ServerConfigurationBlock) -> Result<(), Box<dyn Error>> {
    if let Some(entries) = block.directives.get("request_header") {
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

fn validate_upstream_directives(block: &ServerConfigurationBlock) -> Result<(), Box<dyn Error>> {
    if let Some(entries) = block.directives.get("upstream") {
        for e in entries {
            if e.args
                .first()
                .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
                .is_none()
            {
                return Err("The `upstream` directive requires a URL argument".into());
            }
            if let Some(up_block) = &e.children {
                validate_upstream_block(up_block)?;
            }
        }
    }
    Ok(())
}

fn validate_upstream_block(block: &ServerConfigurationBlock) -> Result<(), Box<dyn Error>> {
    validate_active_check_directives(block)?;
    validate_number(block, "limit", 1)?;
    validate_duration(block, "idle_timeout")?;
    validate_interpolated_str(block, "unix")?;
    #[cfg(not(unix))]
    if block.directives.contains_key("unix") {
        return Err("Unix sockets are not supported on this platform".into());
    }
    Ok(())
}

/// Validate SRV upstream directives.
#[cfg(feature = "srv-lookup")]
fn validate_srv_directives(block: &ServerConfigurationBlock) -> Result<(), Box<dyn Error>> {
    if let Some(entries) = block.directives.get("srv") {
        for e in entries {
            if e.args.first().and_then(|v| v.as_str()).is_none() {
                return Err("The `srv` directive requires an SRV record name argument".into());
            }
            if let Some(srv_block) = &e.children {
                validate_srv_block(srv_block)?;
            }
        }
    }
    Ok(())
}

#[cfg(feature = "srv-lookup")]
fn validate_srv_block(block: &ServerConfigurationBlock) -> Result<(), Box<dyn Error>> {
    validate_number(block, "limit", 1)?;
    validate_duration(block, "idle_timeout")?;
    validate_str(block, "dns_servers")?;
    Ok(())
}

fn validate_passive_check_directives(
    block: &ServerConfigurationBlock,
) -> Result<(), Box<dyn Error>> {
    if let Some(block) = block
        .directives
        .get("passive_check")
        .and_then(|d| d.first())
        .and_then(|d| d.children.as_ref())
    {
        validate_number(block, "max_fails", 0)?;
        validate_duration(block, "window")?;
    }
    Ok(())
}

fn validate_active_check_directives(
    block: &ServerConfigurationBlock,
) -> Result<(), Box<dyn Error>> {
    if let Some(block) = block
        .directives
        .get("active_check")
        .and_then(|d| d.first())
        .and_then(|d| d.children.as_ref())
    {
        validate_str(block, "uri")?;
        validate_str(block, "method")?;
        validate_duration(block, "interval")?;
        validate_duration(block, "timeout")?;
        validate_str(block, "expect_status")?;
        validate_duration(block, "response_time_threshold")?;
        validate_str(block, "body_match")?;
        validate_number(block, "consecutive_fails", 1)?;
        validate_number(block, "consecutive_passes", 1)?;
        validate_bool(block, "no_verification")?;
    }

    Ok(())
}

fn validate_circuit_breaker_directives(
    block: &ServerConfigurationBlock,
) -> Result<(), Box<dyn Error>> {
    if let Some(block) = block
        .directives
        .get("circuit_breaker")
        .and_then(|d| d.first())
        .and_then(|d| d.children.as_ref())
    {
        validate_number(block, "max_fails", 1)?;
        validate_duration(block, "window")?;
        validate_duration(block, "open_duration")?;
        validate_number(block, "consecutive_passes", 1)?;
    }

    Ok(())
}
