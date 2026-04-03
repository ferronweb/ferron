use std::sync::Arc;

use crate::{AccessEvent, Event, EventSink};
use ferron_core::{config::ServerConfigurationBlock, providers::Provider};

pub struct ObservabilityContext {
    pub event: Event,
    pub log_config: Arc<ServerConfigurationBlock>,
}

pub struct ObservabilityProviderEventSink {
    inner: Arc<dyn Provider<ObservabilityContext>>,
    log_config: Arc<ServerConfigurationBlock>,
}

impl ObservabilityProviderEventSink {
    #[inline]
    pub fn new(
        inner: Arc<dyn Provider<ObservabilityContext>>,
        log_config: Arc<ServerConfigurationBlock>,
    ) -> Self {
        Self { inner, log_config }
    }
}

impl EventSink for ObservabilityProviderEventSink {
    fn emit(&self, event: Event) {
        let mut ctx = ObservabilityContext {
            event,
            log_config: self.log_config.clone(),
        };
        let _ = self.inner.execute(&mut ctx);
    }
}

pub struct LogFormatterContext {
    pub access_event: Arc<dyn AccessEvent>,
    pub output: Option<String>,
}
