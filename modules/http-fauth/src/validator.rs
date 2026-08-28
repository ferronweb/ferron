use ferron_core::config::validator::{
    entry_span, first_entry_span, ConfigurationValidationError, ConfigurationValidator,
};
use ferron_core::config::{
    ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationSpan,
    ServerConfigurationValue,
};
use ferron_core::{check_unused_subdirectives, validate_directive, validate_nested};
use std::collections::HashMap;
use std::str::FromStr;

use http::header::HeaderName;

pub struct ForwardedAuthenticationConfigurationValidator;

impl ConfigurationValidator for ForwardedAuthenticationConfigurationValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), ConfigurationValidationError> {
        let is_global = ctx.is_global;
        let mut concurrent_conns_warn: Option<ServerConfigurationSpan> = None;
        let mut no_verification_warn: Option<ServerConfigurationSpan> = None;
        {
            let used_directives = &mut ctx.used_directives;
            if is_global {
                // Manual validation for auth_to_concurrent_conns directive
                if let Some(directives) =
                    config.directives.get(stringify!(auth_to_concurrent_conns))
                {
                    used_directives.insert(stringify!(auth_to_concurrent_conns).to_string());
                    for directive in directives {
                        if directive.args.len() != 1 {
                            return Err(ConfigurationValidationError::from(format!(
                                "Invalid directive '{}': expected {} argument(s), got {}",
                                stringify!(auth_to_concurrent_conns),
                                1,
                                directive.args.len()
                            ))
                            .with_span(entry_span(directive)));
                        }
                        if !matches!(
                            directive.args[0],
                            ServerConfigurationValue::Number(n, _) if n > 0
                        ) && !matches!(
                            directive.args[0],
                            ServerConfigurationValue::Boolean(false, _)
                        ) {
                            return Err(ConfigurationValidationError::from(format!(
                                "Invalid directive '{}': invalid type",
                                stringify!(auth_to_concurrent_conns)
                            ))
                            .with_span(entry_span(directive)));
                        }
                        if matches!(
                            directive.args[0],
                            ServerConfigurationValue::Boolean(false, _)
                        ) {
                            concurrent_conns_warn = entry_span(directive);
                        }
                    }
                };
            }

            validate_directive!(config, used_directives, auth_to, optional args(1) => [ServerConfigurationValue::Boolean(_, _) | ServerConfigurationValue::InterpolatedString(_, _) | ServerConfigurationValue::String(_, _)], {
                let mut sub = std::collections::HashSet::new();
                validate_nested!(auth_to, used(sub), url, args(1) => [ServerConfigurationValue::InterpolatedString(_, _) | ServerConfigurationValue::String(_, _)]);
                validate_nested!(auth_to, used(sub), unix, args(1) => [ServerConfigurationValue::InterpolatedString(_, _) | ServerConfigurationValue::String(_, _)]);
                validate_nested!(auth_to, used(sub), limit, args(1) => [ServerConfigurationValue::Number(_, _) | ServerConfigurationValue::Boolean(false, _)]);
                validate_nested!(auth_to, used(sub), idle_timeout, args(1) => [ServerConfigurationValue::Number(_, _) | ServerConfigurationValue::String(_, _) | ServerConfigurationValue::Boolean(false, _)]);
                validate_nested!(auth_to, used(sub), no_verification, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);
                if block_flag(auth_to, "no_verification") == Some(true) {
                    no_verification_warn = first_entry_span(auth_to, "no_verification");
                }
                validate_nested!(auth_to, used(sub), copy, args(*) => [ServerConfigurationValue::String(_, _)]);
                if auth_to.directives.contains_key("request_header") {
                    sub.insert("request_header".to_string());
                }
                validate_request_header(auth_to)?;
                validate_nested!(auth_to, used(sub), last, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);
                validate_nested!(auth_to, used(sub), intercept_errors, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);
                check_unused_subdirectives!(auth_to, sub, &mut ctx.diagnostics, ctx.scope.clone());
            });
        }

        if let Some(span) = concurrent_conns_warn {
            ctx.add_best_practice_violation(
                "`auth_to_concurrent_conns false` disables the global forwarded-auth connection limit; keep a bounded limit to protect authentication backends under load",
                Some(span),
            );
        }
        if let Some(span) = no_verification_warn {
            ctx.add_best_practice_violation(
                "`auth_to.no_verification` disables TLS certificate verification for the authentication backend; keep verification enabled outside tightly controlled internal test environments",
                Some(span),
            );
        }

        Ok(())
    }
}

fn block_flag(block: &ServerConfigurationBlock, directive: &str) -> Option<bool> {
    block
        .directives
        .get(directive)
        .and_then(|entries| entries.first())
        .map(ServerConfigurationDirectiveEntry::get_flag)
}

fn validate_request_header(
    block: &ServerConfigurationBlock,
) -> Result<(), ConfigurationValidationError> {
    if let Some(entries) = block.directives.get("request_header") {
        for entry in entries {
            if entry.args.is_empty() {
                return Err(ConfigurationValidationError::from(
                    "request_header requires at least one argument",
                )
                .with_span(entry_span(entry)));
            }
            let first = entry.args[0].as_str().ok_or_else(|| {
                ConfigurationValidationError::from("The header name must be a string")
                    .with_span(entry_span(entry))
            })?;
            let (name, needs_value) = match first.chars().next() {
                Some('+') => (&first[1..], true),
                Some('-') => (&first[1..], false),
                _ => (first, true),
            };
            HeaderName::from_str(name).map_err(|parse_err| {
                ConfigurationValidationError::from(format!(
                    "Invalid header name '{name}': {parse_err}"
                ))
                .with_span(entry_span(entry))
            })?;
            if needs_value
                && entry
                    .args
                    .get(1)
                    .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
                    .is_none()
            {
                return Err(ConfigurationValidationError::from(
                    "request_header requires a value for add/replace operations",
                )
                .with_span(entry_span(entry)));
            }
        }
    }
    Ok(())
}
