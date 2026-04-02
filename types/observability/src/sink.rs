use std::sync::Arc;

use crate::Event;

pub trait EventSink: Send + Sync {
    fn emit(&self, event: Event);
}

#[derive(Clone)]
pub struct CompositeEventSink {
    sinks: Vec<Arc<dyn EventSink>>,
}

impl CompositeEventSink {
    #[inline]
    pub fn new(sinks: Vec<Arc<dyn EventSink>>) -> Self {
        Self { sinks }
    }

    #[inline]
    pub fn add_sink(&mut self, sink: Arc<dyn EventSink>) {
        self.sinks.push(sink);
    }

    #[inline]
    pub fn emit(&self, event: Event) {
        for sink in &self.sinks {
            sink.emit(event.clone());
        }
    }
}
