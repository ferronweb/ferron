//! Configuration validation for the forward proxy module.

use std::collections::HashSet;
use std::error::Error;

use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::config::ServerConfigurationDirectiveEntry;

/// Configuration validator for the forward proxy module.
pub struct ForwardProxyConfigurationValidator;

impl ConfigurationValidator for ForwardProxyConfigurationValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        used_directives: &mut HashSet<String>,
        is_global: bool,
    ) -> Result<(), Box<dyn Error>> {
        if is_global {
            return Ok(());
        }

        if let Some(entries) = config.directives.get("forward_proxy") {
            used_directives.insert("forward_proxy".to_string());
            validate_forward_proxy_entries(entries, used_directives)?;
        }
        Ok(())
    }
}

fn validate_forward_proxy_entries(
    entries: &[ServerConfigurationDirectiveEntry],
    used_directives: &mut HashSet<String>,
) -> Result<(), Box<dyn Error>> {
    for entry in entries {
        // Validate args: at most one boolean (the enable toggle)
        if entry.args.len() > 1 {
            return Err(
                "The `forward_proxy` directive may have at most one boolean argument".into(),
            );
        }
        if let Some(arg) = entry.args.first() {
            if arg.as_boolean().is_none() {
                return Err("Invalid `forward_proxy` — expected a boolean".into());
            }
        }

        // Validate block children
        if let Some(block) = &entry.children {
            validate_forward_proxy_block(block, used_directives)?;
        }
    }
    Ok(())
}

fn validate_forward_proxy_block(
    block: &ferron_core::config::ServerConfigurationBlock,
    used_directives: &mut HashSet<String>,
) -> Result<(), Box<dyn Error>> {
    // allow_domains — accepts string arguments
    if let Some(entries) = block.directives.get("allow_domains") {
        used_directives.insert("allow_domains".to_string());
        for e in entries {
            if e.args.is_empty() {
                return Err("The `allow_domains` directive requires at least one argument".into());
            }
            for arg in &e.args {
                if arg.as_str().is_none() {
                    return Err("Invalid `allow_domains` — expected a string".into());
                }
            }
        }
    }

    // allow_ports — accepts numeric arguments
    if let Some(entries) = block.directives.get("allow_ports") {
        used_directives.insert("allow_ports".to_string());
        for e in entries {
            if e.args.is_empty() {
                return Err("The `allow_ports` directive requires at least one argument".into());
            }
            for arg in &e.args {
                if let Some(val) = arg.as_number() {
                    if val <= 0 || val > 65535 {
                        return Err("Invalid `allow_ports` — must be between 1 and 65535".into());
                    }
                } else {
                    return Err("Invalid `allow_ports` — expected a number".into());
                }
            }
        }
    }

    // deny_ips — accepts CIDR string arguments
    if let Some(entries) = block.directives.get("deny_ips") {
        used_directives.insert("deny_ips".to_string());
        for e in entries {
            if e.args.is_empty() {
                return Err("The `deny_ips` directive requires at least one argument".into());
            }
            for arg in &e.args {
                if arg.as_str().is_none() {
                    return Err("Invalid `deny_ips` — expected a CIDR string".into());
                }
            }
        }
    }

    // connect_method — boolean
    validate_bool(block, used_directives, "connect_method")?;

    // http_version — enum
    if let Some(entries) = block.directives.get("http_version") {
        used_directives.insert("http_version".to_string());
        for e in entries {
            if let Some(val) = e.args.first().and_then(|v| v.as_str()) {
                if val != "1.0" && val != "1.1" {
                    return Err("Invalid `http_version` — expected 1.0 or 1.1".into());
                }
            } else {
                return Err("Invalid `http_version` — expected a string".into());
            }
        }
    }

    Ok(())
}

fn validate_bool(
    block: &ferron_core::config::ServerConfigurationBlock,
    used: &mut HashSet<String>,
    name: &str,
) -> Result<(), Box<dyn Error>> {
    if let Some(entries) = block.directives.get(name) {
        used.insert(name.to_string());
        for e in entries {
            if e.args.is_empty() {
                continue; // allow empty args (treat as true)
            }
            if e.args.first().and_then(|v| v.as_boolean()).is_none() {
                return Err(format!("Invalid `{name}` — expected a boolean").into());
            }
        }
    }
    Ok(())
}
