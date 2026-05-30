use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::config::ServerConfigurationValue;

pub struct AdminConfigurationValidator;

impl ConfigurationValidator for AdminConfigurationValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let is_global = ctx.is_global;
        let used_directives = &mut ctx.used_directives;

        if is_global {
            ferron_core::validate_directive!(config, used_directives, admin, optional
                args(1) => [ServerConfigurationValue::Boolean(_, _)], {

                let mut sub = std::collections::HashSet::new();

                // Listen address
                ferron_core::validate_nested!(admin, used(sub), listen, args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)]);

                // Endpoint flags
                ferron_core::validate_nested!(admin, used(sub), health, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);
                ferron_core::validate_nested!(admin, used(sub), status, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);
                ferron_core::validate_nested!(admin, used(sub), config, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);
                ferron_core::validate_nested!(admin, used(sub), reload, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);
                ferron_core::validate_nested!(admin, used(sub), reload_get, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);
                ferron_core::validate_nested!(admin, used(sub), runtime, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);

                ferron_core::check_unused_subdirectives!(admin, sub, &mut ctx.diagnostics, ctx.scope.clone());
            });
        }

        Ok(())
    }
}
