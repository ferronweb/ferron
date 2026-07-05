//! Admin API configuration parsing.
//!
//! Parses the `admin { ... }` block from global configuration.

use std::collections::HashMap;
use std::net::SocketAddr;

use ferron_core::config::ServerConfigurationBlock;

/// Parsed admin API configuration.
///
/// Created from the `admin { ... }` global configuration block.
#[derive(Debug, Clone)]
pub struct AdminConfig {
    /// Address to bind the admin HTTP listener.
    pub listen: SocketAddr,
    /// Optional bearer token for authenticating admin API requests.
    /// When `Some`, clients must send `Authorization: Bearer <token>` header.
    /// The `/health` endpoint is always exempt from authentication.
    pub auth_token: Option<String>,
    /// Whether the `/health` endpoint is enabled.
    pub health: bool,
    /// Whether the `/status` endpoint is enabled.
    pub status: bool,
    /// Whether the `/config` endpoint is enabled.
    pub config: bool,
    /// Whether the `/reload` POST endpoint is enabled.
    pub reload: bool,
    /// Whether the `/reload` GET endpoint is enabled.
    pub reload_get: bool,
    /// Whether the `/runtime` endpoint is enabled.
    pub runtime: bool,
}

impl AdminConfig {
    /// Parse admin configuration from the global config block.
    ///
    /// Looks for the `admin` directive with a nested configuration block.
    /// Returns `None` if the `admin` directive is not present (admin API disabled).
    pub fn from_global(global_config: &ServerConfigurationBlock) -> Option<Self> {
        let admin_entries = global_config.directives.get("admin")?;
        let admin_entry = admin_entries.first()?;
        let admin_block = admin_entry.children.as_ref()?;

        let listen = parse_listen(admin_block)
            .unwrap_or_else(|| "127.0.0.1:8081".parse().expect("default listen address"));
        let auth_token = parse_string_value(admin_block, "auth_token");
        let health = parse_bool_flag(admin_block, "health").unwrap_or(true);
        let status = parse_bool_flag(admin_block, "status").unwrap_or(true);
        let config = parse_bool_flag(admin_block, "config").unwrap_or(true);
        let reload = parse_bool_flag(admin_block, "reload").unwrap_or(true);
        let reload_get = parse_bool_flag(admin_block, "reload_get").unwrap_or(true);
        let runtime = parse_bool_flag(admin_block, "runtime").unwrap_or(true);

        Some(Self {
            listen,
            auth_token,
            health,
            status,
            config,
            reload,
            reload_get,
            runtime,
        })
    }
}

/// Parse the `listen` directive from the admin config block.
fn parse_listen(block: &ServerConfigurationBlock) -> Option<SocketAddr> {
    let entries = block.directives.get("listen")?;
    let entry = entries.first()?;
    let value = entry.args.first()?;
    let addr_str = value.as_string_with_interpolations(&HashMap::new())?;
    addr_str.parse().ok()
}

/// Parse a string value from the config block.
///
/// Returns `None` if the directive is not present.
fn parse_string_value(block: &ServerConfigurationBlock, directive: &str) -> Option<String> {
    let entries = block.directives.get(directive)?;
    let entry = entries.first()?;
    entry
        .args
        .first()
        .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
        .map(|s| s.to_string())
}

/// Parse a boolean flag from the admin config block.
///
/// Returns `None` if the directive is not present.
/// Accepts `true`, `false`, or bare presence (counts as `true`).
fn parse_bool_flag(block: &ServerConfigurationBlock, directive: &str) -> Option<bool> {
    let entries = block.directives.get(directive)?;
    let entry = entries.first()?;

    if entry.args.is_empty() {
        // Bare directive, e.g. `health` without a value
        return Some(true);
    }

    Some(entry.get_flag())
}
