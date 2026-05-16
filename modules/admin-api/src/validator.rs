//! Admin configuration validator.
//!
//! Validates the `admin { ... }` global configuration directive.

use std::collections::HashSet;

use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::config::ServerConfigurationValue;

pub struct AdminConfigurationValidator;

impl ConfigurationValidator for AdminConfigurationValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        used_directives: &mut HashSet<String>,
        is_global: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if is_global {
            ferron_core::validate_directive!(config, used_directives, admin, optional
                args(1) => [ServerConfigurationValue::Boolean(_, _)], {

                // Listen address
                ferron_core::validate_nested!(admin, listen, args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)]);

                // Endpoint flags
                ferron_core::validate_nested!(admin, health, optional args(1) => [ServerConfigurationValue::Boolean(_, _)]);
                ferron_core::validate_nested!(admin, status, optional args(1) => [ServerConfigurationValue::Boolean(_, _)]);
                ferron_core::validate_nested!(admin, config, optional args(1) => [ServerConfigurationValue::Boolean(_, _)]);
                ferron_core::validate_nested!(admin, reload, optional args(1) => [ServerConfigurationValue::Boolean(_, _)]);
            });
        }

        Ok(())
    }
}
