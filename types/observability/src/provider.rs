use crate::{Event, EventSink};
use ferron_core::Provider;

pub trait ObservabilityProvider: Provider {
    fn emit(&self, event: Event);
}

impl EventSink for dyn ObservabilityProvider {
    fn emit(&self, event: Event) {
        ObservabilityProvider::emit(self, event)
    }
}
