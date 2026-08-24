use ferron_core::config::validator::{
    ConfigurationValidator, ConfigurationValidatorDiagnosticKind,
};
use ferron_core::config::ServerConfigurationValue;
use ferron_core::{validate_directive, validate_nested};

/// Characters that are not allowed in a StatsD metric name or prefix.
const INVALID_NAME_CHARS: &[char] = &[':', '|', '#', '@', ' ', '\t', '\n', '\r'];

pub struct StatsdObservabilityConfigurationValidator;

impl ConfigurationValidator for StatsdObservabilityConfigurationValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        validator_ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
        validate_directive!(config, validator_ctx.used_directives, host, optional args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)], {});
        validate_directive!(config, validator_ctx.used_directives, port, optional args(1) => [ServerConfigurationValue::Number(_, _)], {});
        validate_directive!(config, validator_ctx.used_directives, prefix, optional args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)], {});
        validate_directive!(config, validator_ctx.used_directives, datadog, optional args(1) => [ServerConfigurationValue::Boolean(_, _)], {});

        if let Some(baggage_entries) = config.directives.get("baggage") {
            validator_ctx.used_directives.insert("baggage".to_string());
            for baggage_entry in baggage_entries {
                if let Some(children) = &baggage_entry.children {
                    validate_baggage_block(children, validator_ctx)?;
                }
            }

            // Promoted baggage keys are only rendered as DogStatsD tags, which
            // require DogStatsD mode to be enabled.
            let datadog_enabled = config
                .get_value("datadog")
                .and_then(|v| v.as_boolean())
                .unwrap_or(false);
            if !datadog_enabled {
                validator_ctx.add_best_practice_violation(
                    "`baggage` promotion is configured but `datadog` is not enabled; promoted baggage keys are only emitted as DogStatsD tags, which require DogStatsD mode",
                    baggage_entries.first().and_then(|e| e.span.clone()),
                );
            }
        }

        if let Some(port_value) = config.get_value("port") {
            if let Some(port) = port_value.as_number() {
                if !(1..=65535).contains(&port) {
                    validator_ctx
                        .diagnostics
                        .push(validator_ctx.create_diagnostic(
                            ConfigurationValidatorDiagnosticKind::InvalidConfiguration,
                            format!(
                                "Invalid `port` value: must be between 1 and 65535, got {}",
                                port
                            ),
                            value_span(port_value),
                        ));
                }
            }
        }

        if let Some(prefix_value) = config.get_value("prefix") {
            if let Some(prefix) = prefix_value.as_str() {
                if let Some(bad) = prefix.chars().find(|c| INVALID_NAME_CHARS.contains(c)) {
                    validator_ctx
                        .diagnostics
                        .push(validator_ctx.create_diagnostic(
                            ConfigurationValidatorDiagnosticKind::InvalidConfiguration,
                            format!(
                                "Invalid `prefix` value: character `{}` is not allowed in StatsD metric names",
                                bad
                            ),
                            value_span(prefix_value),
                        ));
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
) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
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

            if let Some(children) = &key_entry.children {
                let mut sub = std::collections::HashSet::new();

                validate_nested!(children, used(sub), attribute, optional args(1) => [ServerConfigurationValue::String(_, _)]);

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

/// Extract the source span from a configuration value, if present.
fn value_span(
    value: &ServerConfigurationValue,
) -> Option<ferron_core::config::ServerConfigurationSpan> {
    match value {
        ServerConfigurationValue::String(_, span)
        | ServerConfigurationValue::Number(_, span)
        | ServerConfigurationValue::Float(_, span)
        | ServerConfigurationValue::Boolean(_, span)
        | ServerConfigurationValue::InterpolatedString(_, span) => span.clone(),
    }
}
