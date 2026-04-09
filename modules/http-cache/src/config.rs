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
    pub vary_headers: Vec<HeaderName>,
    pub ignored_store_headers: Vec<HeaderName>,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_response_size: DEFAULT_MAX_CACHE_RESPONSE_SIZE,
            litespeed_override_cache_control: false,
            vary_headers: Vec::new(),
            ignored_store_headers: Vec::new(),
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

    let vary_headers = collect_header_names(configuration, "vary");
    let ignored_store_headers = collect_header_names(configuration, "ignore");

    CacheConfig {
        enabled,
        max_response_size,
        litespeed_override_cache_control,
        vary_headers,
        ignored_store_headers,
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

fn cache_blocks(configuration: &LayeredConfiguration) -> Vec<&ServerConfigurationBlock> {
    configuration
        .get_entries("cache", true)
        .into_iter()
        .filter_map(|entry| entry.children.as_ref())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferron_core::config::{
        ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationValue,
    };
    use std::collections::HashMap;
    use std::sync::Arc;

    fn value_bool(value: bool) -> ServerConfigurationValue {
        ServerConfigurationValue::Boolean(value, None)
    }

    fn value_number(value: i64) -> ServerConfigurationValue {
        ServerConfigurationValue::Number(value, None)
    }

    fn value_string(value: &str) -> ServerConfigurationValue {
        ServerConfigurationValue::String(value.to_string(), None)
    }

    fn make_directive_entry(
        args: Vec<ServerConfigurationValue>,
        children: Option<ServerConfigurationBlock>,
    ) -> ServerConfigurationDirectiveEntry {
        ServerConfigurationDirectiveEntry {
            args,
            children,
            span: None,
        }
    }

    fn make_block(
        directives: Vec<(
            &str,
            Vec<(
                Vec<ServerConfigurationValue>,
                Option<ServerConfigurationBlock>,
            )>,
        )>,
    ) -> ServerConfigurationBlock {
        let mut map = HashMap::new();
        for (name, entries) in directives {
            map.insert(
                name.to_string(),
                entries
                    .into_iter()
                    .map(|(args, children)| make_directive_entry(args, children))
                    .collect(),
            );
        }

        ServerConfigurationBlock {
            directives: Arc::new(map),
            matchers: HashMap::new(),
            span: None,
        }
    }

    fn make_layered_config(layers: Vec<ServerConfigurationBlock>) -> LayeredConfiguration {
        LayeredConfiguration {
            layers: layers.into_iter().map(Arc::new).collect(),
        }
    }

    fn cache_block(
        directives: Vec<(
            &str,
            Vec<(
                Vec<ServerConfigurationValue>,
                Option<ServerConfigurationBlock>,
            )>,
        )>,
    ) -> ServerConfigurationBlock {
        make_block(directives)
    }

    #[test]
    fn parses_host_cache_block() {
        let config = make_layered_config(vec![make_block(vec![(
            "cache",
            vec![(
                vec![],
                Some(cache_block(vec![
                    ("max_response_size", vec![(vec![value_number(4096)], None)]),
                    ("vary", vec![(vec![value_string("Accept-Encoding")], None)]),
                    ("ignore", vec![(vec![value_string("Set-Cookie")], None)]),
                ])),
            )],
        )])]);

        let parsed = parse_cache_config(&config);
        assert!(parsed.enabled);
        assert_eq!(parsed.max_response_size, 4096);
        assert!(!parsed.litespeed_override_cache_control);
        assert_eq!(parsed.vary_headers.len(), 1);
        assert_eq!(parsed.ignored_store_headers.len(), 1);
    }

    #[test]
    fn boolean_false_disables_inherited_cache() {
        let config = make_layered_config(vec![
            make_block(vec![(
                "cache",
                vec![(
                    vec![],
                    Some(cache_block(vec![(
                        "max_response_size",
                        vec![(vec![value_number(4096)], None)],
                    )])),
                )],
            )]),
            make_block(vec![("cache", vec![(vec![value_bool(false)], None)])]),
        ]);

        let parsed = parse_cache_config(&config);
        assert!(!parsed.enabled);
        assert_eq!(parsed.max_response_size, 4096);
    }

    #[test]
    fn global_max_entries_block_does_not_enable_cache() {
        let config = make_layered_config(vec![make_block(vec![(
            "cache",
            vec![(
                vec![],
                Some(cache_block(vec![(
                    "max_entries",
                    vec![(vec![value_number(2048)], None)],
                )])),
            )],
        )])]);

        let parsed = parse_cache_config(&config);
        assert!(!parsed.enabled);
        assert_eq!(parse_max_entries(&config), 2048);
    }

    #[test]
    fn merges_repeatable_nested_headers() {
        let config = make_layered_config(vec![
            make_block(vec![(
                "cache",
                vec![(
                    vec![],
                    Some(cache_block(vec![
                        (
                            "vary",
                            vec![
                                (vec![value_string("Accept-Encoding")], None),
                                (vec![value_string("Accept-Language")], None),
                            ],
                        ),
                        ("ignore", vec![(vec![value_string("Set-Cookie")], None)]),
                    ])),
                )],
            )]),
            make_block(vec![(
                "cache",
                vec![(
                    vec![],
                    Some(cache_block(vec![
                        ("vary", vec![(vec![value_string("X-Device")], None)]),
                        ("ignore", vec![(vec![value_string("Age")], None)]),
                    ])),
                )],
            )]),
        ]);

        let parsed = parse_cache_config(&config);
        assert_eq!(parsed.vary_headers.len(), 3);
        assert_eq!(parsed.ignored_store_headers.len(), 2);
    }

    #[test]
    fn parses_litespeed_override_flag() {
        let config = make_layered_config(vec![make_block(vec![(
            "cache",
            vec![(
                vec![],
                Some(cache_block(vec![(
                    "litespeed_override_cache_control",
                    vec![(vec![], None)],
                )])),
            )],
        )])]);

        let parsed = parse_cache_config(&config);
        assert!(parsed.enabled);
        assert!(parsed.litespeed_override_cache_control);
    }
}
