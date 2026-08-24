use cidr::IpCidr;
use ferron_core::config::validator::{
    entry_span, ConfigurationValidationError, ConfigurationValidator,
};
use ferron_core::config::{ServerConfigurationBlock, ServerConfigurationValue};

#[inline]
fn validate_ip_directive(
    config: &ServerConfigurationBlock,
    used_directives: &mut std::collections::HashSet<String>,
    directive: &str,
) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
    if let Some(entries) = config.directives.get(directive) {
        for entry in entries {
            if entry.args.is_empty() {
                return Err(ConfigurationValidationError::from(format!(
                    "Invalid `{directive}` — directive requires at least one IP or CIDR argument"
                ))
                .with_span(entry_span(entry)));
            }
            for arg in &entry.args {
                if let Some(s) = arg.as_str() {
                    if s.parse::<IpCidr>().is_err() {
                        return Err(ConfigurationValidationError::from(format!(
                            "Invalid `{directive}` — invalid IP or CIDR: {s}"
                        ))
                        .with_span(entry_span(entry)));
                    }
                } else {
                    return Err(ConfigurationValidationError::from(format!(
                        "Invalid `{directive}` — values must be strings (IP or CIDR)"
                    ))
                    .with_span(entry_span(entry)));
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
    ) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
        let used_directives = &mut ctx.used_directives;
        ferron_core::validate_directive!(config, used_directives, abort, optional args(1) => [ServerConfigurationValue::Boolean(_, _)], {});

        validate_ip_directive(config, used_directives, "block")?;
        validate_ip_directive(config, used_directives, "allow")?;

        if let Some(entries) = config.directives.get("status") {
            used_directives.insert("status".to_string());
            for entry in entries {
                if entry.args.is_empty() {
                    return Err(ConfigurationValidationError::from(
                        "Invalid `status` — directive requires a status code as its first argument",
                    )
                    .with_span(entry_span(entry)));
                }

                let status_code = entry.args[0].as_number().ok_or_else(|| {
                    ConfigurationValidationError::from("Invalid `status` — code must be an integer")
                        .with_span(entry_span(entry))
                })?;

                if !(100..=599).contains(&status_code) {
                    return Err(ConfigurationValidationError::from(format!(
                        "Invalid `status` — must be a valid HTTP status code (100-599), got {status_code}"
                    ))
                    .with_span(entry_span(entry)));
                }

                if let Some(children) = &entry.children {
                    for child_name in children.directives.keys() {
                        match child_name.as_str() {
                            "url" | "regex" | "location" | "body" => {
                                // Each should be a string value
                                if let Some(child_entries) = children.directives.get(child_name) {
                                    for child_entry in child_entries {
                                        if child_entry.args.is_empty() {
                                            return Err(ConfigurationValidationError::from(
                                                format!(
                                                "Invalid `{child_name}` — requires a string value"
                                            ),
                                            )
                                            .with_span(entry_span(child_entry)));
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
                                            return Err(ConfigurationValidationError::from(
                                                format!(
                                                "Invalid `{child_name}` — value must be a string"
                                            ),
                                            )
                                            .with_span(entry_span(child_entry)));
                                        }
                                    }
                                }
                            }
                            _ => (),
                        }
                    }

                    if let Some(regex_entries) = children.directives.get("regex") {
                        for regex_entry in regex_entries {
                            if let Some(regex_str) =
                                regex_entry.args.first().and_then(|v| v.as_str())
                            {
                                if regex::Regex::new(regex_str).is_err() {
                                    return Err(ConfigurationValidationError::from(format!(
                                        "Invalid `regex` — invalid regular expression: {regex_str}"
                                    ))
                                    .with_span(entry_span(regex_entry)));
                                }
                            }
                        }
                    }
                }
            }
        }

        ferron_core::validate_directive!(config, used_directives, early_hints, optional args(1) => [ServerConfigurationValue::Boolean(_, _)], {
            let mut sub = std::collections::HashSet::new();
            ferron_core::validate_nested!(early_hints, used(sub), link, args(*) => [ServerConfigurationValue::String(_, _)]);
            ferron_core::check_unused_subdirectives!(early_hints, sub, &mut ctx.diagnostics, ctx.scope.clone());
        });

        Ok(())
    }
}
