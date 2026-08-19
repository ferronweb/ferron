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
