use crate::{
    observability::{Event, EventSink},
    Provider,
};

pub trait ObservabilityProvider: Provider {
    fn emit(&self, event: Event);
}

impl EventSink for dyn ObservabilityProvider {
    fn emit(&self, event: Event) {
        ObservabilityProvider::emit(self, event)
    }
}
