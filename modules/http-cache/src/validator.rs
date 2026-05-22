use std::collections::HashSet;

use cidr::IpCidr;
use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::config::{
    ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationValue,
};
use http::header::HeaderName;

const GLOBAL_CACHE_DIRECTIVES: &[&str] = &["max_entries"];
const HOST_CACHE_DIRECTIVES: &[&str] = &[
    "max_response_size",
    "litespeed_override_cache_control",
    "emit_litespeed_headers",
    "purge_method",
    "purge_allowed_ips",
    "vary",
    "ignore",
];

#[derive(Default)]
pub struct HttpCacheGlobalConfigurationValidator;

#[derive(Default)]
pub struct HttpCacheConfigurationValidator;

impl ConfigurationValidator for HttpCacheGlobalConfigurationValidator {
    fn validate_block(
        &self,
        config: &ServerConfigurationBlock,
        used_directives: &mut HashSet<String>,
        _is_global: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
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
                    &[HOST_CACHE_DIRECTIVES, GLOBAL_CACHE_DIRECTIVES].concat(),
                    "global `cache`",
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
            }
        }

        Ok(())
    }
}

impl ConfigurationValidator for HttpCacheConfigurationValidator {
    fn validate_block(
        &self,
        config: &ServerConfigurationBlock,
        used_directives: &mut HashSet<String>,
        _is_global: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(entries) = config.directives.get("cache") {
            used_directives.insert("cache".to_string());
            for entry in entries {
                if let Some(children) = &entry.children {
                    if !entry.args.is_empty() {
                        return Err(
                            "Invalid `cache` - block form does not accept boolean arguments".into(),
                        );
                    }

                    validate_cache_block(children, HOST_CACHE_DIRECTIVES, "`cache`")?;
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
    allowed_directives: &[&str],
    context: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    for directive_name in block.directives.keys() {
        if !allowed_directives.contains(&directive_name.as_str()) {
            return Err(format!(
                "Invalid `{directive_name}` - unknown directive in {context} block"
            )
            .into());
        }
    }

    for allowed in allowed_directives {
        if let Some(entries) = block.directives.get(*allowed) {
            for entry in entries {
                if entry.children.is_some() {
                    return Err(
                        format!("Invalid `{allowed}` - nested blocks are not supported").into(),
                    );
                }
            }
        }
    }

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
        }
    }

    if let Some(entries) = block.directives.get("purge_allowed_ips") {
        validate_cidr_list(entries, "purge_allowed_ips")?;
    }

    if let Some(entries) = block.directives.get("vary") {
        validate_header_name_list(entries, "vary")?;
    }

    if let Some(entries) = block.directives.get("ignore") {
        validate_header_name_list(entries, "ignore")?;
    }

    Ok(())
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
