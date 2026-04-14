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

    /// Returns `true` if this sink processes `Event::Trace` events.
    /// Used to skip expensive trace event construction when no sink will use them.
    #[inline]
    fn processes_traces(&self) -> bool {
        false
    }

    /// Returns `true` if this sink processes `Event::Access` events.
    /// Used to skip expensive header collection when no access log sink is configured.
    #[inline]
    fn processes_access(&self) -> bool {
        false
    }
}

#[derive(Clone)]
pub struct CompositeEventSink {
    sinks: Vec<Arc<dyn EventSink>>,
    /// Cached flag: whether any sink processes `Event::Trace` events.
    has_trace_sinks: bool,
    /// Cached flag: whether any sink processes `Event::Access` events.
    has_access_sinks: bool,
}

impl CompositeEventSink {
    #[inline]
    pub fn new(sinks: Vec<Arc<dyn EventSink>>) -> Self {
        let has_trace_sinks = sinks.iter().any(|s| s.processes_traces());
        let has_access_sinks = sinks.iter().any(|s| s.processes_access());
        Self {
            sinks,
            has_trace_sinks,
            has_access_sinks,
        }
    }

    #[inline]
    pub fn add_sink(&mut self, sink: Arc<dyn EventSink>) {
        if sink.processes_traces() {
            self.has_trace_sinks = true;
        }
        if sink.processes_access() {
            self.has_access_sinks = true;
        }
        self.sinks.push(sink);
    }

    /// Returns `true` if at least one sink processes trace events.
    /// When `false`, callers can skip expensive trace event construction.
    #[inline]
    pub fn has_trace_sinks(&self) -> bool {
        self.has_trace_sinks
    }

    /// Returns `true` if at least one sink processes access log events.
    /// When `false`, callers can skip expensive header collection for access logging.
    #[inline]
    pub fn has_access_sinks(&self) -> bool {
        self.has_access_sinks
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.sinks.is_empty()
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
