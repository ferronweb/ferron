//! Configuration for abuse protection.

use cidr::IpCidr;
use ferron_core::config::ServerConfigurationBlock;
use ferron_http::abuse::AbuseEventType;

use crate::registry::{AbuseRegistryConfig, ErrorRateThresholdConfig, EventThreshold};

/// Parsed abuse protection configuration from ferron.conf.
#[derive(Debug, Clone, Default)]
pub struct AbuseProtectionConfig {
    pub registry_config: AbuseRegistryConfig,
}

/// Parse `abuse_protection { }` directive from configuration.
///
/// Example configuration:
/// ```ferron
/// abuse_protection {
///     ban_duration 15
///
///     rate_limit_threshold {
///         events 5
///         window 300
///     }
///
///     brute_force_threshold {
///         events 3
///         window 120
///     }
/// }
/// ```
pub fn parse_abuse_protection_config(
    config: &ferron_core::config::layer::LayeredConfiguration,
) -> Option<AbuseProtectionConfig> {
    let entries = config.get_entries("abuse_protection", true);

    if entries.is_empty() {
        return None;
    }

    let entry = entries[0];
    if !entry.get_flag() {
        return Some(AbuseProtectionConfig {
            registry_config: AbuseRegistryConfig {
                enabled: false,
                ..Default::default()
            },
        });
    }

    if let Some(children) = &entry.children {
        parse_abuse_protection_block(children)
            .ok()
            .map(|registry_config| AbuseProtectionConfig { registry_config })
    } else {
        None
    }
}

fn parse_abuse_protection_block(
    block: &ServerConfigurationBlock,
) -> Result<AbuseRegistryConfig, String> {
    let ban_duration_secs = block
        .get_value("ban_duration")
        .and_then(|v| v.as_duration())
        .map(|d| d.as_secs())
        .unwrap_or(AbuseRegistryConfig::DEFAULT_BAN_DURATION_SECS);

    let mut thresholds = Vec::new();

    if let Some(entries) = block.directives.get("rate_limit_threshold") {
        for entry in entries {
            if let Some(children) = &entry.children {
                if let Some(threshold) =
                    parse_threshold_block(AbuseEventType::RateLimitExceeded, children)?
                {
                    thresholds.push(threshold);
                }
            }
        }
    }

    if let Some(entries) = block.directives.get("brute_force_threshold") {
        for entry in entries {
            if let Some(children) = &entry.children {
                if let Some(threshold) =
                    parse_threshold_block(AbuseEventType::BruteForceFailure, children)?
                {
                    thresholds.push(threshold);
                }
            }
        }
    }

    if let Some(entries) = block.directives.get("custom_threshold") {
        for entry in entries {
            if let Some(children) = &entry.children {
                if let Some(threshold) = parse_threshold_block(AbuseEventType::Custom, children)? {
                    thresholds.push(threshold);
                }
            }
        }
    }

    // Parse error_rate_threshold blocks
    let mut error_rate_thresholds = Vec::new();
    if let Some(entries) = block.directives.get("error_rate_threshold") {
        for entry in entries {
            if let Some(children) = &entry.children {
                error_rate_thresholds.push(parse_error_rate_threshold_block(children)?);
            }
        }
    }

    if thresholds.is_empty() {
        thresholds = vec![
            EventThreshold::new(AbuseEventType::RateLimitExceeded, 5, 300),
            EventThreshold::new(AbuseEventType::BruteForceFailure, 3, 120),
        ];
    }

    let mut allowlist = Vec::new();

    if let Some(entries) = block.directives.get("allowlist") {
        for entry in entries {
            for arg in &entry.args {
                if let Some(s) = arg.as_str() {
                    if let Ok(cidr) = s.parse::<IpCidr>() {
                        allowlist.push(cidr);
                    }
                }
            }
        }
    }

    Ok(AbuseRegistryConfig {
        enabled: true,
        ban_duration_secs,
        thresholds,
        error_rate_thresholds,
        allowlist,
    })
}

fn parse_threshold_block(
    event_type: AbuseEventType,
    block: &ServerConfigurationBlock,
) -> Result<Option<EventThreshold>, String> {
    let events_count = block
        .get_value("events")
        .and_then(|v| v.as_number())
        .filter(|&n| n > 0)
        .ok_or_else(|| "threshold: 'events' must be a positive number".to_string())?
        as usize;

    let window_secs = block
        .get_value("window")
        .and_then(|v| v.as_duration())
        .map(|d| d.as_secs())
        .ok_or_else(|| "threshold: 'window' must be a duration".to_string())?;

    Ok(Some(EventThreshold::new(
        event_type,
        events_count,
        window_secs,
    )))
}

fn parse_error_rate_threshold_block(
    block: &ServerConfigurationBlock,
) -> Result<ErrorRateThresholdConfig, String> {
    let events_count = block
        .get_value("events")
        .and_then(|v| v.as_number())
        .filter(|&n| n > 0)
        .unwrap_or(50) as usize;

    let window_secs = block
        .get_value("window")
        .and_then(|v| v.as_duration())
        .map(|d| d.as_secs())
        .unwrap_or(60);

    let mut status_codes = Vec::new();
    if let Some(entries) = block.directives.get("status_codes") {
        for entry in entries {
            for arg in &entry.args {
                if let Some(s) = arg.as_str() {
                    let code: u16 = s
                        .parse()
                        .map_err(|_| format!("Invalid status code '{s}' — must be a number"))?;
                    if !(100..=599).contains(&code) {
                        return Err(format!(
                            "Invalid status code '{s}' — must be between 100 and 599"
                        ));
                    }
                    status_codes.push(code);
                }
            }
        }
    }

    if status_codes.is_empty() {
        status_codes = vec![404];
    }

    Ok(ErrorRateThresholdConfig::new(
        events_count,
        window_secs,
        status_codes,
    ))
}
