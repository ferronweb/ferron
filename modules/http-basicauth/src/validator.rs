//! Configuration validator for `basic_auth` directives.
//!
//! Validates that `basic_auth` blocks contain recognized directives,
/// that all password values are proper hashes (Argon2, PBKDF2, or scrypt),
/// and that nested blocks use only known directive names.
use std::collections::HashSet;

use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::config::{
    ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationValue,
};
use ferron_core::validate_directive;

/// Recognized directives inside a `basic_auth { ... }` block.
const BASICAUTH_DIRECTIVES: &[&str] = &["realm", "users", "brute_force_protection"];

/// Recognized directives inside a `brute_force_protection { ... }` block.
const BRUTE_FORCE_DIRECTIVES: &[&str] = &["enabled", "max_attempts", "lockout_duration", "window"];

/// Validator for `basic_auth` configuration blocks.
#[derive(Default)]
pub struct BasicAuthValidator;

impl ConfigurationValidator for BasicAuthValidator {
    fn validate_block(
        &self,
        config: &ServerConfigurationBlock,
        used_directives: &mut HashSet<String>,
        is_global: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if is_global {
            validate_directive!(config, used_directives, basic_auth_concurrency, args(1) => [ServerConfigurationValue::Number(_, _) | ServerConfigurationValue::Boolean(false, _)], {});
        }

        if let Some(entries) = config.directives.get("basic_auth") {
            used_directives.insert("basic_auth".to_string());
            for entry in entries {
                if let Some(ref children) = entry.children {
                    self.validate_basic_auth_block(children)?;
                }
            }
        }

        Ok(())
    }
}

impl BasicAuthValidator {
    fn validate_basic_auth_block(
        &self,
        block: &ServerConfigurationBlock,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Check all directives are recognized
        for directive_name in block.directives.keys() {
            if !BASICAUTH_DIRECTIVES.contains(&directive_name.as_str()) {
                return Err(format!(
                    "Invalid `{directive_name}` — unknown directive in basic_auth block. \
                     Recognized directives: realm, users, brute_force_protection"
                )
                .into());
            }
        }

        // Validate `realm` — optional, must be a string
        if let Some(entries) = block.directives.get("realm") {
            for entry in entries {
                self.validate_single_string_entry(entry, "realm")?;
            }
        }

        // Validate `users` block — required, must have at least one user with a hash
        let users_entries = block.directives.get("users");
        if users_entries.is_none() {
            return Err("Invalid `basic_auth` — missing required `users` block".into());
        }

        for users_entry in users_entries.into_iter().flatten() {
            if let Some(ref users_block) = users_entry.children {
                self.validate_users_block(users_block)?;
            } else {
                return Err(
                    "Invalid `basic_auth` — `users` must be a block form: `users {{ ... }}`".into(),
                );
            }
        }

        // Validate `brute_force_protection` block — optional
        if let Some(bfp_entries) = block.directives.get("brute_force_protection") {
            for bfp_entry in bfp_entries {
                if let Some(ref bfp_block) = bfp_entry.children {
                    self.validate_brute_force_block(bfp_block)?;
                }
            }
        }

        Ok(())
    }

    fn validate_users_block(
        &self,
        block: &ServerConfigurationBlock,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if block.directives.is_empty() {
            return Err(
                "Invalid `basic_auth` — `users` block must contain at least one user".into(),
            );
        }

        for (username, entries) in block.directives.iter() {
            for entry in entries {
                // Each user must have exactly one string argument (the hash)
                self.validate_single_string_entry(entry, &format!("user '{username}'"))?;

                // The value must be a supported hash format
                if let Some(value) = entry.args.first() {
                    if let Some(hash_str) = value.as_str() {
                        Self::validate_password_hash(hash_str, username)?;
                    }
                }
            }
        }

        Ok(())
    }

    fn validate_password_hash(
        hash: &str,
        username: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Check for known hash prefixes
        let is_valid = hash.starts_with("$argon2id$")
            || hash.starts_with("$argon2i$")
            || hash.starts_with("$argon2d$")
            || hash.starts_with("$pbkdf2$")
            || hash.starts_with("$pbkdf2-sha256$")
            || hash.starts_with("$scrypt$");

        if !is_valid {
            return Err(format!(
                "Invalid `basic_auth` — password for user '{username}' must be a hashed value. \
                 Supported formats: Argon2 ($argon2id$, $argon2i$, $argon2d$), \
                 PBKDF2 ($pbkdf2$, $pbkdf2-sha256$), or scrypt ($scrypt$). \
                 Plaintext passwords are not allowed for security reasons."
            )
            .into());
        }

        Ok(())
    }

    fn validate_brute_force_block(
        &self,
        block: &ServerConfigurationBlock,
    ) -> Result<(), Box<dyn std::error::Error>> {
        for directive_name in block.directives.keys() {
            if !BRUTE_FORCE_DIRECTIVES.contains(&directive_name.as_str()) {
                return Err(format!(
                    "Invalid `{directive_name}` — unknown directive in brute_force_protection block. \
                     Recognized directives: enabled, max_attempts, lockout_duration, window"
                )
                .into());
            }
        }

        // Validate `enabled` — optional, must be boolean
        if let Some(entries) = block.directives.get("enabled") {
            for entry in entries {
                if entry.args.first().and_then(|v| v.as_boolean()).is_none() {
                    return Err(
                        "Invalid `brute_force_protection` — `enabled` must be a boolean value"
                            .into(),
                    );
                }
            }
        }

        // Validate `max_attempts` — optional, must be positive integer
        if let Some(entries) = block.directives.get("max_attempts") {
            for entry in entries {
                self.validate_positive_number_entry(entry, "max_attempts")?;
            }
        }

        // Validate `lockout_duration` — optional, must be a duration string or number
        if let Some(entries) = block.directives.get("lockout_duration") {
            for entry in entries {
                self.validate_duration_entry(entry, "lockout_duration")?;
            }
        }

        // Validate `window` — optional, must be a duration string or number
        if let Some(entries) = block.directives.get("window") {
            for entry in entries {
                self.validate_duration_entry(entry, "window")?;
            }
        }

        Ok(())
    }

    /// Validate that an entry has exactly one string argument.
    fn validate_single_string_entry(
        &self,
        entry: &ServerConfigurationDirectiveEntry,
        name: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let value = entry
            .args
            .first()
            .ok_or(format!("Invalid `basic_auth` — {name} must have a value"))?;

        if value.as_str().is_none() {
            return Err(format!("Invalid `basic_auth` — {name} must be a string value").into());
        }

        Ok(())
    }

    /// Validate that an entry has exactly one positive number argument.
    fn validate_positive_number_entry(
        &self,
        entry: &ServerConfigurationDirectiveEntry,
        name: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let value = entry
            .args
            .first()
            .ok_or(format!("Invalid `basic_auth` — {name} must have a value"))?;

        let n = value.as_number().ok_or(format!(
            "Invalid `basic_auth` — {name} must be an integer value"
        ))?;

        if n <= 0 {
            return Err(format!("Invalid `basic_auth` — {name} must be a positive integer").into());
        }

        Ok(())
    }

    /// Validate that an entry has a duration string or number value.
    fn validate_duration_entry(
        &self,
        entry: &ServerConfigurationDirectiveEntry,
        name: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let value = entry
            .args
            .first()
            .ok_or(format!("Invalid `basic_auth` — {name} must have a value"))?;

        if value.as_str().is_none() && value.as_number().is_none() {
            return Err(format!(
                "Invalid `basic_auth` — {name} must be a duration string (e.g., '15m', '1h') or a number"
            )
            .into());
        }

        Ok(())
    }
}
