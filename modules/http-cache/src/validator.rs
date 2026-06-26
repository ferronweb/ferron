use cidr::IpCidr;
use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::config::{
    ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationSpan,
    ServerConfigurationValue,
};
use http::header::HeaderName;

const GLOBAL_CACHE_DIRECTIVES: &[&str] = &["max_entries", "zone"];
const HOST_CACHE_DIRECTIVES: &[&str] = &[
    "max_response_size",
    "litespeed_override_cache_control",
    "emit_litespeed_headers",
    "purge_method",
    "purge_allowed_ips",
    "purge_propagation",
    "vary",
    "ignore",
    "ignore_request_cache_control",
    "zone",
    "max_entries",
];

#[derive(Default)]
pub struct HttpCacheConfigurationValidator;

impl ConfigurationValidator for HttpCacheConfigurationValidator {
    #[inline]
    fn validate_block(
        &self,
        config: &ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let is_global = ctx.is_global;
        let used_directives = &mut ctx.used_directives;
        if is_global {
            if let Some(entries) = config.directives.get("cache") {
                used_directives.insert("cache".to_string());
                for entry in entries {
                    if !entry.args.is_empty() {
                        return Err(
                            "Invalid `cache` - global cache configuration only supports block form"
                                .into(),
                        );
                    }

                    let Some(children) = &entry.children else {
                        return Err(
                            "Invalid `cache` - global cache configuration requires a block".into(),
                        );
                    };

                    validate_cache_block(
                        children,
                        ctx,
                        &[HOST_CACHE_DIRECTIVES, GLOBAL_CACHE_DIRECTIVES].concat(),
                        false,
                    )?;
                    if !children.directives.contains_key("max_entries") {
                        return Err(
                            "Invalid `cache` - global cache configuration requires `max_entries`"
                                .into(),
                        );
                    }

                    if let Some(nested_entries) = children.directives.get("max_entries") {
                        for nested_entry in nested_entries {
                            validate_single_non_negative_integer(nested_entry, "max_entries")?;
                        }
                    }

                    // Validate zone blocks at global scope
                    if let Some(zone_entries) = children.directives.get("zone") {
                        for zone_entry in zone_entries {
                            validate_global_zone_block(zone_entry, ctx)?;
                        }
                    }
                }
            }
            return Ok(());
        }

        if let Some(entries) = config.directives.get("cache") {
            used_directives.insert("cache".to_string());
            for entry in entries {
                if let Some(children) = &entry.children {
                    if !entry.args.is_empty() {
                        return Err(
                            "Invalid `cache` - block form does not accept boolean arguments".into(),
                        );
                    }

                    validate_cache_block(
                        children,
                        ctx,
                        HOST_CACHE_DIRECTIVES,
                        config.directives.contains_key("basic_auth"),
                    )?;
                } else {
                    if entry.args.len() > 1 {
                        return Err(
                            "Invalid `cache` - expected at most one boolean argument".into()
                        );
                    }
                    if let Some(value) = entry.args.first() {
                        if value.as_boolean().is_none() {
                            return Err("Invalid `cache` - expected a boolean value".into());
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

fn validate_cache_block(
    block: &ServerConfigurationBlock,
    ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    allowed_directives: &[&str],
    parent_has_basic_auth: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut sub = std::collections::HashSet::new();

    for allowed in allowed_directives {
        if let Some(entries) = block.directives.get(*allowed) {
            sub.insert(allowed.to_string());
            if *allowed == "purge_propagation" {
                continue;
            }
            for entry in entries {
                if entry.children.is_some() {
                    return Err(
                        format!("Invalid `{allowed}` - nested blocks are not supported").into(),
                    );
                }
            }
        }
    }

    ferron_core::check_unused_subdirectives!(block, sub, &mut ctx.diagnostics, ctx.scope.clone());

    if let Some(entries) = block.directives.get("max_entries") {
        for entry in entries {
            validate_single_non_negative_integer(entry, "max_entries")?;
        }
    }

    if let Some(entries) = block.directives.get("max_response_size") {
        for entry in entries {
            validate_single_non_negative_integer(entry, "max_response_size")?;
        }
    }

    if let Some(entries) = block.directives.get("litespeed_override_cache_control") {
        for entry in entries {
            validate_boolean_entry(entry, "litespeed_override_cache_control")?;
            if entry.get_flag() {
                ctx.add_best_practice_violation(
                    "`litespeed_override_cache_control` makes LiteSpeed cache headers override standard HTTP cache policy; enable it only for applications that require LiteSpeed-compatible semantics",
                    entry_span(entry),
                );
            }
        }
    }

    if let Some(entries) = block.directives.get("ignore_request_cache_control") {
        for entry in entries {
            validate_boolean_entry(entry, "ignore_request_cache_control")?;
            if entry.get_flag() {
                ctx.add_best_practice_violation(
                    "`ignore_request_cache_control` is enabled - cache policy will be ignored based on request headers",
                    entry_span(entry),
                );
            }
        }
    }

    if let Some(entries) = block.directives.get("emit_litespeed_headers") {
        for entry in entries {
            validate_boolean_entry(entry, "emit_litespeed_headers")?;
        }
    }

    if let Some(entries) = block.directives.get("purge_method") {
        for entry in entries {
            validate_boolean_entry(entry, "purge_method")?;
            if entry.get_flag()
                && !parent_has_basic_auth
                && !block.directives.contains_key("purge_allowed_ips")
            {
                ctx.add_best_practice_violation(
                    "`purge_method` is enabled without `purge_allowed_ips` or a `basic_auth` block in the same scope; add an explicit purge access control before relying on cache purging",
                    entry_span(entry),
                );
            }
        }
    }

    if let Some(entries) = block.directives.get("purge_allowed_ips") {
        validate_cidr_list(entries, "purge_allowed_ips")?;
        for entry in entries {
            for arg in &entry.args {
                if arg
                    .as_str()
                    .is_some_and(|value| value == "0.0.0.0/0" || value == "::/0")
                {
                    ctx.add_best_practice_violation(
                        "`purge_allowed_ips` allows every source address; restrict cache purging to trusted operators or internal networks",
                        entry_span(entry),
                    );
                }
            }
        }
    }

    if let Some(entries) = block.directives.get("vary") {
        validate_header_name_list(entries, "vary")?;
    }

    if let Some(entries) = block.directives.get("ignore") {
        validate_header_name_list(entries, "ignore")?;
    }

    if let Some(entries) = block.directives.get("purge_propagation") {
        for entry in entries {
            if let Some(children) = &entry.children {
                validate_purge_propagation_block(children, ctx)?;
            } else {
                return Err(
                    "Invalid `purge_propagation` - expected a block with nested directives".into(),
                );
            }
        }
    }

    if let Some(entries) = block.directives.get("zone") {
        for entry in entries {
            if entry.children.is_some() {
                return Err(
                    "Invalid `zone` - expected a string argument, not a block".into(),
                );
            }
            if entry.args.len() != 1 {
                return Err(
                    "Invalid `zone` - expected exactly one string argument (the zone name)".into(),
                );
            }
            if entry.args.first().and_then(|v| v.as_str()).is_none() {
                return Err("Invalid `zone` - expected a string value".into());
            }
        }
    }

    Ok(())
}

const PURGE_PROPAGATION_DIRECTIVES: &[&str] = &["control_plane_url", "shared_secret", "node_id"];

fn validate_purge_propagation_block(
    block: &ServerConfigurationBlock,
    ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut sub = std::collections::HashSet::new();

    for allowed in PURGE_PROPAGATION_DIRECTIVES {
        if let Some(entries) = block.directives.get(*allowed) {
            sub.insert(allowed.to_string());
            for entry in entries {
                if entry.children.is_some() {
                    return Err(
                        format!("Invalid `{allowed}` - nested blocks are not supported").into(),
                    );
                }
                if entry.args.len() != 1 {
                    return Err(format!(
                        "Invalid `{allowed}` - expected exactly one string argument"
                    )
                    .into());
                }
                if entry.args.first().and_then(|v| v.as_str()).is_none() {
                    return Err(format!("Invalid `{allowed}` - expected a string value").into());
                }
            }
        }
    }

    ferron_core::check_unused_subdirectives!(block, sub, &mut ctx.diagnostics, ctx.scope.clone());

    if let Some(entries) = block.directives.get("control_plane_url") {
        for entry in entries {
            if let Some(url) = entry.args.first().and_then(|v| v.as_str()) {
                if !url.starts_with("http://") && !url.starts_with("https://") {
                    ctx.add_best_practice_violation(
                        "`control_plane_url` should use HTTPS in production environments",
                        entry_span(entry),
                    );
                }
            }
        }
    }

    if block.directives.contains_key("control_plane_url")
        && !block.directives.contains_key("shared_secret")
    {
        ctx.add_best_practice_violation(
            "`purge_propagation` has `control_plane_url` but no `shared_secret`; add a shared secret to authenticate purge webhooks",
            None,
        );
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

fn validate_boolean_entry(
    entry: &ServerConfigurationDirectiveEntry,
    name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if entry.args.len() > 1 {
        return Err(format!("Invalid `{name}` - expected at most one boolean argument").into());
    }
    if let Some(value) = entry.args.first() {
        if value.as_boolean().is_none() {
            return Err(format!("Invalid `{name}` - expected a boolean value").into());
        }
    }
    Ok(())
}

fn validate_single_non_negative_integer(
    entry: &ServerConfigurationDirectiveEntry,
    name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if entry.args.len() != 1 {
        return Err(format!("Invalid `{name}` - expected exactly one integer argument").into());
    }
    let value = entry
        .args
        .first()
        .and_then(ServerConfigurationValue::as_number)
        .ok_or_else(|| format!("Invalid `{name}` - expected an integer value"))?;
    if value < 0 {
        return Err(format!("Invalid `{name}` - expected a non-negative integer").into());
    }
    Ok(())
}

fn validate_cidr_list(
    entries: &[ServerConfigurationDirectiveEntry],
    name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    for entry in entries {
        if entry.args.is_empty() {
            return Err(format!("Invalid `{name}` - expected at least one IP or CIDR").into());
        }
        for arg in &entry.args {
            let value = arg
                .as_str()
                .ok_or_else(|| format!("Invalid `{name}` - expected string IP/CIDR values"))?;
            value
                .parse::<IpCidr>()
                .map_err(|_| format!("Invalid `{name}` - invalid IP or CIDR `{value}`"))?;
        }
    }
    Ok(())
}

fn validate_header_name_list(
    entries: &[ServerConfigurationDirectiveEntry],
    name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    for entry in entries {
        if entry.args.is_empty() {
            return Err(format!("Invalid `{name}` - expected at least one header name").into());
        }
        for arg in &entry.args {
            let value = arg
                .as_str()
                .ok_or_else(|| format!("Invalid `{name}` - expected string header names"))?;
            HeaderName::from_bytes(value.trim().as_bytes())
                .map_err(|_| format!("Invalid `{name}` - invalid header name `{value}`"))?;
        }
    }
    Ok(())
}

const GLOBAL_ZONE_DIRECTIVES: &[&str] = &["max_entries"];

fn validate_global_zone_block(
    entry: &ServerConfigurationDirectiveEntry,
    ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
) -> Result<(), Box<dyn std::error::Error>> {
    // zone must have exactly one string argument (the zone name)
    if entry.args.len() != 1 {
        return Err(
            "Invalid `zone` - expected exactly one string argument (the zone name)".into(),
        );
    }
    if entry.args.first().and_then(|v| v.as_str()).is_none() {
        return Err("Invalid `zone` - expected a string value".into());
    }

    // zone must be a block
    let Some(children) = &entry.children else {
        return Err("Invalid `zone` - expected a block with nested directives".into());
    };

    // Validate allowed subdirectives inside the zone block
    let mut sub = std::collections::HashSet::new();
    for allowed in GLOBAL_ZONE_DIRECTIVES {
        if let Some(entries) = children.directives.get(*allowed) {
            sub.insert(allowed.to_string());
            for entry in entries {
                if entry.children.is_some() {
                    return Err(format!(
                        "Invalid `{allowed}` - nested blocks are not supported"
                    )
                    .into());
                }
            }
        }
    }

    ferron_core::check_unused_subdirectives!(
        children,
        sub,
        &mut ctx.diagnostics,
        ctx.scope.clone()
    );

    // Validate max_entries if present
    if let Some(entries) = children.directives.get("max_entries") {
        for entry in entries {
            validate_single_non_negative_integer(entry, "max_entries")?;
        }
    }

    Ok(())
}
