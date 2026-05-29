use ferron_core::{
    config::{
        validator::{validate_scoped_block, ConfigurationValidator},
        ServerConfigurationValue,
    },
    validate_directive,
};
use ferron_tls::validate_tls_common;

pub struct TlsAcmeConfigurationValidator;

impl ConfigurationValidator for TlsAcmeConfigurationValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        validator_ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        validate_tls_common!(config, validator_ctx);

        validate_directive!(config, validator_ctx.used_directives, challenge, optional args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)], {});
        validate_directive!(config, validator_ctx.used_directives, contact, optional args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)], {});
        validate_directive!(config, validator_ctx.used_directives, directory, optional args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)], {});
        validate_directive!(config, validator_ctx.used_directives, profile, optional args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)], {});
        validate_directive!(config, validator_ctx.used_directives, eab, optional args(2) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _), ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)], {});
        validate_directive!(config, validator_ctx.used_directives, cache, optional args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)], {});
        validate_directive!(config, validator_ctx.used_directives, save, optional args(*) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)], {});
        validate_directive!(config, validator_ctx.used_directives, post_obtain_command, optional args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)], {});
        validate_directive!(config, validator_ctx.used_directives, no_verification, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)], {});

        // Automatic TLS on demand
        validate_directive!(config, validator_ctx.used_directives, on_demand, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)], {});
        validate_directive!(config, validator_ctx.used_directives, on_demand_ask, optional args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)], {});
        validate_directive!(config, validator_ctx.used_directives, on_demand_ask_no_verification, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)], {});

        // DNS
        validate_directive!(config, validator_ctx.used_directives, dns, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)], {
            validate_scoped_block(dns, validator_ctx, "provider", "dns", None)?;
        });

        Ok(())
    }
}
