use ferron_core::config::ServerConfigurationBlock;
use ferron_observability::baggage::{BaggageKeyPromotion, SignalSet};

/// Default StatsD server port.
pub const DEFAULT_STATSD_PORT: u16 = 8125;

/// Default StatsD server host.
pub const DEFAULT_STATSD_HOST: &str = "127.0.0.1";

/// Shared configuration for a StatsD backend instance
#[derive(Clone, Debug)]
pub struct StatsdBackendConfig {
    /// Hostname or IP address of the StatsD server.
    pub host: String,
    /// UDP port of the StatsD server.
    pub port: u16,
    /// Optional prefix prepended to every metric name with a `.` separator.
    pub prefix: Option<String>,
    /// Whether DogStatsD extensions (tags and the `h` histogram type) are enabled.
    pub datadog: bool,
    /// Baggage key promotions turned into DogStatsD tags for metric events.
    pub baggage_promotions: Vec<BaggageKeyPromotion>,
}

impl StatsdBackendConfig {
    /// Parse the StatsD backend configuration from a ServerConfigurationBlock
    pub fn parse_config(config: &ServerConfigurationBlock) -> Self {
        let host = config
            .get_value("host")
            .and_then(|v| v.as_str())
            .unwrap_or(DEFAULT_STATSD_HOST)
            .to_string();

        let port = config
            .get_value("port")
            .and_then(|v| v.as_number())
            .map(|n| n.clamp(1, u16::MAX as i64) as u16)
            .unwrap_or(DEFAULT_STATSD_PORT);

        let prefix = config
            .get_value("prefix")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        let datadog = config
            .get_value("datadog")
            .and_then(|v| v.as_boolean())
            .unwrap_or(false);

        let baggage_promotions = parse_baggage_promotions(config);

        Self {
            host,
            port,
            prefix,
            datadog,
            baggage_promotions,
        }
    }
}

/// Parse the `baggage` directive from the StatsD config block.
///
/// Each `key` entry promotes a W3C Baggage key into a DogStatsD tag. The
/// promoted key only applies to metrics, so the signal set is fixed to
/// `METRICS`.
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
            signals: Some(SignalSet::METRICS),
            max_distinct,
        });
    }

    promotions
}
