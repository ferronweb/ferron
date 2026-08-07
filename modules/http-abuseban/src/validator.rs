use ferron_core::config::validator::{
    entry_span, ConfigurationValidationError, ConfigurationValidator,
};
use ferron_core::config::{
    ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationValue,
};
use ipnet::IpNet;

const RECOGNIZED_DIRECTIVES: &[&str] = &[
    "enabled",
    "ban_duration",
    "rate_limit_threshold",
    "brute_force_threshold",
    "custom_threshold",
    "error_rate_threshold",
    "allowlist",
];

const THRESHOLD_DIRECTIVES: &[&str] = &["events", "window"];

const ERROR_RATE_THRESHOLD_DIRECTIVES: &[&str] = &["events", "window", "status_codes"];

#[derive(Default)]
pub struct AbuseProtectionValidator;

impl ConfigurationValidator for AbuseProtectionValidator {
    fn validate_block(
        &self,
        config: &ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
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

        ferron_core::validate_directive!(config, ctx.used_directives, abuse_event, args(1) => [ServerConfigurationValue::String(_, _) | ServerConfigurationValue::Boolean(false, _)], {});

        Ok(())
    }
}

impl AbuseProtectionValidator {
    fn validate_abuse_protection_block(
        &self,
        block: &ServerConfigurationBlock,
        ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
        let mut sub = std::collections::HashSet::new();

        for directive_name in block.directives.keys() {
            if !RECOGNIZED_DIRECTIVES.contains(&directive_name.as_str()) {
                let entry = block.directives[directive_name.as_str()]
                    .first()
                    .expect("non-empty block directives");
                return Err(ConfigurationValidationError::from(format!(
                    "Invalid `{directive_name}` — unknown directive in abuse_protection block"
                ))
                .with_span(entry_span(entry)));
            }
        }

        if let Some(entries) = block.directives.get("enabled") {
            sub.insert("enabled".to_string());
            for entry in entries {
                if let Some(value) = entry.args.first() {
                    if value.as_boolean().is_none() {
                        return Err(ConfigurationValidationError::from(
                            "Invalid `enabled` — must be a boolean",
                        )
                        .with_span(entry_span(entry)));
                    }
                }
            }
        }

        if let Some(entries) = block.directives.get("ban_duration") {
            sub.insert("ban_duration".to_string());
            for entry in entries {
                self.validate_duration_entry(entry, "ban_duration")?;
            }
        }

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
                    return Err(ConfigurationValidationError::from(
                        "Invalid `allowlist` — expected at least one IP or CIDR",
                    )
                    .with_span(entry_span(entry)));
                }
                for arg in &entry.args {
                    let value = arg.as_str().ok_or_else(|| {
                        ConfigurationValidationError::from(
                            "Invalid `allowlist` — expected string IP/CIDR values",
                        )
                        .with_span(entry_span(entry))
                    })?;
                    value.parse::<IpNet>().map_err(|_| {
                        ConfigurationValidationError::from(format!(
                            "Invalid `allowlist` — invalid IP or CIDR `{value}`"
                        ))
                        .with_span(entry_span(entry))
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
    ) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
        for directive_name in block.directives.keys() {
            if !THRESHOLD_DIRECTIVES.contains(&directive_name.as_str()) {
                let entry = block.directives[directive_name.as_str()]
                    .first()
                    .expect("non-empty block directives");
                return Err(ConfigurationValidationError::from(format!(
                    "Invalid `{directive_name}` — unknown directive in {threshold_name} block"
                ))
                .with_span(entry_span(entry)));
            }
        }

        let events_entry = block.directives.get("events");
        if events_entry.is_none() {
            return Err(ConfigurationValidationError::from(format!(
                "Invalid `{threshold_name}` — missing required `events` directive"
            )));
        }

        for entry in events_entry.into_iter().flatten() {
            self.validate_number_entry(entry, "events", 1)?;
        }

        let window_entry = block.directives.get("window");
        if window_entry.is_none() {
            return Err(ConfigurationValidationError::from(format!(
                "Invalid `{threshold_name}` — missing required `window` directive"
            )));
        }

        for entry in window_entry.into_iter().flatten() {
            self.validate_duration_entry(entry, "window")?;
        }

        Ok(())
    }

    fn validate_error_rate_threshold_block(
        &self,
        block: &ServerConfigurationBlock,
    ) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
        for directive_name in block.directives.keys() {
            if !ERROR_RATE_THRESHOLD_DIRECTIVES.contains(&directive_name.as_str()) {
                let entry = block.directives[directive_name.as_str()]
                    .first()
                    .expect("non-empty block directives");
                return Err(ConfigurationValidationError::from(format!(
                    "Invalid `{directive_name}` — unknown directive in error_rate_threshold block"
                ))
                .with_span(entry_span(entry)));
            }
        }

        if let Some(entries) = block.directives.get("events") {
            for entry in entries {
                self.validate_number_entry(entry, "events", 1)?;
            }
        }

        if let Some(entries) = block.directives.get("window") {
            for entry in entries {
                self.validate_duration_entry(entry, "window")?;
            }
        }

        if let Some(entries) = block.directives.get("status_codes") {
            for entry in entries {
                if entry.args.is_empty() {
                    return Err(ConfigurationValidationError::from(
                        "Invalid `status_codes` — expected at least one status code",
                    )
                    .with_span(entry_span(entry)));
                }
                for arg in &entry.args {
                    let value = arg.as_str().ok_or_else(|| {
                        ConfigurationValidationError::from(
                            "Invalid `status_codes` — expected string status code values",
                        )
                        .with_span(entry_span(entry))
                    })?;
                    let code: u16 = value.parse().map_err(|_| {
                        ConfigurationValidationError::from(format!(
                            "Invalid `status_codes` — invalid status code `{value}`"
                        ))
                        .with_span(entry_span(entry))
                    })?;
                    if !(100..=599).contains(&code) {
                        return Err(ConfigurationValidationError::from(format!(
                            "Invalid `status_codes` — status code `{value}` must be between 100 and 599"
                        ))
                        .with_span(entry_span(entry)));
                    }
                }
            }
        }

        Ok(())
    }

    fn validate_number_entry(
        &self,
        entry: &ServerConfigurationDirectiveEntry,
        name: &str,
        min: i64,
    ) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
        let value = entry.args.first().ok_or_else(|| {
            ConfigurationValidationError::from(format!(
                "Invalid `{name}` — must be an integer value"
            ))
            .with_span(entry_span(entry))
        })?;
        let n = value.as_number().ok_or_else(|| {
            ConfigurationValidationError::from(format!(
                "Invalid `{name}` — must be an integer value"
            ))
            .with_span(entry_span(entry))
        })?;
        if n < min {
            return Err(ConfigurationValidationError::from(format!(
                "Invalid `{name}` — must be >= {min}"
            ))
            .with_span(entry_span(entry)));
        }
        Ok(())
    }

    fn validate_duration_entry(
        &self,
        entry: &ServerConfigurationDirectiveEntry,
        name: &str,
    ) -> Result<(), ferron_core::config::validator::ConfigurationValidationError> {
        let arg = entry.args.first().ok_or_else(|| {
            ConfigurationValidationError::from(format!(
                "Invalid `{name}` — must be a duration value"
            ))
            .with_span(entry_span(entry))
        })?;
        arg.as_duration().ok_or_else(|| {
            ConfigurationValidationError::from(format!(
                "Invalid `{name}` — must be a duration value"
            ))
            .with_span(entry_span(entry))
        })?;

        Ok(())
    }
}
