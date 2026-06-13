use std::sync::Arc;

use crate::{AccessEvent, EventSink, LogEvent};
use ferron_core::config::ServerConfigurationBlock;

pub struct ObservabilityContext {
    pub log_config: Arc<ServerConfigurationBlock>,
    pub sink: Option<Arc<dyn EventSink>>,
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
