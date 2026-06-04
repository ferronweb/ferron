use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::config::{
    ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationSpan,
    ServerConfigurationValue,
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
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let is_global = ctx.is_global;
        if is_global {
            validate_directive!(config, ctx.used_directives, basic_auth_concurrency, args(1) => [ServerConfigurationValue::Number(_, _) | ServerConfigurationValue::Boolean(false, _)], {});
            if let Some(entries) = config.directives.get("basic_auth_concurrency") {
                for entry in entries {
                    if matches!(
                        entry.args.first(),
                        Some(ServerConfigurationValue::Boolean(false, _))
                    ) {
                        ctx.add_best_practice_violation(
                            "`basic_auth_concurrency false` disables the global password-verification concurrency limit; keep a bounded limit to prevent expensive hash checks from exhausting resources",
                            entry_span(entry),
                        );
                    }
                }
            }
        }

        if let Some(entries) = config.directives.get("basic_auth") {
            ctx.used_directives.insert("basic_auth".to_string());
            for entry in entries {
                if let Some(ref children) = entry.children {
                    self.validate_basic_auth_block(children, ctx)?;
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
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut sub = std::collections::HashSet::new();

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
            sub.insert("realm".to_string());
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
                self.validate_users_block(users_block, ctx)?;
            } else {
                return Err(
                    "Invalid `basic_auth` — `users` must be a block form: `users {{ ... }}`".into(),
                );
            }
        }
        sub.insert("users".to_string());

        // Validate `brute_force_protection` block — optional
        if let Some(bfp_entries) = block.directives.get("brute_force_protection") {
            sub.insert("brute_force_protection".to_string());
            for bfp_entry in bfp_entries {
                if let Some(ref bfp_block) = bfp_entry.children {
                    self.validate_brute_force_block(bfp_block, ctx)?;
                }
            }
        }

        ferron_core::check_unused_subdirectives!(
            block,
            sub,
            &mut ctx.diagnostics,
            ctx.scope.clone()
        );
        Ok(())
    }

    fn validate_users_block(
        &self,
        block: &ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
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
                        if !hash_str.starts_with("$argon2id$") {
                            ctx.add_best_practice_violation(
                                format!(
                                    "Password hash for user '{username}' does not use Argon2id; prefer Argon2id for new Basic Auth credentials"
                                ),
                                entry_span(entry),
                            );
                        }
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
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut sub = std::collections::HashSet::new();

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
            sub.insert("enabled".to_string());
            for entry in entries {
                let enabled = entry.args.first().and_then(|v| v.as_boolean());
                if enabled.is_none() {
                    return Err(
                        "Invalid `brute_force_protection` — `enabled` must be a boolean value"
                            .into(),
                    );
                }
                if enabled == Some(false) {
                    ctx.add_best_practice_violation(
                        "`brute_force_protection.enabled false` disables credential-guessing protection; only disable it when equivalent protection exists at another layer",
                        entry_span(entry),
                    );
                }
            }
        }

        // Validate `max_attempts` — optional, must be positive integer
        if let Some(entries) = block.directives.get("max_attempts") {
            sub.insert("max_attempts".to_string());
            for entry in entries {
                self.validate_positive_number_entry(entry, "max_attempts")?;
            }
        }

        // Validate `lockout_duration` — optional, must be a duration string or number
        if let Some(entries) = block.directives.get("lockout_duration") {
            sub.insert("lockout_duration".to_string());
            for entry in entries {
                self.validate_duration_entry(entry, "lockout_duration")?;
            }
        }

        // Validate `window` — optional, must be a duration string or number
        if let Some(entries) = block.directives.get("window") {
            sub.insert("window".to_string());
            for entry in entries {
                self.validate_duration_entry(entry, "window")?;
            }
        }

        ferron_core::check_unused_subdirectives!(
            block,
            sub,
            &mut ctx.diagnostics,
            ctx.scope.clone()
        );
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

fn entry_span(entry: &ServerConfigurationDirectiveEntry) -> Option<ServerConfigurationSpan> {
    entry.span.clone().or_else(|| {
        entry.args.first().and_then(|value| match value {
            ServerConfigurationValue::String(_, span)
            | ServerConfigurationValue::Number(_, span)
            | ServerConfigurationValue::Float(_, span)
            | ServerConfigurationValue::Boolean(_, span)
            | ServerConfigurationValue::InterpolatedString(_, span) => span.clone(),
        })
    })
}
