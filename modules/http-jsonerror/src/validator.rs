use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::config::{ServerConfigurationBlock, ServerConfigurationValue};

/// Validator for the `json_errors` directive.
#[derive(Default)]
pub struct JsonErrorConfigurationValidator;

impl ConfigurationValidator for JsonErrorConfigurationValidator {
    fn validate_block(
        &self,
        config: &ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
        let used_directives = &mut ctx.used_directives;

        ferron_core::validate_directive!(config, used_directives, json_errors, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)], {
            let mut sub = std::collections::HashSet::new();
            ferron_core::validate_nested!(json_errors, used(sub), format, args(1) => [ServerConfigurationValue::String(_, _)]);
            ferron_core::validate_nested!(json_errors, used(sub), type_uri, args(1) => [ServerConfigurationValue::String(_, _)]);
            ferron_core::validate_nested!(json_errors, used(sub), trace_id, args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);
            ferron_core::check_unused_subdirectives!(json_errors, sub, &mut ctx.diagnostics, ctx.scope.clone());
        });

        Ok(())
    }
}
