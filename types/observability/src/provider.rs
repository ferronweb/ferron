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
