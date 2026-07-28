use std::net::IpAddr;
use std::str::FromStr;

use ferron_core::config::validator::{
    entry_span, ConfigurationValidationError, ConfigurationValidator, ConfigurationValidatorContext,
};
use ferron_core::config::{ServerConfigurationDirectiveEntry, ServerConfigurationValue};
use ipnet::IpNet;

pub struct ForwardProxyConfigurationValidator;

impl ConfigurationValidator for ForwardProxyConfigurationValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
        let used_directives = &mut ctx.used_directives;
        if let Some(entries) = config.directives.get("forward_proxy") {
            used_directives.insert("forward_proxy".to_string());
            validate_forward_proxy_entries(entries, ctx)?;
        }
        Ok(())
    }
}

fn validate_forward_proxy_entries(
    entries: &[ServerConfigurationDirectiveEntry],
    ctx: &mut ConfigurationValidatorContext,
) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
    for entry in entries {
        if entry.args.len() > 1 {
            return Err(ConfigurationValidationError::from(
                "The `forward_proxy` directive may have at most one boolean argument",
            )
            .with_span(entry_span(entry)));
        }
        if let Some(arg) = entry.args.first() {
            if arg.as_boolean().is_none() {
                return Err(ConfigurationValidationError::from(
                    "Invalid `forward_proxy` — expected a boolean",
                )
                .with_span(entry_span(entry)));
            }
        }

        if let Some(block) = &entry.children {
            validate_forward_proxy_block(block, ctx)?;
        }
    }
    Ok(())
}

fn validate_forward_proxy_block(
    block: &ferron_core::config::ServerConfigurationBlock,
    ctx: &mut ConfigurationValidatorContext,
) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
    let mut sub = std::collections::HashSet::new();

    ferron_core::validate_nested!(block, used(sub), allow_domains, args(*) => [ServerConfigurationValue::String(_, _)]);
    ferron_core::validate_nested!(block, used(sub), connect_method, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);

    if let Some(entries) = block.directives.get("allow_domains") {
        for entry in entries {
            if entry
                .args
                .iter()
                .any(|arg| arg.as_str().is_some_and(|domain| domain == "*"))
            {
                ctx.add_best_practice_violation(
                    "`allow_domains \"*\"` permits proxying to any public domain; restrict forward proxy access to the destinations your clients actually need",
                    entry_span(entry),
                );
            }
        }
    }

    if let Some(entries) = block.directives.get("deny_ips") {
        sub.insert("deny_ips".to_string());
        validate_denied_ips(entries)?;
        add_deny_ips_best_practice_diagnostics(entries, ctx);
    }

    if let Some(entries) = block.directives.get("allow_ports") {
        sub.insert("allow_ports".to_string());
        for e in entries {
            if e.args.is_empty() {
                return Err(ConfigurationValidationError::from(
                    "The `allow_ports` directive requires at least one argument",
                )
                .with_span(entry_span(e)));
            }
            for arg in &e.args {
                if let Some(val) = arg.as_number() {
                    if val <= 0 || val > 65535 {
                        return Err(ConfigurationValidationError::from(
                            "Invalid `allow_ports` — must be between 1 and 65535",
                        )
                        .with_span(entry_span(e)));
                    }
                } else {
                    return Err(ConfigurationValidationError::from(
                        "Invalid `allow_ports` — expected a number",
                    )
                    .with_span(entry_span(e)));
                }
            }
        }
    }

    if let Some(entries) = block.directives.get("http_version") {
        sub.insert("http_version".to_string());
        for e in entries {
            if let Some(val) = e.args.first().and_then(|v| v.as_str()) {
                if val != "1.0" && val != "1.1" {
                    return Err(ConfigurationValidationError::from(
                        "Invalid `http_version` — expected 1.0 or 1.1",
                    )
                    .with_span(entry_span(e)));
                }
            } else {
                return Err(ConfigurationValidationError::from(
                    "Invalid `http_version` — expected a string",
                )
                .with_span(entry_span(e)));
            }
        }
    }

    ferron_core::check_unused_subdirectives!(block, sub, &mut ctx.diagnostics, ctx.scope.clone());
    Ok(())
}

fn validate_denied_ips(
    entries: &[ServerConfigurationDirectiveEntry],
) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
    for entry in entries {
        if entry.args.is_empty() {
            return Err(ConfigurationValidationError::from(
                "The `deny_ips` directive requires at least one IP or CIDR",
            )
            .with_span(entry_span(entry)));
        }
        for arg in &entry.args {
            let Some(value) = arg.as_str() else {
                return Err(ConfigurationValidationError::from(
                    "Invalid `deny_ips` — expected string IP/CIDR values",
                )
                .with_span(entry_span(entry)));
            };
            if value.parse::<IpNet>().is_err() && value.parse::<IpAddr>().is_err() {
                return Err(ConfigurationValidationError::from(format!(
                    "Invalid `deny_ips` — invalid IP or CIDR `{value}`"
                ))
                .with_span(entry_span(entry)));
            }
        }
    }
    Ok(())
}

fn deny_entries_contain_ip(entries: &[ServerConfigurationDirectiveEntry], ip: IpAddr) -> bool {
    entries.iter().any(|entry| {
        entry.args.iter().any(|arg| {
            let Some(value) = arg.as_str() else {
                return false;
            };
            if let Ok(net) = value.parse::<IpNet>() {
                net.contains(&ip)
            } else {
                value
                    .parse::<IpAddr>()
                    .is_ok_and(|candidate| candidate == ip)
            }
        })
    })
}

fn add_deny_ips_best_practice_diagnostics(
    entries: &[ServerConfigurationDirectiveEntry],
    ctx: &mut ConfigurationValidatorContext,
) {
    let loopback_v4 = IpAddr::from_str("127.0.0.1").expect("valid IP");
    let loopback_v6 = IpAddr::from_str("::1").expect("valid IP");
    let metadata = IpAddr::from_str("169.254.169.254").expect("valid IP");

    if !deny_entries_contain_ip(entries, loopback_v4)
        || !deny_entries_contain_ip(entries, loopback_v6)
        || !deny_entries_contain_ip(entries, metadata)
    {
        ctx.add_best_practice_violation(
            "Custom `deny_ips` overrides the built-in forward-proxy deny list; include loopback and cloud metadata ranges unless this proxy is isolated by stronger network controls",
            entries.first().and_then(entry_span),
        );
    }
}
