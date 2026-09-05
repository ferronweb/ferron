use std::path::PathBuf;
use std::time::Duration;

use cidr::IpCidr;
use http::header::HeaderName;

use ferron_core::config::layer::LayeredConfiguration;
use ferron_core::config::parse_duration;
use ferron_core::config::ServerConfigurationBlock;

pub const DEFAULT_MAX_CACHE_ENTRIES: usize = 1024;
pub const DEFAULT_MAX_CACHE_RESPONSE_SIZE: usize = 2 * 1024 * 1024;
pub const DEFAULT_COALESCE_TIMEOUT_SECS: u64 = 5;
pub const DEFAULT_PERSIST_INTERVAL: Duration = Duration::from_secs(30);
pub const MIN_PERSIST_INTERVAL: Duration = Duration::from_secs(1);

/// Identifies which cache store a request belongs to.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CacheZoneId {
    /// The global zone, used when a global `cache { max_entries = N }` block
    /// exists without explicit `zone` blocks. All hosts share this store.
    Global,
    /// A named zone defined at global scope via `zone "name" { ... }`.
    /// Multiple hostnames can reference the same named zone.
    Named(String),
    /// An implicit per-host zone keyed by hostname. Used when no global zone
    /// exists and the host does not specify an explicit `zone` directive.
    Host(String),
}

impl CacheZoneId {
    /// Return a stable string label for use in metric attributes.
    pub fn label(&self) -> &str {
        match self {
            CacheZoneId::Global => "global",
            CacheZoneId::Named(name) => name.as_str(),
            CacheZoneId::Host(host) => host.as_str(),
        }
    }
}

/// Configuration for multi-instance cache purge propagation via an external
/// control-plane service.
#[derive(Clone, Default)]
pub struct PurgePropagationConfig {
    /// URL of the external control-plane endpoint to POST purge events to.
    pub control_plane_url: Option<String>,
    /// Shared secret included as `X-Purge-Secret` header when pushing purge
    /// events to the control-plane.
    pub shared_secret: Option<String>,
    /// Identifier for this edge instance, sent as `origin` JSON field
    /// in outbound webhook requests. Defaults to the machine hostname
    /// if not set.
    pub node_id: Option<String>,
}

#[derive(Clone)]
pub struct CacheConfig {
    pub enabled: bool,
    pub max_response_size: usize,
    pub litespeed_override_cache_control: bool,
    pub ignore_request_cache_control: bool,
    pub emit_litespeed_headers: bool,
    pub enable_stale_while_revalidate: bool,
    pub enable_stale_if_error: bool,
    /// How long a singleflight follower waits for the leader before it stops
    /// coalescing and fetches from the upstream itself.
    pub coalesce_timeout: std::time::Duration,
    pub vary_headers: Vec<HeaderName>,
    pub vary_cookies: Vec<String>,
    pub ignored_store_headers: Vec<HeaderName>,
    pub purge_method: bool,
    pub purge_allowed_ips: Vec<IpCidr>,
    pub purge_propagation: PurgePropagationConfig,
    /// Cache zone this host belongs to. Determines which physical cache store
    /// is used. Resolved by `resolve_zone_id()` based on the host directive
    /// and global configuration.
    pub zone: Option<CacheZoneId>,
}

impl Default for CacheConfig {
    #[inline]
    fn default() -> Self {
        Self {
            enabled: false,
            max_response_size: DEFAULT_MAX_CACHE_RESPONSE_SIZE,
            litespeed_override_cache_control: false,
            ignore_request_cache_control: false,
            emit_litespeed_headers: false,
            enable_stale_while_revalidate: true,
            enable_stale_if_error: true,
            coalesce_timeout: std::time::Duration::from_secs(DEFAULT_COALESCE_TIMEOUT_SECS),
            vary_headers: Vec::new(),
            vary_cookies: Vec::new(),
            ignored_store_headers: Vec::new(),
            purge_method: false,
            purge_allowed_ips: Vec::new(),
            purge_propagation: PurgePropagationConfig::default(),
            zone: None,
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
    let ignore_request_cache_control =
        get_nested_bool(configuration, "ignore_request_cache_control", false);

    let enable_stale_while_revalidate =
        get_nested_bool(configuration, "enable_stale_while_revalidate", true);
    let enable_stale_if_error = get_nested_bool(configuration, "enable_stale_if_error", true);
    let coalesce_timeout = std::time::Duration::from_secs(get_nested_non_negative_usize(
        configuration,
        "coalesce_timeout",
        DEFAULT_COALESCE_TIMEOUT_SECS as usize,
    ) as u64);

    let vary_headers = collect_header_names(configuration, "vary");
    let vary_cookies = collect_string_values(configuration, "vary_cookies");
    let ignored_store_headers = collect_header_names(configuration, "ignore");
    let purge_method = get_nested_bool(configuration, "purge_method", false);
    let purge_allowed_ips = collect_purge_allowed_ips(configuration);
    let purge_propagation = parse_purge_propagation(configuration);
    let zone = parse_zone_name(configuration);

    CacheConfig {
        enabled,
        max_response_size,
        litespeed_override_cache_control,
        ignore_request_cache_control,
        emit_litespeed_headers,
        enable_stale_while_revalidate,
        enable_stale_if_error,
        coalesce_timeout,
        vary_headers,
        vary_cookies,
        ignored_store_headers,
        purge_method,
        purge_allowed_ips,
        purge_propagation,
        zone,
    }
}

#[inline]
pub fn parse_max_entries(configuration: &LayeredConfiguration) -> usize {
    get_nested_non_negative_usize(configuration, "max_entries", DEFAULT_MAX_CACHE_ENTRIES)
}

/// Check whether the host-level cache block explicitly specifies `max_entries`.
///
/// This is distinct from `parse_max_entries()` which reads from any layer
/// (including inherited global values). This function only checks the highest-
/// priority (host-level) cache block, using `inherit = false`.
#[inline]
pub fn has_host_max_entries(configuration: &LayeredConfiguration) -> bool {
    for entry in configuration.get_entries("cache", false) {
        if let Some(children) = &entry.children {
            if children.directives.contains_key("max_entries") {
                return true;
            }
        }
    }
    false
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

fn collect_string_values(configuration: &LayeredConfiguration, directive: &str) -> Vec<String> {
    let mut values = Vec::new();
    for block in cache_blocks(configuration) {
        if let Some(entries) = block.directives.get(directive) {
            for entry in entries {
                for arg in &entry.args {
                    if let Some(value) = arg.as_str() {
                        let trimmed = value.trim().to_string();
                        if !trimmed.is_empty() && !values.contains(&trimmed) {
                            values.push(trimmed);
                        }
                    }
                }
            }
        }
    }
    values
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

fn parse_purge_propagation(configuration: &LayeredConfiguration) -> PurgePropagationConfig {
    let propagation_block = cache_blocks(configuration).into_iter().find_map(|block| {
        block
            .directives
            .get("purge_propagation")
            .and_then(|entries| entries.first())
            .and_then(|entry| entry.children.as_ref())
    });

    let Some(block) = propagation_block else {
        return PurgePropagationConfig::default();
    };

    let control_plane_url = block
        .directives
        .get("control_plane_url")
        .and_then(|entries| entries.first())
        .and_then(|entry| entry.args.first())
        .and_then(|value| value.as_str())
        .map(|s| s.to_string());

    let shared_secret = block
        .directives
        .get("shared_secret")
        .and_then(|entries| entries.first())
        .and_then(|entry| entry.args.first())
        .and_then(|value| value.as_str())
        .map(|s| s.to_string());

    let node_id = block
        .directives
        .get("node_id")
        .and_then(|entries| entries.first())
        .and_then(|entry| entry.args.first())
        .and_then(|value| value.as_str())
        .map(|s| s.to_string());

    PurgePropagationConfig {
        control_plane_url,
        shared_secret,
        node_id,
    }
}

/// Parse the `zone` directive from a host-level cache block.
///
/// Returns the zone name as a `CacheZoneId::Named` if `zone "name"` is present,
/// or `None` if no explicit zone directive exists.
fn parse_zone_name(configuration: &LayeredConfiguration) -> Option<CacheZoneId> {
    for block in cache_blocks(configuration) {
        if let Some(entries) = block.directives.get("zone") {
            if let Some(entry) = entries.first() {
                if let Some(value) = entry.args.first().and_then(|v| v.as_str()) {
                    return Some(CacheZoneId::Named(value.to_string()));
                }
            }
        }
    }
    None
}

/// Parse `max_entries` for a named zone from the global `cache` block.
///
/// Looks for `zone "name" { max_entries = N }` inside the global cache block.
/// Returns `Some(max_entries)` if the zone is defined, `None` otherwise.
pub fn parse_global_zone_max_entries(
    configuration: &LayeredConfiguration,
    zone_name: &str,
) -> Option<usize> {
    for entry in configuration.get_entries("cache", true) {
        if let Some(children) = &entry.children {
            if let Some(zone_entries) = children.directives.get("zone") {
                for zone_entry in zone_entries {
                    if zone_entry
                        .args
                        .first()
                        .and_then(|v| v.as_str())
                        .is_some_and(|name| name == zone_name)
                    {
                        if let Some(zone_block) = &zone_entry.children {
                            if let Some(max_entries_entries) =
                                zone_block.directives.get("max_entries")
                            {
                                if let Some(entry) = max_entries_entries.first() {
                                    if let Some(value) = entry.args.first() {
                                        if let Some(n) = value.as_number() {
                                            return Some(n.max(0) as usize);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// Detect whether a global zone exists in the configuration.
///
/// A global zone exists when the global `cache` block contains `max_entries`
/// but does NOT contain any explicit `zone` blocks. In this case, all hosts
/// without an explicit `zone` directive share the global cache store.
pub fn has_global_zone(configuration: &LayeredConfiguration) -> bool {
    for entry in configuration.get_entries("cache", true) {
        if let Some(children) = &entry.children {
            let has_max_entries = children.directives.contains_key("max_entries");
            let has_zone_blocks = children.directives.contains_key("zone");
            if has_max_entries && !has_zone_blocks {
                return true;
            }
        }
    }
    false
}

/// To-disk persistence settings for a cache zone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PersistConfig {
    /// Directory the zone's snapshot and journal live in. `None` disables
    /// to-disk persistence for the zone.
    pub dir: Option<PathBuf>,
    /// How often queued mutations are flushed to the journal.
    pub interval: Duration,
    /// Whether private-scoped entries are written to disk.
    pub include_private: bool,
}

impl Default for PersistConfig {
    fn default() -> Self {
        Self {
            dir: None,
            interval: DEFAULT_PERSIST_INTERVAL,
            include_private: false,
        }
    }
}

/// Resolve persistence settings for `zone` from the layered configuration.
///
/// Each directive is taken from the first block, in precedence order, that
/// defines it:
///
/// 1. The host-level `cache` block (never for the global zone, whose store is
///    shared and must not depend on any single host's block).
/// 2. The named zone's `zone "name" { ... }` block inside the global cache.
/// 3. The global `cache` block.
///
/// Durations below `MIN_PERSIST_INTERVAL` are clamped up to it.
pub fn resolve_persist_config(
    zone: &CacheZoneId,
    configuration: &LayeredConfiguration,
) -> PersistConfig {
    // The global layer is always added first (see the http-server resolver);
    // any later layers are host/path scopes.
    let layers = &configuration.layers;

    let mut blocks: Vec<&ServerConfigurationBlock> = Vec::new();
    match zone {
        CacheZoneId::Global => {
            if let Some(global) = layers.first() {
                blocks.extend(cache_block_children(global));
            }
        }
        CacheZoneId::Host(_) => {
            for layer in layers.iter().rev() {
                blocks.extend(cache_block_children(layer));
            }
        }
        CacheZoneId::Named(name) => {
            // Host/path scopes first (everything but the global layer at
            // index 0), in priority order.
            for layer in layers.iter().skip(1).rev() {
                blocks.extend(cache_block_children(layer));
            }
            if let Some(block) = named_zone_block(configuration, name) {
                blocks.push(block);
            }
            if let Some(global) = layers.first() {
                blocks.extend(cache_block_children(global));
            }
        }
    }

    let mut dir = None;
    let mut interval = None;
    let mut include_private = None;
    for block in blocks {
        if dir.is_none() {
            dir = block_persist_dir(block);
        }
        if interval.is_none() {
            interval = block_persist_interval(block);
        }
        if include_private.is_none() {
            include_private = block_persist_private(block);
        }
    }

    PersistConfig {
        dir,
        interval: interval
            .unwrap_or(DEFAULT_PERSIST_INTERVAL)
            .max(MIN_PERSIST_INTERVAL),
        include_private: include_private.unwrap_or(false),
    }
}

/// The `cache` blocks declared by a single configuration layer.
fn cache_block_children<'a>(
    layer: &'a ServerConfigurationBlock,
) -> impl Iterator<Item = &'a ServerConfigurationBlock> + 'a {
    layer
        .directives
        .get("cache")
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.children.as_ref())
}

/// The `zone "name" { ... }` block inside the global `cache` block, if any.
fn named_zone_block<'a>(
    configuration: &'a LayeredConfiguration,
    zone_name: &str,
) -> Option<&'a ServerConfigurationBlock> {
    for entry in configuration.get_entries("cache", true) {
        let Some(children) = &entry.children else {
            continue;
        };
        let Some(zone_entries) = children.directives.get("zone") else {
            continue;
        };
        for zone_entry in zone_entries {
            if zone_entry
                .args
                .first()
                .and_then(|v| v.as_str())
                .is_some_and(|name| name == zone_name)
            {
                return zone_entry.children.as_ref();
            }
        }
    }
    None
}

fn block_persist_dir(block: &ServerConfigurationBlock) -> Option<PathBuf> {
    block
        .directives
        .get("persist")?
        .first()?
        .args
        .first()?
        .as_str()
        .map(PathBuf::from)
}

fn block_persist_interval(block: &ServerConfigurationBlock) -> Option<Duration> {
    let value = block
        .directives
        .get("persist_interval")?
        .first()?
        .args
        .first()?
        .as_str()?;
    parse_duration(value).ok()
}

fn block_persist_private(block: &ServerConfigurationBlock) -> Option<bool> {
    block
        .directives
        .get("persist_private")?
        .first()?
        .args
        .first()
        .and_then(|value| value.as_boolean())
        .or(Some(true))
}
