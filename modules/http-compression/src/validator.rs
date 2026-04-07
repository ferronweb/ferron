//! Configuration validator for the HTTP dynamic compression module.

use ferron_core::validate_directive;

pub struct DynamicCompressionConfigurationValidator;

impl ferron_core::config::validator::ConfigurationValidator
    for DynamicCompressionConfigurationValidator
{
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        used_directives: &mut std::collections::HashSet<String>,
        _is_global: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Dynamic content compression (on-the-fly)
        validate_directive!(config, used_directives, dynamic_compressed, optional
            args(1) => [ferron_core::config::ServerConfigurationValue::Boolean(_, _)], {});

        Ok(())
    }
}
