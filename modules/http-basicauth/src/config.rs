//! Configuration parsing for `basic_auth` blocks.
//!
//! Parses `basic_auth { ... }` directive entries from the layered configuration
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
    let entries = config.get_entries("basic_auth", true);
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
