use std::str::FromStr;

use ferron_core::config::layer::LayeredConfiguration;
use http::HeaderName;
use typemap_rev::TypeMapKey;

pub struct TraceIdConfig {
    pub reflect_request: bool,
    pub header_name: HeaderName,
}

impl Default for TraceIdConfig {
    #[inline]
    fn default() -> Self {
        Self {
            reflect_request: false,
            header_name: HeaderName::from_static("x-ferron-trace-id"),
        }
    }
}

impl TraceIdConfig {
    #[inline]
    pub fn from_layered_config(layered_config: &LayeredConfiguration) -> Option<Self> {
        let trace_id_header_block = layered_config.get_entry("trace_id_header", true)?;

        if !trace_id_header_block.get_flag() {
            // `trace_id_header false`
            return None;
        }

        let mut config = Self::default();

        if trace_id_header_block
            .children
            .as_ref()
            .map_or(false, |tih| tih.get_flag("reflect_request"))
        {
            config.reflect_request = true;
        }
        if let Some(header_name) = trace_id_header_block
            .children
            .as_ref()
            .and_then(|tih| tih.get_value("header_name"))
            .and_then(|hn| hn.as_str())
            .and_then(|hns| HeaderName::from_str(hns).ok())
        {
            config.header_name = header_name;
        }

        Some(config)
    }
}

impl TypeMapKey for TraceIdConfig {
    type Value = Self;
}
