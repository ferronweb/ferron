//! Configuration parsing for `basicauth` blocks.
//!
//! Parses `basicauth { ... }` directive entries from the layered configuration
//! into typed `BasicAuthConfig` structures. Only hashed passwords are supported.

use std::collections::HashMap;

use ferron_core::config::layer::LayeredConfiguration;
use ferron_core::config::ServerConfigurationBlock;

use crate::brute_force::BruteForceConfig;

/// A single basic auth rule parsed from configuration.
#[derive(Debug, Clone)]
pub struct BasicAuthConfig {
    /// Authentication realm shown in the auth dialog.
    pub realm: String,
    /// Map of username → hashed password (Argon2, PBKDF2, or scrypt).
    pub users: HashMap<String, String>,
    /// Brute-force protection settings (enabled by default).
    pub brute_force: BruteForceConfig,
}

impl BasicAuthConfig {
    /// Default realm name.
    pub const DEFAULT_REALM: &str = "Restricted Access";
}

/// Parse all `basicauth` directives from the layered configuration.
///
/// Returns `Some(config)` if at least one `basicauth` block is found,
/// merging users from all blocks. Returns `None` if no `basicauth` is configured.
pub fn parse_basicauth_config(config: &LayeredConfiguration) -> Option<BasicAuthConfig> {
    let entries = config.get_entries("basicauth", true);
    if entries.is_empty() {
        return None;
    }

    let mut merged_config = BasicAuthConfig {
        realm: BasicAuthConfig::DEFAULT_REALM.to_string(),
        users: HashMap::new(),
        brute_force: BruteForceConfig::default(),
    };

    for entry in entries {
        if let Some(children) = &entry.children {
            parse_basicauth_block(children, &mut merged_config);
        }
    }

    if merged_config.users.is_empty() {
        None
    } else {
        Some(merged_config)
    }
}

fn parse_basicauth_block(block: &ServerConfigurationBlock, config: &mut BasicAuthConfig) {
    // Parse optional `realm`
    if let Some(realm_val) = block.get_value("realm") {
        if let Some(realm_str) = realm_val.as_str() {
            config.realm = realm_str.to_string();
        }
    }

    // Parse `users` block
    if let Some(users_entries) = block.directives.get("users") {
        for users_entry in users_entries {
            if let Some(ref users_block) = users_entry.children {
                parse_users_block(users_block, &mut config.users);
            }
        }
    }

    // Parse `brute_force_protection` block
    if let Some(bfp_entries) = block.directives.get("brute_force_protection") {
        for bfp_entry in bfp_entries {
            if let Some(ref bfp_block) = bfp_entry.children {
                parse_brute_force_block(bfp_block, &mut config.brute_force);
            }
        }
    }
}

fn parse_users_block(block: &ServerConfigurationBlock, users: &mut HashMap<String, String>) {
    // Each directive inside `users { ... }` is a username with the hash as its argument.
    // e.g.: `alice "$argon2id$..."`
    for (username, entries) in block.directives.iter() {
        for entry in entries {
            if let Some(hash_val) = entry.args.first() {
                if let Some(hash_str) = hash_val.as_str() {
                    users.insert(username.clone(), hash_str.to_string());
                }
            }
        }
    }
}

fn parse_brute_force_block(block: &ServerConfigurationBlock, bfc: &mut BruteForceConfig) {
    // Parse `enabled` — optional flag
    if let Some(enabled_val) = block.get_value("enabled") {
        if let Some(enabled) = enabled_val.as_boolean() {
            bfc.enabled = enabled;
        }
    }

    // Parse `max_attempts` — optional, default 5
    if let Some(max_attempts_val) = block.get_value("max_attempts") {
        if let Some(n) = max_attempts_val.as_number() {
            if n > 0 {
                bfc.max_attempts = n as usize;
            }
        }
    }

    // Parse `lockout_duration` — optional, accepts duration string or seconds
    if let Some(ld_val) = block.get_value("lockout_duration") {
        if let Some(secs) = parse_duration_value(ld_val) {
            if secs > 0 {
                bfc.lockout_duration_secs = secs;
            }
        }
    }

    // Parse `window` — optional, accepts duration string or seconds
    if let Some(w_val) = block.get_value("window") {
        if let Some(secs) = parse_duration_value(w_val) {
            if secs > 0 {
                bfc.window_secs = secs;
            }
        }
    }
}

/// Parse a duration value that can be either a number (seconds) or a duration string
/// like "15m", "1h", "5m".
fn parse_duration_value(val: &ferron_core::config::ServerConfigurationValue) -> Option<u64> {
    // Try as a number first (seconds)
    if let Some(n) = val.as_number() {
        return Some(n as u64);
    }

    // Try as a string with duration suffix
    if let Some(s) = val.as_str() {
        return parse_duration_string(s);
    }

    None
}

/// Parse a duration string like "15m", "1h", "30s", "1d".
fn parse_duration_string(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    let last_char = s.chars().last()?;
    match last_char {
        's' | 'S' => s[..s.len() - 1].parse::<u64>().ok(),
        'm' | 'M' => {
            let minutes = s[..s.len() - 1].parse::<u64>().ok()?;
            Some(minutes * 60)
        }
        'h' | 'H' => {
            let hours = s[..s.len() - 1].parse::<u64>().ok()?;
            Some(hours * 3600)
        }
        'd' | 'D' => {
            let days = s[..s.len() - 1].parse::<u64>().ok()?;
            Some(days * 86400)
        }
        _ => {
            // Plain number — treat as seconds
            s.parse::<u64>().ok()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferron_core::config::{
        ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationValue,
    };
    use std::collections::HashMap as StdHashMap;
    use std::sync::Arc;

    fn make_value_string(s: &str) -> ServerConfigurationValue {
        ServerConfigurationValue::String(s.to_string(), None)
    }

    fn make_value_number(n: i64) -> ServerConfigurationValue {
        ServerConfigurationValue::Number(n, None)
    }

    fn make_value_boolean(b: bool) -> ServerConfigurationValue {
        ServerConfigurationValue::Boolean(b, None)
    }

    fn make_users_block(users: Vec<(&str, &str)>) -> ServerConfigurationBlock {
        let mut directives = StdHashMap::new();
        for (username, hash) in users {
            directives.insert(
                username.to_string(),
                vec![ServerConfigurationDirectiveEntry {
                    args: vec![make_value_string(hash)],
                    children: None,
                    span: None,
                }],
            );
        }
        ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: StdHashMap::new(),
            span: None,
        }
    }

    fn make_parent_basicauth_block(
        realm: Option<&str>,
        users_block: ServerConfigurationBlock,
        brute_force_block: Option<ServerConfigurationBlock>,
    ) -> ServerConfigurationBlock {
        let mut directives = StdHashMap::new();

        if let Some(r) = realm {
            directives.insert(
                "realm".to_string(),
                vec![ServerConfigurationDirectiveEntry {
                    args: vec![make_value_string(r)],
                    children: None,
                    span: None,
                }],
            );
        }

        directives.insert(
            "users".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![],
                children: Some(users_block),
                span: None,
            }],
        );

        if let Some(bf_block) = brute_force_block {
            directives.insert(
                "brute_force_protection".to_string(),
                vec![ServerConfigurationDirectiveEntry {
                    args: vec![],
                    children: Some(bf_block),
                    span: None,
                }],
            );
        }

        ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: StdHashMap::new(),
            span: None,
        }
    }

    #[test]
    fn parses_minimal_basicauth_block() {
        let users = make_users_block(vec![("alice", "$argon2id$v=19$m=19456,t=2,p=1$abc123")]);
        let parent = make_parent_basicauth_block(None, users, None);

        let mut config = BasicAuthConfig {
            realm: BasicAuthConfig::DEFAULT_REALM.to_string(),
            users: HashMap::new(),
            brute_force: BruteForceConfig::default(),
        };
        parse_basicauth_block(&parent, &mut config);

        assert_eq!(config.realm, "Restricted Access");
        assert_eq!(config.users.len(), 1);
        assert_eq!(
            config.users.get("alice"),
            Some(&"$argon2id$v=19$m=19456,t=2,p=1$abc123".to_string())
        );
        assert!(config.brute_force.enabled);
    }

    #[test]
    fn parses_full_basicauth_block() {
        let users = make_users_block(vec![
            ("alice", "$argon2id$hash1"),
            ("bob", "$argon2id$hash2"),
        ]);

        let bf_block = ServerConfigurationBlock {
            directives: Arc::new(StdHashMap::from([
                (
                    "enabled".to_string(),
                    vec![ServerConfigurationDirectiveEntry {
                        args: vec![make_value_boolean(true)],
                        children: None,
                        span: None,
                    }],
                ),
                (
                    "max_attempts".to_string(),
                    vec![ServerConfigurationDirectiveEntry {
                        args: vec![make_value_number(3)],
                        children: None,
                        span: None,
                    }],
                ),
                (
                    "lockout_duration".to_string(),
                    vec![ServerConfigurationDirectiveEntry {
                        args: vec![make_value_string("10m")],
                        children: None,
                        span: None,
                    }],
                ),
                (
                    "window".to_string(),
                    vec![ServerConfigurationDirectiveEntry {
                        args: vec![make_value_string("2m")],
                        children: None,
                        span: None,
                    }],
                ),
            ])),
            matchers: StdHashMap::new(),
            span: None,
        };

        let parent = make_parent_basicauth_block(Some("My Realm"), users, Some(bf_block));

        let mut config = BasicAuthConfig {
            realm: BasicAuthConfig::DEFAULT_REALM.to_string(),
            users: HashMap::new(),
            brute_force: BruteForceConfig::default(),
        };
        parse_basicauth_block(&parent, &mut config);

        assert_eq!(config.realm, "My Realm");
        assert_eq!(config.users.len(), 2);
        assert_eq!(config.brute_force.max_attempts, 3);
        assert_eq!(config.brute_force.lockout_duration_secs, 600);
        assert_eq!(config.brute_force.window_secs, 120);
    }

    #[test]
    fn merges_users_from_multiple_blocks() {
        let users1 = make_users_block(vec![("alice", "$argon2id$hash1")]);
        let users2 = make_users_block(vec![("bob", "$argon2id$hash2")]);

        let mut config = BasicAuthConfig {
            realm: BasicAuthConfig::DEFAULT_REALM.to_string(),
            users: HashMap::new(),
            brute_force: BruteForceConfig::default(),
        };
        parse_basicauth_block(
            &make_parent_basicauth_block(None, users1, None),
            &mut config,
        );
        parse_basicauth_block(
            &make_parent_basicauth_block(None, users2, None),
            &mut config,
        );

        assert_eq!(config.users.len(), 2);
        assert!(config.users.contains_key("alice"));
        assert!(config.users.contains_key("bob"));
    }

    #[test]
    fn parses_duration_strings() {
        assert_eq!(parse_duration_string("30s"), Some(30));
        assert_eq!(parse_duration_string("5m"), Some(300));
        assert_eq!(parse_duration_string("1h"), Some(3600));
        assert_eq!(parse_duration_string("1d"), Some(86400));
        assert_eq!(parse_duration_string("900"), Some(900));
        assert_eq!(parse_duration_string(""), None);
        assert_eq!(parse_duration_string("abc"), None);
    }
}
