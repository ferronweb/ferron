//! Configuration validation for the forward proxy module.

use std::error::Error;

use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::config::{ServerConfigurationDirectiveEntry, ServerConfigurationValue};

/// Configuration validator for the forward proxy module.
pub struct ForwardProxyConfigurationValidator;

impl ConfigurationValidator for ForwardProxyConfigurationValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let used_directives = &mut ctx.used_directives;
        if let Some(entries) = config.directives.get("forward_proxy") {
            used_directives.insert("forward_proxy".to_string());
            validate_forward_proxy_entries(entries)?;
        }
        Ok(())
    }
}

fn validate_forward_proxy_entries(
    entries: &[ServerConfigurationDirectiveEntry],
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
            validate_forward_proxy_block(block)?;
        }
    }
    Ok(())
}

fn validate_forward_proxy_block(
    block: &ferron_core::config::ServerConfigurationBlock,
) -> Result<(), Box<dyn Error>> {
    ferron_core::validate_nested!(block, allow_domains, args(*) => [ServerConfigurationValue::String(_, _)]);
    ferron_core::validate_nested!(block, deny_ips, args(*) => [ServerConfigurationValue::String(_, _)]);
    ferron_core::validate_nested!(block, connect_method, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);

    // -- manual validation ---------------
    // allow_ports — accepts numeric arguments
    if let Some(entries) = block.directives.get("allow_ports") {
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

    // http_version — enum
    if let Some(entries) = block.directives.get("http_version") {
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
