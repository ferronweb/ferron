use cidr::IpCidr;
use ferron_core::config::validator::ConfigurationValidator;
use ferron_core::config::{
    ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationSpan,
    ServerConfigurationValue,
};
use ferron_core::validate_directive;

/// Recognized directives inside an `abuse_protection { ... }` block.
const RECOGNIZED_DIRECTIVES: &[&str] = &[
    "enabled",
    "ban_duration",
    "rate_limit_threshold",
    "brute_force_threshold",
    "custom_threshold",
    "error_rate_threshold",
    "allowlist",
];

/// Recognized directives inside a threshold block.
const THRESHOLD_DIRECTIVES: &[&str] = &["events", "window"];

/// Recognized directives inside an `error_rate_threshold { ... }` block.
const ERROR_RATE_THRESHOLD_DIRECTIVES: &[&str] = &["events", "window", "status_codes"];

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
                if !entry.get_flag() {
                    ctx.add_best_practice_violation(
                        "`abuse_protection false` disables IP banning for repeated abuse events; keep it enabled unless another layer handles abusive clients",
                        entry_span(entry),
                    );
                }
                if let Some(ref children) = entry.children {
                    self.validate_abuse_protection_block(children, ctx)?;
                }
            }
        }

        validate_directive!(config, ctx.used_directives, abuse_event, args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::Boolean(false, _)], {});

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

        // Validate error_rate_threshold blocks
        if let Some(entries) = block.directives.get("error_rate_threshold") {
            sub.insert("error_rate_threshold".to_string());
            for entry in entries {
                if let Some(ref children) = entry.children {
                    self.validate_error_rate_threshold_block(children)?;
                }
            }
        }

        if let Some(entries) = block.directives.get("allowlist") {
            sub.insert("allowlist".to_string());
            for entry in entries {
                if entry.args.is_empty() {
                    return Err("Invalid `allowlist` — expected at least one IP or CIDR".into());
                }
                for arg in &entry.args {
                    let value = arg
                        .as_str()
                        .ok_or("Invalid `allowlist` — expected string IP/CIDR values")?;
                    value.parse::<IpCidr>().map_err(|_| {
                        format!("Invalid `allowlist` — invalid IP or CIDR `{value}`")
                    })?;
                    if value == "0.0.0.0/0" || value == "::/0" {
                        ctx.add_best_practice_violation(
                            "`allowlist` exempts every source address from abuse protection; restrict it to known trusted clients",
                            entry_span(entry),
                        );
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

    fn validate_error_rate_threshold_block(
        &self,
        block: &ServerConfigurationBlock,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Check all directives are recognized
        for directive_name in block.directives.keys() {
            if !ERROR_RATE_THRESHOLD_DIRECTIVES.contains(&directive_name.as_str()) {
                return Err(format!(
                    "Invalid `{directive_name}` — unknown directive in error_rate_threshold block"
                )
                .into());
            }
        }

        // Validate `events` — optional, must be a positive integer (default: 50)
        if let Some(entries) = block.directives.get("events") {
            for entry in entries {
                self.validate_number_entry(entry, "events", 1)?;
            }
        }

        // Validate `window` — optional, must be a duration (default: 60s)
        if let Some(entries) = block.directives.get("window") {
            for entry in entries {
                self.validate_duration_entry(entry, "window")?;
            }
        }

        // Validate `status_codes` — optional, must have at least one valid status code
        if let Some(entries) = block.directives.get("status_codes") {
            for entry in entries {
                if entry.args.is_empty() {
                    return Err(
                        "Invalid `status_codes` — expected at least one status code".into(),
                    );
                }
                for arg in &entry.args {
                    let value = arg
                        .as_str()
                        .ok_or("Invalid `status_codes` — expected string status code values")?;
                    let code: u16 = value.parse().map_err(|_| {
                        format!("Invalid `status_codes` — invalid status code `{value}`")
                    })?;
                    if !(100..=599).contains(&code) {
                        return Err(format!(
                            "Invalid `status_codes` — status code `{value}` must be between 100 and 599"
                        )
                        .into());
                    }
                }
            }
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
