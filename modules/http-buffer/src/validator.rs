//! Configuration validator for `buffer_request` and `buffer_response` directives.
//!
//! Validates that both directives, if present, contain either an integer
//! (buffer size in bytes) or `false` (disabled).

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
    ) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
        let used_directives = &mut ctx.used_directives;
        validate_directive!(config, used_directives, buffer_request, optional
            args(1) => [ferron_core::config::ServerConfigurationValue::Number(_, _)], {});

        validate_directive!(config, used_directives, buffer_response, optional
            args(1) => [ferron_core::config::ServerConfigurationValue::Number(_, _)], {});

        for entry in config
            .directives
            .get("buffer_request")
            .iter()
            .map(|v| v.iter())
            .chain(
                config
                    .directives
                    .get("buffer_response")
                    .iter()
                    .map(|v| v.iter()),
            )
            .flatten()
        {
            if entry
                .get_value()
                .and_then(|e| e.as_number())
                .is_some_and(|e| e.is_negative())
            {
                return Err(
                    ferron_core::config::validator::ConfigurationValidationError::from(
                        "HTTP request/response buffer sizes must not be negative",
                    )
                    .with_span(entry.span.clone()),
                );
            }
        }

        Ok(())
    }
}
