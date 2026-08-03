use std::error::Error;
use std::str::FromStr;

use ferron_core::config::ServerConfigurationDirectiveEntry;
use http::header::HeaderName;

use super::types::{HeaderAction, ProxyConfig};

#[inline]
pub(super) fn parse_request_header_entry(
    entry: &ServerConfigurationDirectiveEntry,
    cfg: &mut ProxyConfig,
    ctx: &ferron_http::HttpContext,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    if entry.args.is_empty() {
        return Err("request_header requires at least one argument".into());
    }

    let first_arg = entry.args[0]
        .as_str()
        .ok_or("request_header name must be a string")?;

    match first_arg.chars().next() {
        Some('+') => {
            let name = &first_arg[1..];
            let value = entry
                .args
                .get(1)
                .and_then(|v| v.as_string_with_interpolations(ctx))
                .ok_or("request_header +Name requires a value")?;
            let header_name = HeaderName::from_str(name)
                .map_err(|e| format!("Invalid header name '{name}': {e}"))?;
            cfg.headers_to_add
                .push(HeaderAction::Append(header_name, value));
        }
        Some('-') => {
            let name = &first_arg[1..];
            let header_name = HeaderName::from_str(name)
                .map_err(|e| format!("Invalid header name '{name}': {e}"))?;
            cfg.headers_to_remove.push(header_name);
        }
        _ => {
            let name = first_arg;
            let value = entry
                .args
                .get(1)
                .and_then(|v| v.as_string_with_interpolations(ctx))
                .ok_or("request_header Name requires a value")?;
            let header_name = HeaderName::from_str(name)
                .map_err(|e| format!("Invalid header name '{name}': {e}"))?;
            cfg.headers_to_replace.push((header_name, value));
        }
    }

    Ok(())
}

#[inline]
pub(super) fn parse_affinity_entry(
    type_val: &str,
    entry: &ServerConfigurationDirectiveEntry,
    _ctx: &ferron_http::HttpContext,
) -> Result<crate::types::affinity::AffinityConfig, Box<dyn Error + Send + Sync>> {
    use crate::types::affinity::{
        AffinityConfig, AffinityType, CookieAffinityConfig, SameSiteMode,
    };

    let affinity_type = match type_val {
        "cookie" => {
            let mut cookie_cfg = CookieAffinityConfig::default();
            if let Some(block) = &entry.children {
                for (name, entries) in block.directives.iter() {
                    match name.as_str() {
                        "name" => {
                            if let Some(val) = entries
                                .first()
                                .and_then(|e| e.args.first())
                                .and_then(|v| v.as_str())
                            {
                                cookie_cfg.name = val.to_string();
                            }
                        }
                        "ttl" => {
                            if let Some(val) = entries
                                .first()
                                .and_then(|e| e.args.first())
                                .and_then(|v| v.as_duration())
                            {
                                cookie_cfg.ttl = Some(val);
                            }
                        }
                        "path" => {
                            if let Some(val) = entries
                                .first()
                                .and_then(|e| e.args.first())
                                .and_then(|v| v.as_str())
                            {
                                cookie_cfg.path = val.to_string();
                            }
                        }
                        "domain" => {
                            if let Some(val) = entries
                                .first()
                                .and_then(|e| e.args.first())
                                .and_then(|v| v.as_str())
                            {
                                cookie_cfg.domain = Some(val.to_string());
                            }
                        }
                        "secure" => {
                            cookie_cfg.secure =
                                entries.first().map(|e| e.get_flag()).unwrap_or(true);
                        }
                        "httponly" => {
                            cookie_cfg.httponly =
                                entries.first().map(|e| e.get_flag()).unwrap_or(true);
                        }
                        "samesite" => {
                            if let Some(val) = entries
                                .first()
                                .and_then(|e| e.args.first())
                                .and_then(|v| v.as_str())
                            {
                                cookie_cfg.samesite = match val.to_lowercase().as_str() {
                                    "strict" => SameSiteMode::Strict,
                                    "lax" => SameSiteMode::Lax,
                                    "none" => SameSiteMode::None,
                                    _ => {
                                        return Err(format!(
                                            "Invalid samesite mode: {val}, must be strict, lax, or none"
                                        )
                                        .into())
                                    }
                                };
                            }
                        }
                        _ => {}
                    }
                }
            }
            AffinityType::Cookie(cookie_cfg)
        }
        "header" => {
            let header_name = entry
                .children
                .as_ref()
                .and_then(|block| block.directives.get("name"))
                .and_then(|entries| entries.first())
                .and_then(|e| e.args.first())
                .and_then(|v| v.as_str())
                .ok_or("header affinity requires a 'name' subdirective")?;
            let header_name = HeaderName::from_str(header_name)
                .map_err(|e| format!("Invalid header name '{header_name}': {e}"))?;
            AffinityType::Header(header_name)
        }
        "ip" => AffinityType::Ip,
        "hash" => {
            let mut variable: Option<String> = None;
            if let Some(block) = &entry.children {
                for (name, entries) in block.directives.iter() {
                    if name.as_str() == "variable" {
                        if let Some(val) = entries
                            .first()
                            .and_then(|e| e.args.first())
                            .and_then(|v| v.as_str())
                        {
                            variable = Some(val.to_string());
                        }
                    }
                }
            }
            let variable = variable.ok_or("hash affinity requires a 'variable' subdirective")?;
            AffinityType::Hash { variable }
        }
        _ => {
            return Err(format!(
                "Invalid affinity type: {type_val}, must be cookie, header, ip, or hash"
            )
            .into())
        }
    };

    Ok(AffinityConfig { affinity_type })
}
