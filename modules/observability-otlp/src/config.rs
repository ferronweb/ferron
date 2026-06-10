use std::str::FromStr;

use ferron_core::config::ServerConfigurationBlock;
use ferron_observability::baggage::{BaggageKeyPromotion, SignalSet};

/// Log style for OTLP log records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogStyle {
    /// Legacy behavior: emit `event.message` as the log body verbatim and do not
    /// publish per-event attributes to the OTLP record. The configured `format`
    /// directive (if any) is honored as before.
    Legacy,
    /// Modern (OTEL-friendly) behavior: emit a short `summary` plus typed
    /// `attributes` instead of the human-readable `message`. The `format`
    /// directive is ignored for log records in this mode.
    #[default]
    Modern,
}

/// Parse a `log_style` directive value into a [`LogStyle`]. Returns `None` if
/// the value is not a recognized log style.
pub fn parse_log_style(value: &str) -> Option<LogStyle> {
    match value.to_ascii_lowercase().as_str() {
        "legacy" => Some(LogStyle::Legacy),
        "modern" => Some(LogStyle::Modern),
        _ => None,
    }
}

/// Per-host configuration for a single OTLP signal (logs, metrics, or traces)
pub struct SignalConfig {
    pub endpoint: String,
    pub protocol: String,
    pub authorization: Option<String>,
}

/// Shared configuration for an OTLP backend instance
pub struct OtlpBackendConfig {
    pub service_name: String,
    pub no_verify: bool,
    pub logs: Option<SignalConfig>,
    pub metrics: Option<SignalConfig>,
    pub traces: Option<SignalConfig>,
    pub baggage_promotions: Vec<BaggageKeyPromotion>,
    pub log_style: LogStyle,
    pub authorization: Option<String>,
}

impl OtlpBackendConfig {
    /// Parse the OTLP backend configuration from a ServerConfigurationBlock
    pub fn parse_config(config: &ServerConfigurationBlock) -> Self {
        let service_name = config
            .get_value("service_name")
            .and_then(|v| v.as_str())
            .unwrap_or("ferron")
            .to_string();

        let no_verify = config.get_flag("no_verification");

        let logs = SignalConfig::parse_config(config, "logs");
        let metrics = SignalConfig::parse_config(config, "metrics");
        let traces = SignalConfig::parse_config(config, "traces");
        let baggage_promotions = parse_baggage_promotions(config);
        let log_style = config
            .get_value("log_style")
            .and_then(|v| v.as_str())
            .and_then(parse_log_style)
            .unwrap_or_default();
        let authorization = config
            .get_value("authorization")
            .and_then(|v| v.as_str())
            .map(|v| v.to_string());

        Self {
            service_name,
            no_verify,
            logs,
            metrics,
            traces,
            baggage_promotions,
            log_style,
            authorization,
        }
    }
}

impl SignalConfig {
    /// Parse a single signal sub-block (logs, metrics, or traces)
    fn parse_config(parent: &ServerConfigurationBlock, name: &str) -> Option<SignalConfig> {
        let entries = parent.directives.get(name)?;
        let entry = entries.first()?;
        let endpoint = entry.args.first().and_then(|v| v.as_str())?.to_string();

        let default_protocol =
            if hyper::Uri::from_str(&endpoint).is_ok_and(|uri| uri.port_u16() == Some(4317)) {
                "grpc"
            } else {
                "http/protobuf"
            };

        let Some(children) = entry.children.as_ref() else {
            return Some(Self {
                endpoint,
                protocol: default_protocol.to_string(),
                authorization: None,
            });
        };

        let protocol = children
            .get_value("protocol")
            .and_then(|v| v.as_str())
            .unwrap_or(default_protocol)
            .to_string();

        let authorization = children
            .get_value("authorization")
            .and_then(|v| v.as_str())
            .map(|s: &str| s.to_string());

        Some(Self {
            endpoint,
            protocol,
            authorization,
        })
    }
}

/// Parse the `baggage` directive from the OTLP config block.
///
/// Expected format:
/// ```text
/// baggage {
///     key "tenant.id" {
///         attribute "tenant.id"
///         signals traces logs
///         max_distinct 1000
///     }
/// }
/// ```
fn parse_baggage_promotions(config: &ServerConfigurationBlock) -> Vec<BaggageKeyPromotion> {
    let Some(baggage_entries) = config.directives.get("baggage") else {
        return Vec::new();
    };
    let Some(baggage_block) = baggage_entries.first().and_then(|e| e.children.as_ref()) else {
        return Vec::new();
    };

    let Some(key_entries) = baggage_block.directives.get("key") else {
        return Vec::new();
    };

    let mut promotions = Vec::new();
    for key_entry in key_entries {
        let Some(baggage_key) = key_entry.args.first().and_then(|v| v.as_str()) else {
            continue;
        };

        let children = key_entry.children.as_ref();

        let attribute_name = children
            .and_then(|c| c.get_value("attribute"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let signals = children.and_then(parse_signal_set);

        let max_distinct = children
            .and_then(|c| c.get_value("max_distinct"))
            .and_then(|v| {
                if v.as_boolean().is_some_and(|v| !v) {
                    None
                } else {
                    Some(v.as_number().unwrap_or(100))
                }
            })
            .map(|n| n as usize);

        promotions.push(BaggageKeyPromotion {
            baggage_key: baggage_key.to_string(),
            attribute_name,
            signals,
            max_distinct,
        });
    }

    promotions
}

/// Parse a `signals` directive value into a SignalSet.
/// The value can be a single signal name or multiple args.
fn parse_signal_set(children: &ServerConfigurationBlock) -> Option<SignalSet> {
    let entries = children.directives.get("signals")?;
    let entry = entries.first()?;
    if entry.args.is_empty() {
        return None;
    }
    let mut set = SignalSet::empty();
    for arg in &entry.args {
        if let Some(name) = arg.as_str() {
            match name {
                "traces" => set = set.insert(SignalSet::TRACES),
                "logs" => set = set.insert(SignalSet::LOGS),
                "metrics" => set = set.insert(SignalSet::METRICS),
                _ => {}
            }
        }
    }
    if set == SignalSet::empty() {
        None
    } else {
        Some(set)
    }
}
