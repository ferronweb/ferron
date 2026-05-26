use ferron_core::config::ServerConfigurationBlock;

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

        Self {
            service_name,
            no_verify,
            logs,
            metrics,
            traces,
        }
    }
}

impl SignalConfig {
    /// Parse a single signal sub-block (logs, metrics, or traces)
    fn parse_config(parent: &ServerConfigurationBlock, name: &str) -> Option<SignalConfig> {
        let entries = parent.directives.get(name)?;
        let entry = entries.first()?;
        let endpoint = entry.args.first().and_then(|v| v.as_str())?.to_string();

        let children = entry.children.as_ref()?;

        let protocol = children
            .get_value("protocol")
            .and_then(|v| v.as_str())
            .unwrap_or("grpc")
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
