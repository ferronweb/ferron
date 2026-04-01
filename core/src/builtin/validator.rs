use crate::{config::ServerConfigurationValue, validate_directive, validate_nested};

pub struct BuiltinGlobalConfigurationValidator;

impl crate::config::validator::ConfigurationValidator for BuiltinGlobalConfigurationValidator {
    fn validate_block(
        &self,
        config: &crate::config::ServerConfigurationBlock,
        used_directives: &mut std::collections::HashSet<String>,
        is_global: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if is_global {
            validate_directive!(config, used_directives, runtime, no_args, {
                validate_nested!(runtime, "io_uring", optional args(1) => [ServerConfigurationValue::Boolean(_, _)]);
            });
        }

        Ok(())
    }
}
