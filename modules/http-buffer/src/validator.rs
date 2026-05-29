//! Configuration validator for `buffer_request` and `buffer_response` directives.
//!
//! Validates that both directives, if present, contain either an integer
//! (buffer size in bytes) or `#null` (disabled).

use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::config::ServerConfigurationBlock;
use ferron_core::validate_directive;

/// Validator for HTTP buffer configuration blocks.
#[derive(Default)]
pub struct HttpBufferConfigurationValidator;

impl ConfigurationValidator for HttpBufferConfigurationValidator {
    fn validate_block(
        &self,
        config: &ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let used_directives = &mut ctx.used_directives;
        validate_directive!(config, used_directives, buffer_request, optional
            args(1) => [ferron_core::config::ServerConfigurationValue::Number(_, _)], {});

        validate_directive!(config, used_directives, buffer_response, optional
            args(1) => [ferron_core::config::ServerConfigurationValue::Number(_, _)], {});

        Ok(())
    }
}
