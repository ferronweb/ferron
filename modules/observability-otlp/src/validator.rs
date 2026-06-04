use ferron_core::{
    config::{
        validator::{validate_scoped_block_flat, ConfigurationValidator},
        ServerConfigurationValue,
    },
    validate_directive, validate_nested,
};

pub struct OtlpObservabilityConfigurationValidator;

impl ConfigurationValidator for OtlpObservabilityConfigurationValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        validator_ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        validate_scoped_block_flat(config, validator_ctx, "format", "logformat", Some("text"))?;

        validate_directive!(config, validator_ctx.used_directives, logs, optional args(1) => [ServerConfigurationValue::String(_, _)], {
            let mut sub = std::collections::HashSet::new();
            validate_nested!(logs, used(sub), protocol, optional args(1) => [ServerConfigurationValue::String(_, _)]);
            validate_nested!(logs, used(sub), authorization, optional args(1) => [ServerConfigurationValue::String(_, _)]);
            ferron_core::check_unused_subdirectives!(logs, sub, &mut validator_ctx.diagnostics, validator_ctx.scope.clone());
        });

        validate_directive!(config, validator_ctx.used_directives, metrics, optional args(1) => [ServerConfigurationValue::String(_, _)], {
            let mut sub = std::collections::HashSet::new();
            validate_nested!(metrics, used(sub), protocol, optional args(1) => [ServerConfigurationValue::String(_, _)]);
            validate_nested!(metrics, used(sub), authorization, optional args(1) => [ServerConfigurationValue::String(_, _)]);
            ferron_core::check_unused_subdirectives!(metrics, sub, &mut validator_ctx.diagnostics, validator_ctx.scope.clone());
        });

        validate_directive!(config, validator_ctx.used_directives, traces, optional args(1) => [ServerConfigurationValue::String(_, _)], {
            let mut sub = std::collections::HashSet::new();
            validate_nested!(traces, used(sub), protocol, optional args(1) => [ServerConfigurationValue::String(_, _)]);
            validate_nested!(traces, used(sub), authorization, optional args(1) => [ServerConfigurationValue::String(_, _)]);
            ferron_core::check_unused_subdirectives!(traces, sub, &mut validator_ctx.diagnostics, validator_ctx.scope.clone());
        });

        validate_directive!(config, validator_ctx.used_directives, service_name, optional args(1) => [ServerConfigurationValue::String(_, _)], {});

        validate_directive!(
            config,
            validator_ctx.used_directives,
            no_verification,
            optional,
            {}
        );

        Ok(())
    }
}
