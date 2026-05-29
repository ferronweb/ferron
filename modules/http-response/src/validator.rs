//! Configuration validator for `status`, `abort`, `block`, and `allow` directives.

use std::collections::HashSet;

use cidr::IpCidr;
use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::config::{ServerConfigurationBlock, ServerConfigurationValue};

#[inline]
fn validate_ip_directive(
    config: &ServerConfigurationBlock,
    used_directives: &mut HashSet<String>,
    directive: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(entries) = config.directives.get(directive) {
        for entry in entries {
            if entry.args.is_empty() {
                return Err(format!(
                    "Invalid `{directive}` — directive requires at least one IP or CIDR argument"
                )
                .into());
            }
            for arg in &entry.args {
                if let Some(s) = arg.as_str() {
                    if s.parse::<IpCidr>().is_err() {
                        return Err(
                            format!("Invalid `{directive}` — invalid IP or CIDR: {s}").into()
                        );
                    }
                } else {
                    return Err(format!(
                        "Invalid `{directive}` — values must be strings (IP or CIDR)"
                    )
                    .into());
                }
            }
        }
        used_directives.insert(directive.to_string());
    }

    Ok(())
}

/// Validator for http-response related directives.
#[derive(Default)]
pub struct HttpResponseValidator;

impl ConfigurationValidator for HttpResponseValidator {
    fn validate_block(
        &self,
        config: &ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let used_directives = &mut ctx.used_directives;
        ferron_core::validate_directive!(config, used_directives, abort, optional args(1) => [ServerConfigurationValue::Boolean(_, _)], {});

        validate_ip_directive(config, used_directives, "block")?;
        validate_ip_directive(config, used_directives, "allow")?;

        // Validate `status` directives
        if let Some(entries) = config.directives.get("status") {
            used_directives.insert("status".to_string());
            for entry in entries {
                if entry.args.is_empty() {
                    return Err(
                        "Invalid `status` — directive requires a status code as its first argument"
                            .into(),
                    );
                }

                let status_code = entry.args[0]
                    .as_number()
                    .ok_or("Invalid `status` — code must be an integer")?;

                if !(100..=599).contains(&status_code) {
                    return Err(format!(
                        "Invalid `status` — must be a valid HTTP status code (100-599), got {status_code}"
                    )
                    .into());
                }

                // Validate child block directives
                if let Some(children) = &entry.children {
                    for child_name in children.directives.keys() {
                        match child_name.as_str() {
                            "url" | "regex" | "location" | "body" => {
                                // Each should be a string value
                                if let Some(child_entries) = children.directives.get(child_name) {
                                    for child_entry in child_entries {
                                        if child_entry.args.is_empty() {
                                            return Err(format!(
                                                "Invalid `{child_name}` — requires a string value"
                                            )
                                            .into());
                                        }
                                        if child_entry.args[0].as_str().is_none()
                                            && (child_name.as_str() == "regex"
                                                || !matches!(
                                                    child_entry.args[0],
                                                    ServerConfigurationValue::InterpolatedString(
                                                        _,
                                                        _,
                                                    ),
                                                ))
                                        {
                                            return Err(format!(
                                                "Invalid `{child_name}` — value must be a string"
                                            )
                                            .into());
                                        }
                                    }
                                }
                            }
                            _ => (),
                        }
                    }

                    // Validate regex if present
                    if let Some(regex_entries) = children.directives.get("regex") {
                        for entry in regex_entries {
                            if let Some(regex_str) = entry.args.first().and_then(|v| v.as_str()) {
                                if fancy_regex::Regex::new(regex_str).is_err() {
                                    return Err(format!(
                                        "Invalid `regex` — invalid regular expression: {regex_str}"
                                    )
                                    .into());
                                }
                            }
                        }
                    }
                }
            }
        }

        // Validate `early_hints` directives
        ferron_core::validate_directive!(config, used_directives, early_hints, optional args(1) => [ServerConfigurationValue::Boolean(_, _)], {
            ferron_core::validate_nested!(early_hints, link, args(*) => [ServerConfigurationValue::String(_, _)]);
        });

        Ok(())
    }
}
