use ferron_core::{
    config::{validator::ConfigurationValidator, ServerConfigurationValue},
    validate_directive,
};
use ferron_tls::validate_tls_common;

pub struct TlsHttpConfigurationValidator;

impl ConfigurationValidator for TlsHttpConfigurationValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        validator_ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        validate_tls_common!(config, validator_ctx);

        validate_directive!(config, validator_ctx.used_directives, url, optional args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)], {});
        validate_directive!(config, validator_ctx.used_directives, refresh_interval, optional args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _) | ServerConfigurationValue::Number(_, _)], {});
        validate_directive!(config, validator_ctx.used_directives, no_verification, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)], {});

        Ok(())
    }
}
