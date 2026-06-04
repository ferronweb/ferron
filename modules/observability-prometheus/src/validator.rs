use ferron_core::{
    config::{validator::ConfigurationValidator, ServerConfigurationValue},
    validate_directive,
};

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

        Ok(())
    }
}
