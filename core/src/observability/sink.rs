use std::sync::Arc;

use crate::observability::Event;

pub trait EventSink: Send + Sync {
    fn emit(&self, event: Event);
}

pub struct CompositeEventSink {
    sinks: Vec<Arc<dyn EventSink>>,
}

impl CompositeEventSink {
    pub fn new(sinks: Vec<Arc<dyn EventSink>>) -> Self {
        Self { sinks }
    }

    pub fn add_sink(&mut self, sink: Arc<dyn EventSink>) {
        self.sinks.push(sink);
    }

    pub fn emit(&self, event: Event) {
        for sink in &self.sinks {
            sink.emit(event.clone());
        }
    }
}
