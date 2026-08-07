use ferron_observability::EventTraceContext;

use crate::proto::opentelemetry::proto::trace::v1::Span;

/// Decode a 32-char hex-ASCII trace ID into its 16 raw bytes.
pub(crate) fn decode_trace_id(hex_ascii: &[u8]) -> Option<Vec<u8>> {
    decode_hex_id(hex_ascii, 16)
}

/// Decode a 16-char hex-ASCII span ID into its 8 raw bytes.
pub(crate) fn decode_span_id(hex_ascii: &[u8]) -> Option<Vec<u8>> {
    decode_hex_id(hex_ascii, 8)
}

fn decode_hex_id(hex_ascii: &[u8], expected_len: usize) -> Option<Vec<u8>> {
    let s = std::str::from_utf8(hex_ascii).ok()?;
    if s.len() != expected_len * 2 {
        return None;
    }
    let bytes = hex::decode(s).ok()?;
    if bytes.iter().all(|byte| *byte == 0) {
        // Zero IDs are invalid in OTLP.
        return None;
    }
    Some(bytes)
}

/// IDs requested for a new span via an incoming trace context.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RequestedIds {
    pub(crate) trace_id: Option<[u8; 16]>,
    pub(crate) span_id: Option<[u8; 8]>,
}

/// Extract requested span IDs from an event's trace context.
///
/// The event does not carry its own span object, so the incoming trace
/// context determines the span's own IDs (trace continuation).
pub(crate) fn parse_requested_ids(trace_context: &EventTraceContext) -> RequestedIds {
    RequestedIds {
        trace_id: decode_trace_id(&trace_context.trace_id).and_then(|v| v.try_into().ok()),
        span_id: decode_span_id(&trace_context.span_id).and_then(|v| v.try_into().ok()),
    }
}

/// Generate a random 16-byte trace ID (never all-zero).
pub(crate) fn generate_trace_id() -> [u8; 16] {
    loop {
        let id: [u8; 16] = rand::random();
        if id != [0; 16] {
            return id;
        }
    }
}

/// Generate a random 8-byte span ID (never all-zero).
pub(crate) fn generate_span_id() -> [u8; 8] {
    loop {
        let id: [u8; 8] = rand::random();
        if id != [0; 8] {
            return id;
        }
    }
}

/// A started but not yet finished span, tracked for parent resolution.
pub(crate) struct StoredSpan {
    pub(crate) span: Span,
    pub(crate) baggage: Option<String>,
}

/// Correlation context: tracks active spans per host sink instance.
pub struct CorrelationContext {
    /// Active spans: span_key -> started span
    active_spans: lru::LruCache<String, StoredSpan>,
}

impl CorrelationContext {
    pub fn new() -> Self {
        Self {
            // 65536 is always non-zero, so the conversion to NonZeroUsize
            // cannot panic.
            active_spans: lru::LruCache::new(65536.try_into().unwrap()),
        }
    }

    /// Store a started span under its key, evicting the oldest entry when the
    /// cache is full. Returns the evicted span, if any.
    pub fn insert_span(
        &mut self,
        key: impl Into<String>,
        stored: StoredSpan,
    ) -> Option<StoredSpan> {
        self.active_spans
            .push(key.into(), stored)
            .map(|(_, stored)| stored)
    }

    /// Remove and return the span stored under `key`, if any.
    pub(crate) fn remove_span(&mut self, key: &str) -> Option<StoredSpan> {
        self.active_spans.pop(key)
    }

    /// Look up an active span's IDs for use as a parent.
    pub fn get_parent_ids(&mut self, key: &str) -> Option<(Vec<u8>, Vec<u8>, Option<String>)> {
        self.active_spans.get(key).map(|stored| {
            (
                stored.span.trace_id.clone(),
                stored.span.span_id.clone(),
                stored.baggage.clone(),
            )
        })
    }

    /// Peek at the span stored under `key`, without removing it.
    pub fn get_span(&mut self, key: &str) -> Option<&Span> {
        self.active_spans.get(key).map(|stored| &stored.span)
    }
}

impl Default for CorrelationContext {
    fn default() -> Self {
        Self::new()
    }
}
