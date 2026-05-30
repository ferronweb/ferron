use ferron_core::config::ServerConfigurationValue;

pub struct ScgiConfigurationValidator;

impl ferron_core::config::validator::ConfigurationValidator for ScgiConfigurationValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let used_directives = &mut ctx.used_directives;
        ferron_core::validate_directive!(config, used_directives, scgi, optional args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::Boolean(_, _)], {
            let mut sub = std::collections::HashSet::new();
            ferron_core::validate_nested!(scgi, used(sub), backend, args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)]);
            ferron_core::validate_nested!(scgi, used(sub), environment, args(2) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _), ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)]);
            ferron_core::check_unused_subdirectives!(scgi, sub, &mut ctx.diagnostics, ctx.scope.clone());
        });

        Ok(())
    }
}
