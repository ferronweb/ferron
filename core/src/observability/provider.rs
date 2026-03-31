use crate::observability::{Event, EventSink};

pub trait ObservabilityProvider {
    fn emit(&self, event: Event);
}

impl EventSink for dyn ObservabilityProvider + Send + Sync {
    fn emit(&self, event: Event) {
        ObservabilityProvider::emit(self, event)
    }
}
