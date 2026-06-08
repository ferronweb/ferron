//! Configuration validator for `buffer_request` and `buffer_response` directives.
//!
//! Validates that both directives, if present, contain either an integer
//! (buffer size in bytes) or `#null` (disabled).

use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::config::{ServerConfigurationBlock, ServerConfigurationValue};
use ferron_core::validate_directive;

/// Validator for HTTP buffer configuration blocks.
#[derive(Default)]
pub struct HttpTraceIdConfigurationValidator;

impl ConfigurationValidator for HttpTraceIdConfigurationValidator {
    fn validate_block(
        &self,
        config: &ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let used_directives = &mut ctx.used_directives;
        validate_directive!(config, used_directives, trace_id_header, optional
          args(1) => [ServerConfigurationValue::Boolean(_, _)]
        | args(0) => [ServerConfigurationValue::Boolean(_, _)], {
            let mut sub = std::collections::HashSet::new();

            ferron_core::validate_nested!(trace_id_header, used(sub), reflect_request, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);
            ferron_core::validate_nested!(trace_id_header, used(sub), header_name, args(1) => [ServerConfigurationValue::String(_, _)]);

            ferron_core::check_unused_subdirectives!(trace_id_header, sub, &mut ctx.diagnostics, ctx.scope.clone());
        });

        Ok(())
    }
}
