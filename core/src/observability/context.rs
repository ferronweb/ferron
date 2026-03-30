use crate::observability::CompositeEventSink;

pub struct ObservabilityContext {
    pub sink: CompositeEventSink,
}
