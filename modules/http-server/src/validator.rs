use ferron_core::{
    config::ServerConfigurationValue, validate_args, validate_directive, validate_nested,
};

pub struct HttpConfigurationValidator;

impl ferron_core::config::validator::ConfigurationValidator for HttpConfigurationValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        used_directives: &mut std::collections::HashSet<String>,
        _is_global: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        validate_directive!(config, used_directives, tls, optional
            args(1) => [ServerConfigurationValue::Boolean(_, _)]
            | args(2) => [ServerConfigurationValue::String(_, _), ServerConfigurationValue::String(_, _)],
            {
            validate_nested!(tls, "provider", args(1) => ServerConfigurationValue::String(_, _));
        });

        Ok(())
    }
}
