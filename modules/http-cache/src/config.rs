use cidr::IpCidr;
use http::header::HeaderName;

use ferron_core::config::layer::LayeredConfiguration;
use ferron_core::config::ServerConfigurationBlock;

pub const DEFAULT_MAX_CACHE_ENTRIES: usize = 1024;
pub const DEFAULT_MAX_CACHE_RESPONSE_SIZE: usize = 2 * 1024 * 1024;
pub const DEFAULT_MAX_CACHE_AGE_SECS: u64 = 300;

#[derive(Clone)]
pub struct CacheConfig {
    pub enabled: bool,
    pub max_response_size: usize,
    pub litespeed_override_cache_control: bool,
    pub emit_litespeed_headers: bool,
    pub vary_headers: Vec<HeaderName>,
    pub ignored_store_headers: Vec<HeaderName>,
    pub purge_method: bool,
    pub purge_allowed_ips: Vec<IpCidr>,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_response_size: DEFAULT_MAX_CACHE_RESPONSE_SIZE,
            litespeed_override_cache_control: false,
            emit_litespeed_headers: false,
            vary_headers: Vec::new(),
            ignored_store_headers: Vec::new(),
            purge_method: false,
            purge_allowed_ips: Vec::new(),
        }
    }
}

pub fn parse_cache_config(configuration: &LayeredConfiguration) -> CacheConfig {
    let enabled = parse_cache_enabled(configuration);
    let max_response_size = get_nested_non_negative_usize(
        configuration,
        "max_response_size",
        DEFAULT_MAX_CACHE_RESPONSE_SIZE,
    );
    let litespeed_override_cache_control =
        get_nested_bool(configuration, "litespeed_override_cache_control", false);
    let emit_litespeed_headers = get_nested_bool(configuration, "emit_litespeed_headers", false);

    let vary_headers = collect_header_names(configuration, "vary");
    let ignored_store_headers = collect_header_names(configuration, "ignore");
    let purge_method = get_nested_bool(configuration, "purge_method", false);
    let purge_allowed_ips = collect_purge_allowed_ips(configuration);

    CacheConfig {
        enabled,
        max_response_size,
        litespeed_override_cache_control,
        emit_litespeed_headers,
        vary_headers,
        ignored_store_headers,
        purge_method,
        purge_allowed_ips,
    }
}

pub fn parse_max_entries(configuration: &LayeredConfiguration) -> usize {
    get_nested_non_negative_usize(configuration, "max_entries", DEFAULT_MAX_CACHE_ENTRIES)
}

fn parse_cache_enabled(configuration: &LayeredConfiguration) -> bool {
    for entry in configuration.get_entries("cache", true) {
        if let Some(value) = entry.args.first().and_then(|value| value.as_boolean()) {
            return value;
        }

        if let Some(children) = &entry.children {
            if !children.directives.keys().all(|name| name == "max_entries") {
                return true;
            }
        } else {
            return true;
        }
    }

    false
}

fn get_nested_non_negative_usize(
    configuration: &LayeredConfiguration,
    directive: &str,
    default: usize,
) -> usize {
    find_nested_value(configuration, directive)
        .and_then(|value| value.as_number())
        .map(|value| value.max(0) as usize)
        .unwrap_or(default)
}

fn get_nested_bool(configuration: &LayeredConfiguration, directive: &str, default: bool) -> bool {
    cache_blocks(configuration)
        .into_iter()
        .find_map(|block| {
            block.directives.get(directive).and_then(|entries| {
                entries.first().map(|entry| {
                    entry
                        .args
                        .first()
                        .and_then(|value| value.as_boolean())
                        .unwrap_or(true)
                })
            })
        })
        .unwrap_or(default)
}

fn find_nested_value<'a>(
    configuration: &'a LayeredConfiguration,
    directive: &str,
) -> Option<&'a ferron_core::config::ServerConfigurationValue> {
    cache_blocks(configuration)
        .into_iter()
        .find_map(|block| block.get_value(directive))
}

fn collect_header_names(configuration: &LayeredConfiguration, directive: &str) -> Vec<HeaderName> {
    let mut names = Vec::new();
    for block in cache_blocks(configuration) {
        if let Some(entries) = block.directives.get(directive) {
            for entry in entries {
                for arg in &entry.args {
                    if let Some(value) = arg.as_str() {
                        if let Ok(name) = HeaderName::from_bytes(value.trim().as_bytes()) {
                            if !names.contains(&name) {
                                names.push(name);
                            }
                        }
                    }
                }
            }
        }
    }
    names
}

fn collect_purge_allowed_ips(configuration: &LayeredConfiguration) -> Vec<IpCidr> {
    let mut ips = Vec::new();
    for block in cache_blocks(configuration) {
        if let Some(entries) = block.directives.get("purge_allowed_ips") {
            for entry in entries {
                for arg in &entry.args {
                    if let Some(value) = arg.as_str() {
                        if let Ok(cidr) = value.parse::<IpCidr>() {
                            ips.push(cidr);
                        }
                    }
                }
            }
        }
    }
    ips
}

fn cache_blocks(configuration: &LayeredConfiguration) -> Vec<&ServerConfigurationBlock> {
    configuration
        .get_entries("cache", true)
        .into_iter()
        .filter_map(|entry| entry.children.as_ref())
        .collect()
}
