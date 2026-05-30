use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::config::{ServerConfigurationBlock, ServerConfigurationDirectiveEntry};

/// Recognized directives inside an `abuse_protection { ... }` block.
const RECOGNIZED_DIRECTIVES: &[&str] = &[
    "enabled",
    "ban_duration",
    "rate_limit_threshold",
    "brute_force_threshold",
    "custom_threshold",
    "allowlist",
];

/// Recognized directives inside a threshold block.
const THRESHOLD_DIRECTIVES: &[&str] = &["events", "window"];

/// Validator for `abuse_protection` configuration blocks.
#[derive(Default)]
pub struct AbuseProtectionValidator;

impl ConfigurationValidator for AbuseProtectionValidator {
    fn validate_block(
        &self,
        config: &ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Check if this block contains an `abuse_protection` directive
        if let Some(entries) = config.directives.get("abuse_protection") {
            ctx.used_directives.insert("abuse_protection".to_string());
            for entry in entries {
                if let Some(ref children) = entry.children {
                    self.validate_abuse_protection_block(children, ctx)?;
                }
            }
        }

        Ok(())
    }
}

impl AbuseProtectionValidator {
    fn validate_abuse_protection_block(
        &self,
        block: &ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut sub = std::collections::HashSet::new();

        // Check all directives are recognized
        for directive_name in block.directives.keys() {
            if !RECOGNIZED_DIRECTIVES.contains(&directive_name.as_str()) {
                return Err(format!(
                    "Invalid `{directive_name}` — unknown directive in abuse_protection block"
                )
                .into());
            }
        }

        // Validate `enabled` — optional, must be a boolean
        if let Some(entries) = block.directives.get("enabled") {
            sub.insert("enabled".to_string());
            for entry in entries {
                if let Some(value) = entry.args.first() {
                    if value.as_boolean().is_none() {
                        return Err("Invalid `enabled` — must be a boolean".into());
                    }
                }
            }
        }

        // Validate `ban_duration` — optional, must be a duration
        if let Some(entries) = block.directives.get("ban_duration") {
            sub.insert("ban_duration".to_string());
            for entry in entries {
                self.validate_duration_entry(entry, "ban_duration")?;
            }
        }

        // Validate threshold blocks
        for threshold_name in &[
            "rate_limit_threshold",
            "brute_force_threshold",
            "custom_threshold",
        ] {
            if let Some(entries) = block.directives.get(*threshold_name) {
                sub.insert(threshold_name.to_string());
                for entry in entries {
                    if let Some(ref children) = entry.children {
                        self.validate_threshold_block(children, threshold_name)?;
                    }
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

    fn validate_threshold_block(
        &self,
        block: &ServerConfigurationBlock,
        threshold_name: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Check all directives are recognized
        for directive_name in block.directives.keys() {
            if !THRESHOLD_DIRECTIVES.contains(&directive_name.as_str()) {
                return Err(format!(
                    "Invalid `{directive_name}` — unknown directive in {} block",
                    threshold_name
                )
                .into());
            }
        }

        // Validate `events` — required, must be a positive integer
        let events_entry = block.directives.get("events");
        if events_entry.is_none() {
            return Err(format!(
                "Invalid `{}` — missing required `events` directive",
                threshold_name
            )
            .into());
        }

        for entry in events_entry.into_iter().flatten() {
            self.validate_number_entry(entry, "events", 1)?;
        }

        // Validate `window` — required, must be a positive integer
        let window_entry = block.directives.get("window");
        if window_entry.is_none() {
            return Err(format!(
                "Invalid `{}` — missing required `window` directive",
                threshold_name
            )
            .into());
        }

        for entry in window_entry.into_iter().flatten() {
            self.validate_duration_entry(entry, "window")?;
        }

        Ok(())
    }

    /// Validate that an entry has exactly one number argument >= min_value.
    fn validate_number_entry(
        &self,
        entry: &ServerConfigurationDirectiveEntry,
        name: &str,
        min: i64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let value = entry
            .args
            .first()
            .ok_or(format!("Invalid `{name}` — must be an integer value"))?;
        let n = value
            .as_number()
            .ok_or(format!("Invalid `{name}` — must be an integer value"))?;
        if n < min {
            return Err(format!("Invalid `{name}` — must be >= {min}").into());
        }
        Ok(())
    }

    /// Validate that an entry has exactly one duration argument.
    fn validate_duration_entry(
        &self,
        entry: &ServerConfigurationDirectiveEntry,
        name: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        entry
            .args
            .first()
            .ok_or(format!("Invalid `{name}` — must be a duration value"))?
            .as_duration()
            .ok_or(format!("Invalid `{name}` — must be a duration value"))?;

        Ok(())
    }
}
