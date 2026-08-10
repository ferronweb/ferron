use ferron_core::config::ServerConfigurationBlock;

/// Default StatsD server port.
pub const DEFAULT_STATSD_PORT: u16 = 8125;

/// Default StatsD server host.
pub const DEFAULT_STATSD_HOST: &str = "127.0.0.1";

/// Shared configuration for a StatsD backend instance
#[derive(Clone, Debug, PartialEq)]
pub struct StatsdBackendConfig {
    /// Hostname or IP address of the StatsD server.
    pub host: String,
    /// UDP port of the StatsD server.
    pub port: u16,
    /// Optional prefix prepended to every metric name with a `.` separator.
    pub prefix: Option<String>,
    /// Whether DogStatsD extensions (tags and the `h` histogram type) are enabled.
    pub datadog: bool,
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

        Self {
            host,
            port,
            prefix,
            datadog,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use ferron_core::config::{
        ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationValue,
    };

    use super::*;

    fn block_with(directives: &[(&str, ServerConfigurationValue)]) -> ServerConfigurationBlock {
        let mut map = HashMap::new();
        for (name, value) in directives {
            map.insert(
                name.to_string(),
                vec![ServerConfigurationDirectiveEntry {
                    args: vec![value.clone()],
                    children: None,
                    span: None,
                }],
            );
        }
        ServerConfigurationBlock {
            directives: Arc::new(map),
            matchers: HashMap::new(),
            span: None,
        }
    }

    #[test]
    fn defaults_when_empty() {
        let block = block_with(&[]);
        let config = StatsdBackendConfig::parse_config(&block);
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 8125);
        assert_eq!(config.prefix, None);
        assert!(!config.datadog);
    }

    #[test]
    fn parses_all_directives() {
        let block = block_with(&[
            (
                "host",
                ServerConfigurationValue::String("statsd.example.com".to_string(), None),
            ),
            ("port", ServerConfigurationValue::Number(9125, None)),
            (
                "prefix",
                ServerConfigurationValue::String("myapp".to_string(), None),
            ),
            ("datadog", ServerConfigurationValue::Boolean(true, None)),
        ]);
        let config = StatsdBackendConfig::parse_config(&block);
        assert_eq!(config.host, "statsd.example.com");
        assert_eq!(config.port, 9125);
        assert_eq!(config.prefix.as_deref(), Some("myapp"));
        assert!(config.datadog);
    }

    #[test]
    fn empty_prefix_is_none() {
        let block = block_with(&[(
            "prefix",
            ServerConfigurationValue::String(String::new(), None),
        )]);
        let config = StatsdBackendConfig::parse_config(&block);
        assert_eq!(config.prefix, None);
    }

    #[test]
    fn out_of_range_port_is_clamped() {
        let block = block_with(&[("port", ServerConfigurationValue::Number(0, None))]);
        let config = StatsdBackendConfig::parse_config(&block);
        assert_eq!(config.port, 1);
    }
}
