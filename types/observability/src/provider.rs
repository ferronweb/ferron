use std::collections::BTreeMap;
use std::sync::Arc;

use crate::{AccessEvent, EventSink, LogEvent};
use ferron_core::config::ServerConfigurationBlock;

pub struct ObservabilityContext {
    pub log_config: Arc<ServerConfigurationBlock>,
    pub sink: Option<Arc<dyn EventSink>>,
    /// Control plane metadata to include in observability signals
    /// (from `control_plane { metadata { ... } }`).
    pub control_plane_metadata: Option<Arc<BTreeMap<String, String>>>,
}

impl ObservabilityContext {
    pub fn new(log_config: Arc<ServerConfigurationBlock>) -> Self {
        Self {
            log_config,
            sink: None,
            control_plane_metadata: None,
        }
    }
}

pub struct LogFormatterContext {
    pub access_event: Arc<dyn AccessEvent>,
    pub log_config: Arc<ServerConfigurationBlock>,
    pub output: Option<String>,
}

pub struct ApplicationLogFormatterContext<'a> {
    pub log_event: &'a LogEvent,
    pub log_config: Arc<ServerConfigurationBlock>,
    pub output: Option<String>,
}
