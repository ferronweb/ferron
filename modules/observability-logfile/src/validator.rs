use ferron_core::{
    config::{
        validator::{validate_scoped_block_flat, ConfigurationValidator},
        ServerConfigurationValue,
    },
    validate_directive,
};

pub struct LogFileObservabilityConfigurationValidator;

impl ConfigurationValidator for LogFileObservabilityConfigurationValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        validator_ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Log format
        validate_scoped_block_flat(config, validator_ctx, "format", "logformat", Some("text"))?;
        validate_scoped_block_flat(
            config,
            validator_ctx,
            "error_format",
            "logformat_application",
            Some("text"),
        )?;

        // Access log
        validate_directive!(config, validator_ctx.used_directives, access_log, optional args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)], {});
        // Including log rotation
        validate_directive!(config, validator_ctx.used_directives, access_log_rotate_size, optional args(1) => [ServerConfigurationValue::Number(_, _)], {});
        validate_directive!(config, validator_ctx.used_directives, access_log_rotate_keep, optional args(1) => [ServerConfigurationValue::Number(_, _)], {});

        // Error log
        validate_directive!(config, validator_ctx.used_directives, error_log, optional args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)], {});
        // Including log rotation
        validate_directive!(config, validator_ctx.used_directives, error_log_rotate_size, optional args(1) => [ServerConfigurationValue::Number(_, _)], {});
        validate_directive!(config, validator_ctx.used_directives, error_log_rotate_keep, optional args(1) => [ServerConfigurationValue::Number(_, _)], {});

        Ok(())
    }
}
