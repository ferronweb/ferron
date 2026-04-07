//! Configuration validator for `basicauth` directives.
//!
//! Validates that `basicauth` blocks contain recognized directives,
/// that all password values are proper hashes (Argon2, PBKDF2, or scrypt),
/// and that nested blocks use only known directive names.
use std::collections::HashSet;

use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::config::{ServerConfigurationBlock, ServerConfigurationDirectiveEntry};

/// Recognized directives inside a `basicauth { ... }` block.
const BASICAUTH_DIRECTIVES: &[&str] = &["realm", "users", "brute_force_protection"];

/// Recognized directives inside a `brute_force_protection { ... }` block.
const BRUTE_FORCE_DIRECTIVES: &[&str] = &["enabled", "max_attempts", "lockout_duration", "window"];

/// Validator for `basicauth` configuration blocks.
#[derive(Default)]
pub struct BasicAuthValidator;

impl ConfigurationValidator for BasicAuthValidator {
    fn validate_block(
        &self,
        config: &ServerConfigurationBlock,
        used_directives: &mut HashSet<String>,
        _is_global: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(entries) = config.directives.get("basicauth") {
            used_directives.insert("basicauth".to_string());
            for entry in entries {
                if let Some(ref children) = entry.children {
                    self.validate_basicauth_block(children)?;
                }
            }
        }

        Ok(())
    }
}

impl BasicAuthValidator {
    fn validate_basicauth_block(
        &self,
        block: &ServerConfigurationBlock,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Check all directives are recognized
        for directive_name in block.directives.keys() {
            if !BASICAUTH_DIRECTIVES.contains(&directive_name.as_str()) {
                return Err(format!(
                    "Invalid `{directive_name}` — unknown directive in basicauth block. \
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
            return Err("Invalid `basicauth` — missing required `users` block".into());
        }

        for users_entry in users_entries.into_iter().flatten() {
            if let Some(ref users_block) = users_entry.children {
                self.validate_users_block(users_block)?;
            } else {
                return Err(
                    "Invalid `basicauth` — `users` must be a block form: `users {{ ... }}`".into(),
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
                "Invalid `basicauth` — `users` block must contain at least one user".into(),
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
                "Invalid `basicauth` — password for user '{username}' must be a hashed value. \
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
            .ok_or(format!("Invalid `basicauth` — {name} must have a value"))?;

        if value.as_str().is_none() {
            return Err(format!("Invalid `basicauth` — {name} must be a string value").into());
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
            .ok_or(format!("Invalid `basicauth` — {name} must have a value"))?;

        let n = value.as_number().ok_or(format!(
            "Invalid `basicauth` — {name} must be an integer value"
        ))?;

        if n <= 0 {
            return Err(format!("Invalid `basicauth` — {name} must be a positive integer").into());
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
            .ok_or(format!("Invalid `basicauth` — {name} must have a value"))?;

        if value.as_str().is_none() && value.as_number().is_none() {
            return Err(format!(
                "Invalid `basicauth` — {name} must be a duration string (e.g., '15m', '1h') or a number"
            )
            .into());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferron_core::config::{ServerConfigurationDirectiveEntry, ServerConfigurationValue};
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

    fn make_basicauth_block(
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

    fn make_parent_with_basicauth(children: ServerConfigurationBlock) -> ServerConfigurationBlock {
        let mut d = StdHashMap::new();
        d.insert(
            "basicauth".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![],
                children: Some(children),
                span: None,
            }],
        );
        ServerConfigurationBlock {
            directives: Arc::new(d),
            matchers: StdHashMap::new(),
            span: None,
        }
    }

    #[test]
    fn valid_basicauth_block_with_argon2() {
        let users = make_users_block(vec![("alice", "$argon2id$v=19$m=19456,t=2,p=1$abc123")]);
        let inner = make_basicauth_block(None, users, None);
        let parent = make_parent_with_basicauth(inner);
        let mut used = HashSet::new();
        let validator = BasicAuthValidator;
        assert!(validator.validate_block(&parent, &mut used, false).is_ok());
        assert!(used.contains("basicauth"));
    }

    #[test]
    fn valid_basicauth_block_with_all_options() {
        let users = make_users_block(vec![("alice", "$argon2id$v=19$m=19456,t=2,p=1$abc123")]);
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
                        args: vec![make_value_number(5)],
                        children: None,
                        span: None,
                    }],
                ),
                (
                    "lockout_duration".to_string(),
                    vec![ServerConfigurationDirectiveEntry {
                        args: vec![make_value_string("15m")],
                        children: None,
                        span: None,
                    }],
                ),
                (
                    "window".to_string(),
                    vec![ServerConfigurationDirectiveEntry {
                        args: vec![make_value_string("5m")],
                        children: None,
                        span: None,
                    }],
                ),
            ])),
            matchers: StdHashMap::new(),
            span: None,
        };
        let inner = make_basicauth_block(Some("My Realm"), users, Some(bf_block));
        let parent = make_parent_with_basicauth(inner);
        let mut used = HashSet::new();
        let validator = BasicAuthValidator;
        assert!(validator.validate_block(&parent, &mut used, false).is_ok());
    }

    #[test]
    fn rejects_plaintext_password() {
        let users = make_users_block(vec![("alice", "my-plain-password")]);
        let inner = make_basicauth_block(None, users, None);
        let parent = make_parent_with_basicauth(inner);
        let mut used = HashSet::new();
        let validator = BasicAuthValidator;
        let err = validator
            .validate_block(&parent, &mut used, false)
            .unwrap_err();
        assert!(err.to_string().contains("hashed"));
        assert!(err.to_string().contains("alice"));
    }

    #[test]
    fn rejects_unknown_directive_in_basicauth() {
        let users = make_users_block(vec![("alice", "$argon2id$hash")]);
        let mut inner = make_basicauth_block(None, users, None);
        let directives = Arc::make_mut(&mut inner.directives);
        directives.insert(
            "unknown_directive".to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![make_value_string("value")],
                children: None,
                span: None,
            }],
        );
        let parent = make_parent_with_basicauth(inner);
        let mut used = HashSet::new();
        let validator = BasicAuthValidator;
        let err = validator
            .validate_block(&parent, &mut used, false)
            .unwrap_err();
        assert!(err.to_string().contains("unknown directive"));
    }

    #[test]
    fn rejects_empty_users_block() {
        let users = ServerConfigurationBlock {
            directives: Arc::new(StdHashMap::new()),
            matchers: StdHashMap::new(),
            span: None,
        };
        let inner = make_basicauth_block(None, users, None);
        let parent = make_parent_with_basicauth(inner);
        let mut used = HashSet::new();
        let validator = BasicAuthValidator;
        let err = validator
            .validate_block(&parent, &mut used, false)
            .unwrap_err();
        assert!(err.to_string().contains("at least one user"));
    }

    #[test]
    fn rejects_missing_users_block() {
        let inner = ServerConfigurationBlock {
            directives: Arc::new(StdHashMap::from([(
                "realm".to_string(),
                vec![ServerConfigurationDirectiveEntry {
                    args: vec![make_value_string("My Realm")],
                    children: None,
                    span: None,
                }],
            )])),
            matchers: StdHashMap::new(),
            span: None,
        };
        let parent = make_parent_with_basicauth(inner);
        let mut used = HashSet::new();
        let validator = BasicAuthValidator;
        let err = validator
            .validate_block(&parent, &mut used, false)
            .unwrap_err();
        assert!(err.to_string().contains("missing required `users` block"));
    }

    #[test]
    fn rejects_invalid_hash_format() {
        // $md5$ is not a supported format
        let users = make_users_block(vec![("alice", "$md5$abc123")]);
        let inner = make_basicauth_block(None, users, None);
        let parent = make_parent_with_basicauth(inner);
        let mut used = HashSet::new();
        let validator = BasicAuthValidator;
        let err = validator
            .validate_block(&parent, &mut used, false)
            .unwrap_err();
        assert!(err.to_string().contains("alice"));
        assert!(err.to_string().contains("hashed"));
    }

    #[test]
    fn accepts_pbkdf2_hash() {
        let users = make_users_block(vec![("alice", "$pbkdf2-sha256$abc123")]);
        let inner = make_basicauth_block(None, users, None);
        let parent = make_parent_with_basicauth(inner);
        let mut used = HashSet::new();
        let validator = BasicAuthValidator;
        assert!(validator.validate_block(&parent, &mut used, false).is_ok());
    }

    #[test]
    fn accepts_scrypt_hash() {
        let users = make_users_block(vec![("alice", "$scrypt$abc123")]);
        let inner = make_basicauth_block(None, users, None);
        let parent = make_parent_with_basicauth(inner);
        let mut used = HashSet::new();
        let validator = BasicAuthValidator;
        assert!(validator.validate_block(&parent, &mut used, false).is_ok());
    }

    #[test]
    fn rejects_unknown_brute_force_directive() {
        let users = make_users_block(vec![("alice", "$argon2id$hash")]);
        let bf_block = ServerConfigurationBlock {
            directives: Arc::new(StdHashMap::from([(
                "unknown_option".to_string(),
                vec![ServerConfigurationDirectiveEntry {
                    args: vec![make_value_string("value")],
                    children: None,
                    span: None,
                }],
            )])),
            matchers: StdHashMap::new(),
            span: None,
        };
        let inner = make_basicauth_block(None, users, Some(bf_block));
        let parent = make_parent_with_basicauth(inner);
        let mut used = HashSet::new();
        let validator = BasicAuthValidator;
        let err = validator
            .validate_block(&parent, &mut used, false)
            .unwrap_err();
        assert!(err.to_string().contains("unknown directive"));
    }

    #[test]
    fn skips_block_without_basicauth() {
        let block = ServerConfigurationBlock {
            directives: Arc::new(StdHashMap::new()),
            matchers: StdHashMap::new(),
            span: None,
        };
        let mut used = HashSet::new();
        let validator = BasicAuthValidator;
        assert!(validator.validate_block(&block, &mut used, false).is_ok());
    }
}
