use std::collections::HashMap;
use std::str::FromStr;
use std::time::Duration;

use ferron_core::config::ServerConfigurationBlock;

pub struct TlsHttpConfig {
    pub url: hyper::Uri,
    pub no_verification: bool,
    pub refresh_interval: std::time::Duration,
}

impl TlsHttpConfig {
    pub fn from_config(config: &ServerConfigurationBlock) -> anyhow::Result<Self> {
        let url = config
            .get_value("url")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .and_then(|v| hyper::Uri::from_str(v.as_str()).ok())
            .ok_or(anyhow::anyhow!(
                "`tls-http` module requires certificate URL endpoint"
            ))?;
        let refresh_interval = config
            .get_value("refresh_interval")
            .and_then(|v| v.as_duration())
            .unwrap_or(Duration::from_hours(1));
        let no_verification = config.get_flag("no_verification");
        Ok(Self {
            url,
            no_verification,
            refresh_interval,
        })
    }
}
