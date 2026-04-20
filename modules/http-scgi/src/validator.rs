use ferron_core::config::ServerConfigurationValue;

pub struct ScgiConfigurationValidator;

impl ferron_core::config::validator::ConfigurationValidator for ScgiConfigurationValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        used_directives: &mut std::collections::HashSet<String>,
        _is_global: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        ferron_core::validate_directive!(config, used_directives, scgi, optional args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::Boolean(_, _)], {
            ferron_core::validate_nested!(scgi, backend, args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)]);
            ferron_core::validate_nested!(scgi, environment, args(2) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _), ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)]);
        });

        Ok(())
    }
}
