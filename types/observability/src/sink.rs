use std::sync::Arc;

use crate::sampler::TraceSampler;
use crate::{Event, TraceEvent};

/// A sink that receives and processes observability events.
///
/// Implement this trait to create a new observability backend (console,
/// file, OTLP, Prometheus, etc.). The server calls [`emit`](EventSink::emit)
/// for every event that should be processed by this sink.
///
/// Override [`emit_arc`](EventSink::emit_arc) when your sink can work with
/// a shared reference to avoid cloning the full `Event` for multi-sink dispatch.
pub trait EventSink: Send + Sync {
    /// Receive and process a single event.
    fn emit(&self, event: Event);

    /// Emit an event shared via `Arc`. Override this to avoid cloning the full
    /// `Event` when your sink can work with a shared reference. The default
    /// implementation clones the event.
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

/// An event sink that dispatches events to multiple inner sinks.
///
/// The composite sink is the per-host event hub. It evaluates trace sampling
/// before dispatching and uses `Arc` wrapping to avoid cloning events when
/// multiple sinks are registered.
#[derive(Clone)]
pub struct CompositeEventSink {
    sinks: Vec<Arc<dyn EventSink>>,
    /// Cached flag: whether any sink processes `Event::Trace` events.
    has_trace_sinks: bool,
    /// Cached flag: whether any sink processes `Event::Access` events.
    has_access_sinks: bool,
    /// Optional trace sampler applied before dispatching trace events.
    trace_sampler: Option<TraceSampler>,
}

impl CompositeEventSink {
    /// Create a new composite sink without a trace sampler.
    #[inline]
    pub fn new(sinks: Vec<Arc<dyn EventSink>>) -> Self {
        let has_trace_sinks = sinks.iter().any(|s| s.processes_traces());
        let has_access_sinks = sinks.iter().any(|s| s.processes_access());
        Self {
            sinks,
            has_trace_sinks,
            has_access_sinks,
            trace_sampler: None,
        }
    }

    /// Create a new composite sink with an optional trace sampler.
    ///
    /// When a sampler is provided, `Event::Trace` events are evaluated against
    /// it before being dispatched to individual sinks. Events that are not
    /// sampled are silently dropped.
    #[inline]
    pub fn with_sampler(sinks: Vec<Arc<dyn EventSink>>, sampler: Option<TraceSampler>) -> Self {
        let has_trace_sinks = sinks.iter().any(|s| s.processes_traces());
        let has_access_sinks = sinks.iter().any(|s| s.processes_access());
        Self {
            sinks,
            has_trace_sinks,
            has_access_sinks,
            trace_sampler: sampler,
        }
    }

    /// Add an event sink to the composite sink.
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

    /// Returns `true` if no sinks are registered.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.sinks.is_empty()
    }

    /// Returns a reference to the trace sampler, if one is configured.
    #[inline]
    pub fn trace_sampler(&self) -> Option<&TraceSampler> {
        self.trace_sampler.as_ref()
    }

    /// Emit an event to all registered sinks.
    ///
    /// Returns `true` if the event was dispatched (or dropped by sampling),
    /// `false` if the event was dropped due to sampling.
    #[inline]
    pub fn emit(&self, event: Event) -> bool {
        // Apply trace sampling before dispatching
        if let Event::Trace(ref trace_event) = event {
            if let Some(sampler) = &self.trace_sampler {
                let should_sample = match trace_event {
                    TraceEvent::StartSpan {
                        trace_context,
                        parent,
                        builder_attributes,
                        ..
                    } => {
                        let trace_id = trace_context.as_ref().map(|tc| &tc.trace_id);
                        let parent_ref = parent.as_ref();
                        let attrs: Vec<(&str, &crate::TraceAttributeValue)> = builder_attributes
                            .iter()
                            .map(|(k, v)| (k.as_ref(), v))
                            .collect();
                        sampler.should_sample(parent_ref, trace_id, &attrs)
                    }
                    TraceEvent::EndSpan { .. } => true, // Observability backends would discard it if StartSpan isn't sent...
                };
                if !should_sample {
                    return false;
                }
            }
        }

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
        };
        true
    }
}

impl Default for CompositeEventSink {
    #[inline]
    fn default() -> Self {
        Self::new(vec![])
    }
}
