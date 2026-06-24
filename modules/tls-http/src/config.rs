use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use ferron_core::config::ServerConfigurationBlock;

pub struct TlsHttpConfig {
    pub url: hyper::Uri,
    pub no_verification: bool,
    pub refresh_interval: std::time::Duration,
    pub on_demand: bool,
    pub on_demand_ask: Option<String>,
    pub on_demand_ask_auth: Option<String>,
    pub on_demand_ask_no_verification: bool,
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
        let on_demand = config.get_flag("on_demand");
        let on_demand_ask = first_value(config, "on_demand_ask");
        let on_demand_ask_auth = first_value(config, "on_demand_ask_auth");
        let on_demand_ask_no_verification = config.get_flag("on_demand_ask_no_verification");
        Ok(Self {
            url,
            no_verification,
            refresh_interval,
            on_demand,
            on_demand_ask,
            on_demand_ask_auth,
            on_demand_ask_no_verification,
        })
    }
}

#[derive(Clone)]
pub struct TlsHttpOnDemandConfigData {
    pub url: hyper::Uri,
    pub no_verification: bool,
    pub refresh_interval: Duration,
    pub on_demand_ask: Option<String>,
    pub on_demand_ask_auth: Option<String>,
    pub on_demand_ask_no_verification: bool,
    pub sni_hostname: Option<String>,
    pub port: u16,
    pub error_message: Arc<parking_lot::RwLock<Option<String>>>,
}

fn first_value(config: &ServerConfigurationBlock, name: &str) -> Option<String> {
    config
        .get_value(name)
        .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
}
