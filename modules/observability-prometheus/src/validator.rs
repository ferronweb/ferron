use ferron_core::config::validator::{
    ConfigurationValidator, ConfigurationValidatorDiagnosticKind,
};
use ferron_core::config::ServerConfigurationValue;
use ferron_core::{validate_directive, validate_nested};

pub struct PrometheusObservabilityConfigurationValidator;

impl ConfigurationValidator for PrometheusObservabilityConfigurationValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        validator_ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Prometheus endpoint configuration
        validate_directive!(config, validator_ctx.used_directives, endpoint_listen, optional args(1) => [ServerConfigurationValue::String(_, _)], {});
        validate_directive!(config, validator_ctx.used_directives, endpoint_format, optional args(1) => [ServerConfigurationValue::String(_, _)], {});

        let endpoint_listen_value = config.get_value("endpoint_listen");

        // Check if Prometheus endpoint seems to be publicly exposed and emit a warning if so
        if let Some(endpoint_listen) = endpoint_listen_value.and_then(|value| value.as_str()) {
            let Ok(addr) = endpoint_listen.parse::<std::net::SocketAddr>() else {
                return Ok(());
            };

            if !addr.ip().is_loopback() {
                validator_ctx.add_best_practice_violation(
                    "`admin.listen` is not bound to a loopback address; the admin API is unauthenticated, unencrypted, and should only be reachable through a trusted local or protected management path",
                    endpoint_listen_value.and_then(|v| match v {
                        ServerConfigurationValue::String(_, span)
                        | ServerConfigurationValue::Number(_, span)
                        | ServerConfigurationValue::Float(_, span)
                        | ServerConfigurationValue::Boolean(_, span)
                        | ServerConfigurationValue::InterpolatedString(_, span) => span.clone(),
                    }),
                );
            }
        }

        // Validate `baggage { key "..." { ... } }` block
        if let Some(baggage_entries) = config.directives.get("baggage") {
            validator_ctx.used_directives.insert("baggage".to_string());
            for baggage_entry in baggage_entries {
                if let Some(children) = &baggage_entry.children {
                    validate_baggage_block(children, validator_ctx)?;
                }
            }
        }

        Ok(())
    }
}

/// Validate the contents of a `baggage { ... }` block.
fn validate_baggage_block(
    block: &ferron_core::config::ServerConfigurationBlock,
    validator_ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(key_entries) = block.directives.get("key") {
        for key_entry in key_entries {
            // Each `key` must have exactly 1 string argument (the baggage key name)
            if key_entry.args.len() != 1 {
                validator_ctx.diagnostics.push(validator_ctx.create_diagnostic(
                    ConfigurationValidatorDiagnosticKind::InvalidConfiguration,
                    format!(
                        "Invalid `baggage key` directive: expected 1 argument (the baggage key name), got {}",
                        key_entry.args.len()
                    ),
                    key_entry.span.clone(),
                ));
                continue;
            }
            if !matches!(&key_entry.args[0], ServerConfigurationValue::String(_, _)) {
                validator_ctx
                    .diagnostics
                    .push(validator_ctx.create_diagnostic(
                        ConfigurationValidatorDiagnosticKind::InvalidConfiguration,
                        "Invalid `baggage key` directive: argument must be a string".to_string(),
                        key_entry.span.clone(),
                    ));
                continue;
            }

            // Validate children of the key block
            if let Some(children) = &key_entry.children {
                let mut sub = std::collections::HashSet::new();

                // `attribute` — optional string
                validate_nested!(children, used(sub), attribute, optional args(1) => [ServerConfigurationValue::String(_, _)]);

                // `max_distinct` — optional number
                if let Some(max_entries) = children.directives.get("max_distinct") {
                    sub.insert("max_distinct".to_string());
                    for max_entry in max_entries {
                        if max_entry.args.len() != 1 {
                            validator_ctx.diagnostics.push(
                                validator_ctx.create_diagnostic(
                                    ConfigurationValidatorDiagnosticKind::InvalidConfiguration,
                                    "Invalid `max_distinct` directive: expected exactly 1 argument"
                                        .to_string(),
                                    max_entry.span.clone(),
                                ),
                            );
                        } else if !matches!(
                            &max_entry.args[0],
                            ServerConfigurationValue::Number(_, _)
                                | ServerConfigurationValue::Boolean(false, _)
                        ) {
                            validator_ctx.diagnostics.push(
                                validator_ctx.create_diagnostic(
                                    ConfigurationValidatorDiagnosticKind::InvalidConfiguration,
                                    "Invalid `max_distinct` value: must be a number or `false`"
                                        .to_string(),
                                    max_entry.span.clone(),
                                ),
                            );
                        }
                        if max_entry.args[0].as_boolean().is_some_and(|v| !v) {
                            validator_ctx.add_best_practice_violation(
                                "`max_distinct` set to `false`, high cardinality might be allowed.",
                                max_entry.span.clone(),
                            );
                        }
                    }
                }

                ferron_core::check_unused_subdirectives!(
                    children,
                    sub,
                    &mut validator_ctx.diagnostics,
                    validator_ctx.scope.clone()
                );
            }
        }
    }

    Ok(())
}
