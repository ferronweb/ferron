use std::sync::Arc;

use crate::Event;

pub trait EventSink: Send + Sync {
    fn emit(&self, event: Event);

    /// Emit an event shared via `Arc`. Override this to avoid cloning the full
    /// `Event` when your sink can work with a shared reference. The default
    /// implementation clones the event for backward compatibility.
    #[inline]
    fn emit_arc(&self, event: Arc<Event>) {
        let event = Arc::unwrap_or_clone(event);
        self.emit(event);
    }
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
        match self.sinks.len() {
            0 => {}
            1 => {
                self.sinks[0].emit(event);
            }
            _ => {
                // For multiple sinks, wrap in Arc so each sink can choose to clone or consume
                let event = Arc::new(event);
                for sink in &self.sinks {
                    sink.emit_arc(Arc::clone(&event));
                }
            }
        }
    }
}
