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

        // Validate the port range (1-65535)
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

        // Validate that the prefix does not contain StatsD reserved characters
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

#[cfg(test)]
mod tests {
    use ferron_core::config::validator::{
        ConfigurationValidatorContext, ConfigurationValidatorDiagnosticKind,
    };
    use ferron_core::config::ServerConfigurationValue;

    use super::*;

    fn ctx() -> ConfigurationValidatorContext {
        ConfigurationValidatorContext {
            used_directives: std::collections::HashSet::new(),
            is_global: false,
            // Test-only context: the empty validator map is not shared across threads.
            #[allow(clippy::arc_with_non_send_sync)]
            scoped_validators: std::sync::Arc::new(std::collections::HashMap::new()),
            diagnostics: Vec::new(),
            scope: Some(String::from("observability.statsd")),
        }
    }

    fn make_block(
        directives: &[(&str, ServerConfigurationValue)],
    ) -> ferron_core::config::ServerConfigurationBlock {
        use std::collections::HashMap;
        use std::sync::Arc;

        let mut map = HashMap::new();
        for (name, value) in directives {
            map.insert(
                name.to_string(),
                vec![ferron_core::config::ServerConfigurationDirectiveEntry {
                    args: vec![value.clone()],
                    children: None,
                    span: None,
                }],
            );
        }
        ferron_core::config::ServerConfigurationBlock {
            directives: Arc::new(map),
            matchers: HashMap::new(),
            span: None,
        }
    }

    #[test]
    fn valid_config_passes() {
        let block = make_block(&[
            (
                "host",
                ServerConfigurationValue::String("127.0.0.1".to_string(), None),
            ),
            ("port", ServerConfigurationValue::Number(8125, None)),
            (
                "prefix",
                ServerConfigurationValue::String("myapp".to_string(), None),
            ),
            ("datadog", ServerConfigurationValue::Boolean(true, None)),
        ]);
        let mut context = ctx();
        let validator = StatsdObservabilityConfigurationValidator;
        let result = validator.validate_block(&block, &mut context);
        assert!(result.is_ok());
        assert!(context.diagnostics.is_empty());
    }

    #[test]
    fn invalid_port_is_rejected() {
        let block = make_block(&[("port", ServerConfigurationValue::Number(70000, None))]);
        let mut context = ctx();
        let validator = StatsdObservabilityConfigurationValidator;
        validator.validate_block(&block, &mut context).unwrap();
        assert!(context.diagnostics.iter().any(|d| {
            matches!(
                d.kind,
                ConfigurationValidatorDiagnosticKind::InvalidConfiguration
            ) && d.message.contains("port")
        }));
    }

    #[test]
    fn invalid_prefix_is_rejected() {
        let block = make_block(&[(
            "prefix",
            ServerConfigurationValue::String("bad|prefix".to_string(), None),
        )]);
        let mut context = ctx();
        let validator = StatsdObservabilityConfigurationValidator;
        validator.validate_block(&block, &mut context).unwrap();
        assert!(context.diagnostics.iter().any(|d| {
            matches!(
                d.kind,
                ConfigurationValidatorDiagnosticKind::InvalidConfiguration
            ) && d.message.contains("prefix")
        }));
    }
}
