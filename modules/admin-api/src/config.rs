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
    /// Whether the `/health` endpoint is enabled.
    pub health: bool,
    /// Whether the `/status` endpoint is enabled.
    pub status: bool,
    /// Whether the `/config` endpoint is enabled.
    pub config: bool,
    /// Whether the `/reload` endpoint is enabled.
    pub reload: bool,
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
        let health = parse_bool_flag(admin_block, "health").unwrap_or(true);
        let status = parse_bool_flag(admin_block, "status").unwrap_or(true);
        let config = parse_bool_flag(admin_block, "config").unwrap_or(true);
        let reload = parse_bool_flag(admin_block, "reload").unwrap_or(true);

        Some(Self {
            listen,
            health,
            status,
            config,
            reload,
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

    let value = entry.args.first()?;
    if let Some(b) = value.as_boolean() {
        Some(b)
    } else if let Some(s) = value.as_str() {
        match s {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        }
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferron_core::config::{
        ServerConfigurationBlockBuilder, ServerConfigurationDirectiveEntry,
        ServerConfigurationValue,
    };

    fn admin_block(inner: ServerConfigurationBlockBuilder) -> ServerConfigurationBlock {
        ServerConfigurationBlockBuilder::new()
            .directive_with_block_builder("admin", Vec::<String>::new(), inner)
            .build()
    }

    fn bool_entry(value: bool) -> ServerConfigurationDirectiveEntry {
        ServerConfigurationDirectiveEntry {
            args: vec![ServerConfigurationValue::Boolean(value, None)],
            children: None,
            span: None,
        }
    }

    fn string_entry(s: &str) -> ServerConfigurationDirectiveEntry {
        ServerConfigurationDirectiveEntry {
            args: vec![ServerConfigurationValue::String(s.to_string(), None)],
            children: None,
            span: None,
        }
    }

    #[test]
    fn returns_none_without_admin_block() {
        let config = ServerConfigurationBlockBuilder::new().build();
        assert!(AdminConfig::from_global(&config).is_none());
    }

    #[test]
    fn parses_listen_address() {
        let inner = ServerConfigurationBlockBuilder::new()
            .directive("listen", string_entry("127.0.0.1:9090"));
        let config = admin_block(inner);

        let parsed = AdminConfig::from_global(&config).unwrap();
        assert_eq!(
            parsed.listen,
            "127.0.0.1:9090".parse::<SocketAddr>().unwrap()
        );
    }

    #[test]
    fn defaults_to_localhost_8081() {
        let inner = ServerConfigurationBlockBuilder::new();
        let config = admin_block(inner);

        let parsed = AdminConfig::from_global(&config).unwrap();
        assert_eq!(
            parsed.listen,
            "127.0.0.1:8081".parse::<SocketAddr>().unwrap()
        );
    }

    #[test]
    fn disables_endpoints_with_false() {
        let inner = ServerConfigurationBlockBuilder::new()
            .directive("health", bool_entry(false))
            .directive("status", bool_entry(false))
            .directive("config", bool_entry(false))
            .directive("reload", bool_entry(false));
        let config = admin_block(inner);

        let parsed = AdminConfig::from_global(&config).unwrap();
        assert!(!parsed.health);
        assert!(!parsed.status);
        assert!(!parsed.config);
        assert!(!parsed.reload);
    }

    #[test]
    fn enables_endpoints_with_true() {
        let inner = ServerConfigurationBlockBuilder::new()
            .directive("health", bool_entry(true))
            .directive("status", bool_entry(true));
        let config = admin_block(inner);

        let parsed = AdminConfig::from_global(&config).unwrap();
        assert!(parsed.health);
        assert!(parsed.status);
        // Not specified → defaults to true
        assert!(parsed.config);
        assert!(parsed.reload);
    }

    #[test]
    fn all_endpoints_default_to_true() {
        let inner = ServerConfigurationBlockBuilder::new()
            .directive("listen", string_entry("127.0.0.1:8081"));
        let config = admin_block(inner);

        let parsed = AdminConfig::from_global(&config).unwrap();
        assert!(parsed.health);
        assert!(parsed.status);
        assert!(parsed.config);
        assert!(parsed.reload);
    }
}
