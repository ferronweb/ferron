use std::sync::Arc;

use crate::{Event, EventSink};
use ferron_core::providers::Provider;

pub struct ObservabilityContext {
    pub event: Event,
}

impl EventSink for dyn Provider<ObservabilityContext> {
    fn emit(&self, event: Event) {
        let mut ctx = ObservabilityContext { event };
        let _ = self.execute(&mut ctx);
    }
}

pub struct ObservabilityProviderEventSink {
    inner: Arc<dyn Provider<ObservabilityContext>>,
}

impl ObservabilityProviderEventSink {
    #[inline]
    pub fn new(inner: Arc<dyn Provider<ObservabilityContext>>) -> Self {
        Self { inner }
    }
}

impl EventSink for ObservabilityProviderEventSink {
    fn emit(&self, event: Event) {
        self.inner.emit(event)
    }
}
