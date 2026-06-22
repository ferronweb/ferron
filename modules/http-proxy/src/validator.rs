use std::collections::HashMap;
use std::error::Error;
use std::str::FromStr;

use ferron_core::config::validator::{ConfigurationValidator, ConfigurationValidatorContext};
use ferron_core::config::{
    ServerConfigurationBlock, ServerConfigurationDirectiveEntry,
    ServerConfigurationInterpolatedStringPart, ServerConfigurationSpan, ServerConfigurationValue,
};
use ferron_core::util::parse_duration;
use http::header::HeaderName;

/// Configuration validator for the reverse proxy module.
pub struct ProxyConfigurationValidator;

impl ConfigurationValidator for ProxyConfigurationValidator {
    fn validate_block(
        &self,
        config: &ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let is_global = ctx.is_global;
        let used_directives = &mut ctx.used_directives;
        if is_global {
            // Validate global concurrent_conns directive
            if let Some(entries) = config.directives.get("concurrent_conns") {
                used_directives.insert("concurrent_conns".to_string());
                for e in entries {
                    if let Some(val) = e.args.first().and_then(|v| v.as_number()) {
                        if val < 0 {
                            return Err("Invalid `concurrent_conns` — must be non-negative".into());
                        }
                    } else {
                        return Err("Invalid `concurrent_conns` — expected a number".into());
                    }
                }
            }
        }
        if let Some(entries) = config.directives.get("proxy") {
            used_directives.insert("proxy".to_string());
            validate_proxy_entries(entries, ctx)?;
        }
        Ok(())
    }
}

fn validate_proxy_entries(
    entries: &[ServerConfigurationDirectiveEntry],
    ctx: &mut ConfigurationValidatorContext,
) -> Result<(), Box<dyn Error>> {
    for entry in entries {
        if entry.args.len() > 1 {
            return Err(
                "The `proxy` directive may have at most one shorthand upstream argument".into(),
            );
        }
        for arg in &entry.args {
            if arg.as_string_with_interpolations(&HashMap::new()).is_none() {
                return Err("Invalid proxy upstream URL — expected a string".into());
            }
            warn_user_controlled_upstream(arg, entry, ctx);
        }
        if let Some(block) = &entry.children {
            validate_proxy_block(block, ctx)?;
        }
    }
    Ok(())
}

fn validate_proxy_block(
    block: &ServerConfigurationBlock,
    ctx: &mut ConfigurationValidatorContext,
) -> Result<(), Box<dyn Error>> {
    let mut sub = std::collections::HashSet::new();

    ferron_core::validate_nested!(block, used(sub), algorithm, args(1) => [ServerConfigurationValue::String(_, _)]);
    validate_circuit_breaker_directives(block, ctx, &mut sub)?;
    ferron_core::validate_nested!(block, used(sub), retry_connection, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);
    ferron_core::validate_nested!(block, used(sub), keepalive, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);
    ferron_core::validate_nested!(block, used(sub), http2, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);
    ferron_core::validate_nested!(block, used(sub), http2_only, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);
    ferron_core::validate_nested!(block, used(sub), intercept_errors, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);
    ferron_core::validate_nested!(block, used(sub), no_verification, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);
    if block_flag(block, "no_verification") == Some(true) {
        ctx.add_best_practice_violation(
            "`proxy.no_verification` disables TLS certificate verification for HTTPS upstreams; use it only for testing or tightly controlled internal networks",
            first_entry_span(block, "no_verification"),
        );
    }
    ferron_core::validate_nested!(block, used(sub), proxy_header, optional args(1) => [ServerConfigurationValue::String(_, _)]);
    if block.directives.contains_key("request_header") {
        sub.insert("request_header".to_string());
    }
    validate_request_header(block)?;
    if block.directives.contains_key("proxy_concurrent_conns") {
        sub.insert("proxy_concurrent_conns".to_string());
    }
    validate_number(block, "proxy_concurrent_conns", 0)?;
    validate_upstream_directives(block, ctx, &mut sub)?;
    #[cfg(feature = "srv-lookup")]
    validate_srv_directives(block, ctx, &mut sub)?;

    ferron_core::check_unused_subdirectives!(block, sub, &mut ctx.diagnostics, ctx.scope.clone());
    Ok(())
}

fn validate_number(
    block: &ServerConfigurationBlock,
    name: &str,
    min: i64,
) -> Result<(), Box<dyn Error>> {
    if let Some(entries) = block.directives.get(name) {
        for e in entries {
            if let Some(val) = e.args.first().and_then(|v| v.as_number()) {
                if val < min {
                    return Err(format!("Invalid `{name}` — must be >= {min}").into());
                }
            } else {
                return Err(format!("Invalid `{name}` — expected a number").into());
            }
        }
    }
    Ok(())
}

fn validate_duration(block: &ServerConfigurationBlock, name: &str) -> Result<(), Box<dyn Error>> {
    if let Some(entries) = block.directives.get(name) {
        for e in entries {
            if let Some(val) = e.args.first().and_then(|v| v.as_str()) {
                parse_duration(val).map_err(|e| format!("Invalid `{name}` duration: {e}"))?;
            } else {
                return Err(format!("Invalid `{name}` — expected a duration string").into());
            }
        }
    }
    Ok(())
}

fn validate_request_header(block: &ServerConfigurationBlock) -> Result<(), Box<dyn Error>> {
    if let Some(entries) = block.directives.get("request_header") {
        for e in entries {
            if e.args.is_empty() {
                return Err("request_header requires at least one argument".into());
            }
            let first = e.args[0]
                .as_str()
                .ok_or("The header name must be a string")?;
            let (name, needs_value) = match first.chars().next() {
                Some('+') => (&first[1..], true),
                Some('-') => (&first[1..], false),
                _ => (first, true),
            };
            HeaderName::from_str(name).map_err(|e| format!("Invalid header name '{name}': {e}"))?;
            if needs_value
                && e.args
                    .get(1)
                    .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
                    .is_none()
            {
                return Err("request_header requires a value for add/replace operations".into());
            }
        }
    }
    Ok(())
}

fn validate_upstream_directives(
    block: &ServerConfigurationBlock,
    ctx: &mut ConfigurationValidatorContext,
    parent_used: &mut std::collections::HashSet<String>,
) -> Result<(), Box<dyn Error>> {
    if let Some(entries) = block.directives.get("upstream") {
        parent_used.insert("upstream".to_string());
        for e in entries {
            if e.args
                .first()
                .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
                .is_none()
            {
                return Err("The `upstream` directive requires a URL argument".into());
            }
            if let Some(value) = e.args.first() {
                warn_user_controlled_upstream(value, e, ctx);
            }
            if let Some(up_block) = &e.children {
                validate_upstream_block(up_block, ctx)?;
            }
        }
    }
    Ok(())
}

fn validate_upstream_block(
    block: &ServerConfigurationBlock,
    ctx: &mut ConfigurationValidatorContext,
) -> Result<(), Box<dyn Error>> {
    let mut sub = std::collections::HashSet::new();

    validate_active_check_directives(block, ctx, &mut sub)?;
    ferron_core::validate_nested!(block, used(sub), limit, args(1) => [ServerConfigurationValue::Number(_, _)]);
    if block.directives.contains_key("idle_timeout") {
        sub.insert("idle_timeout".to_string());
    }
    validate_duration(block, "idle_timeout")?;
    ferron_core::validate_nested!(block, used(sub), unix, args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)]);
    ferron_core::validate_nested!(block, used(sub), cert, args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)]);
    ferron_core::validate_nested!(block, used(sub), key, args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)]);
    #[cfg(not(unix))]
    if block.directives.contains_key("unix") {
        return Err("Unix sockets are not supported on this platform".into());
    }

    ferron_core::check_unused_subdirectives!(block, sub, &mut ctx.diagnostics, ctx.scope.clone());
    Ok(())
}

/// Validate SRV upstream directives.
#[cfg(feature = "srv-lookup")]
fn validate_srv_directives(
    block: &ServerConfigurationBlock,
    ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    parent_used: &mut std::collections::HashSet<String>,
) -> Result<(), Box<dyn Error>> {
    if let Some(entries) = block.directives.get("srv") {
        parent_used.insert("srv".to_string());
        for e in entries {
            if e.args.first().and_then(|v| v.as_str()).is_none() {
                return Err("The `srv` directive requires an SRV record name argument".into());
            }
            if let Some(srv_block) = &e.children {
                validate_srv_block(srv_block, ctx)?;
            }
        }
    }
    Ok(())
}

#[cfg(feature = "srv-lookup")]
fn validate_srv_block(
    block: &ServerConfigurationBlock,
    ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
) -> Result<(), Box<dyn Error>> {
    let mut sub = std::collections::HashSet::new();

    if block.directives.contains_key("limit") {
        sub.insert("limit".to_string());
    }
    validate_number(block, "limit", 1)?;
    if block.directives.contains_key("idle_timeout") {
        sub.insert("idle_timeout".to_string());
    }
    validate_duration(block, "idle_timeout")?;
    ferron_core::validate_nested!(block, used(sub), dns_servers, args(1) => [ServerConfigurationValue::String(_, _)]);
    ferron_core::validate_nested!(block, used(sub), cert, args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)]);
    ferron_core::validate_nested!(block, used(sub), key, args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::InterpolatedString(_, _)]);

    ferron_core::check_unused_subdirectives!(block, sub, &mut ctx.diagnostics, ctx.scope.clone());
    Ok(())
}

fn validate_active_check_directives(
    block: &ServerConfigurationBlock,
    ctx: &mut ConfigurationValidatorContext,
    parent_used: &mut std::collections::HashSet<String>,
) -> Result<(), Box<dyn Error>> {
    if let Some(ac_entry) = block.directives.get("active_check").and_then(|d| d.first()) {
        parent_used.insert("active_check".to_string());
        if let Some(active_block) = ac_entry.children.as_ref() {
            let mut sub = std::collections::HashSet::new();

            ferron_core::validate_nested!(active_block, used(sub), uri, args(1) => [ServerConfigurationValue::String(_, _)]);
            ferron_core::validate_nested!(active_block, used(sub), method, args(1) => [ServerConfigurationValue::String(_, _)]);
            if active_block.directives.contains_key("interval") {
                sub.insert("interval".to_string());
            }
            validate_duration(active_block, "interval")?;
            if active_block.directives.contains_key("timeout") {
                sub.insert("timeout".to_string());
            }
            validate_duration(active_block, "timeout")?;
            ferron_core::validate_nested!(active_block, used(sub), expect_status, args(1) => [ServerConfigurationValue::String(_, _)]);
            if active_block
                .directives
                .contains_key("response_time_threshold")
            {
                sub.insert("response_time_threshold".to_string());
            }
            validate_duration(active_block, "response_time_threshold")?;
            ferron_core::validate_nested!(active_block, used(sub), body_match, args(1) => [ServerConfigurationValue::String(_, _)]);
            if active_block.directives.contains_key("consecutive_fails") {
                sub.insert("consecutive_fails".to_string());
            }
            validate_number(active_block, "consecutive_fails", 1)?;
            if active_block.directives.contains_key("consecutive_passes") {
                sub.insert("consecutive_passes".to_string());
            }
            validate_number(active_block, "consecutive_passes", 1)?;
            ferron_core::validate_nested!(active_block, used(sub), no_verification, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);
            if block_flag(active_block, "no_verification") == Some(true) {
                ctx.add_best_practice_violation(
                "`active_check.no_verification` disables TLS certificate verification for health checks; keep verification enabled unless probes target a strictly internal endpoint",
                first_entry_span(active_block, "no_verification"),
            );
            }

            ferron_core::check_unused_subdirectives!(
                active_block,
                sub,
                &mut ctx.diagnostics,
                ctx.scope.clone()
            );
        }
    }

    Ok(())
}

fn validate_circuit_breaker_directives(
    block: &ServerConfigurationBlock,
    ctx: &mut ConfigurationValidatorContext,
    parent_used: &mut std::collections::HashSet<String>,
) -> Result<(), Box<dyn Error>> {
    if let Some(cb_entry) = block
        .directives
        .get("circuit_breaker")
        .and_then(|d| d.first())
    {
        parent_used.insert("circuit_breaker".to_string());
        if let Some(cb_block) = cb_entry.children.as_ref() {
            let mut sub = std::collections::HashSet::new();

            if cb_block.directives.contains_key("max_fails") {
                sub.insert("max_fails".to_string());
            }
            validate_number(cb_block, "max_fails", 1)?;
            if cb_block.directives.contains_key("window") {
                sub.insert("window".to_string());
            }
            validate_duration(cb_block, "window")?;
            if cb_block.directives.contains_key("open_duration") {
                sub.insert("open_duration".to_string());
            }
            validate_duration(cb_block, "open_duration")?;
            if cb_block.directives.contains_key("consecutive_passes") {
                sub.insert("consecutive_passes".to_string());
            }
            validate_number(cb_block, "consecutive_passes", 1)?;
            ferron_core::validate_nested!(cb_block, used(sub), record_5xx, optional args(1) => [ServerConfigurationValue::Boolean(_, _)] | args(0) => [ServerConfigurationValue::Boolean(_, _)]);

            ferron_core::check_unused_subdirectives!(
                cb_block,
                sub,
                &mut ctx.diagnostics,
                ctx.scope.clone()
            );
        }
    }

    Ok(())
}

fn entry_span(entry: &ServerConfigurationDirectiveEntry) -> Option<ServerConfigurationSpan> {
    entry.span.clone().or_else(|| {
        entry.args.first().and_then(|value| match value {
            ServerConfigurationValue::String(_, span)
            | ServerConfigurationValue::Number(_, span)
            | ServerConfigurationValue::Float(_, span)
            | ServerConfigurationValue::Boolean(_, span)
            | ServerConfigurationValue::InterpolatedString(_, span) => span.clone(),
        })
    })
}

fn first_entry_span(
    block: &ServerConfigurationBlock,
    directive: &str,
) -> Option<ServerConfigurationSpan> {
    block
        .directives
        .get(directive)
        .and_then(|entries| entries.first())
        .and_then(entry_span)
}

fn block_flag(block: &ServerConfigurationBlock, directive: &str) -> Option<bool> {
    block
        .directives
        .get(directive)
        .and_then(|entries| entries.first())
        .map(ServerConfigurationDirectiveEntry::get_flag)
}

fn value_uses_request_header_interpolation(value: &ServerConfigurationValue) -> bool {
    match value {
        ServerConfigurationValue::InterpolatedString(parts, _) => parts.iter().any(|part| {
            matches!(
                part,
                ServerConfigurationInterpolatedStringPart::Variable(variable)
                    if variable.starts_with("request.header.")
            )
        }),
        ServerConfigurationValue::String(value, _) => value.contains("{{request.header."),
        _ => false,
    }
}

fn warn_user_controlled_upstream(
    value: &ServerConfigurationValue,
    entry: &ServerConfigurationDirectiveEntry,
    ctx: &mut ConfigurationValidatorContext,
) {
    if value_uses_request_header_interpolation(value) {
        ctx.add_best_practice_violation(
            "Proxy upstream URLs interpolate user-controlled request headers; derive upstream targets from static configuration or trusted server-controlled variables to avoid SSRF",
            entry_span(entry),
        );
    }
}
