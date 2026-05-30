use std::collections::HashMap;
use std::error::Error;
use std::str::FromStr;

use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::config::{ServerConfigurationBlock, ServerConfigurationValue};
use http::header::HeaderName;

/// Configuration validator for the HTTP headers module.
pub struct HttpHeadersConfigurationValidator;

impl ConfigurationValidator for HttpHeadersConfigurationValidator {
    fn validate_block(
        &self,
        config: &ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let _is_global = ctx.is_global;
        let used_directives = &mut ctx.used_directives;
        // Validate header directives
        if let Some(entries) = config.directives.get("header") {
            used_directives.insert("header".to_string());
            for e in entries {
                if e.args.is_empty() {
                    return Err("Invalid `header` — requires at least one argument".into());
                }
                let first = e.args[0]
                    .as_str()
                    .ok_or("Invalid `header` — name must be a string")?;
                let (name, needs_value) = match first.chars().next() {
                    Some('+') => (&first[1..], true),
                    Some('-') => (&first[1..], false),
                    _ => (first, true),
                };
                HeaderName::from_str(name)
                    .map_err(|e| format!("Invalid `header` — invalid header name '{name}': {e}"))?;
                if needs_value
                    && e.args
                        .get(1)
                        .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
                        .is_none()
                {
                    return Err(
                        "Invalid `header` — requires a value for add/replace operations".into(),
                    );
                }
            }
        }

        // Validate cors directives
        if let Some(entries) = config.directives.get("cors") {
            used_directives.insert("cors".to_string());
            for e in entries {
                if let Some(block) = &e.children {
                    validate_cors_block(block, ctx)?;
                }
            }
        }

        Ok(())
    }
}

fn validate_cors_block(
    block: &ServerConfigurationBlock,
    ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
) -> Result<(), Box<dyn Error>> {
    let mut sub = std::collections::HashSet::new();
    ferron_core::validate_nested!(block, used(sub), origins, args(*) => [ServerConfigurationValue::String(_, _)]);
    ferron_core::validate_nested!(block, used(sub), methods, args(*) => [ServerConfigurationValue::String(_, _)]);
    ferron_core::validate_nested!(block, used(sub), headers, args(*) => [ServerConfigurationValue::String(_, _)]);
    ferron_core::validate_nested!(block, used(sub), credentials, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);
    ferron_core::validate_nested!(block, used(sub), max_age, optional args(1) => [ServerConfigurationValue::Number(_, _) | ServerConfigurationValue::Float(_, _) | ServerConfigurationValue::String(_, _)]);
    ferron_core::validate_nested!(block, used(sub), expose_headers, args(*) => [ServerConfigurationValue::String(_, _)]);
    ferron_core::check_unused_subdirectives!(block, sub, &mut ctx.diagnostics, ctx.scope.clone());
    Ok(())
}
