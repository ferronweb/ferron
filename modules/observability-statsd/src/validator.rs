use ferron_core::config::validator::{
    ConfigurationValidator, ConfigurationValidatorDiagnosticKind,
};
use ferron_core::config::ServerConfigurationValue;
use ferron_core::validate_directive;

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
