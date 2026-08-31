//! Event types for the observability subsystem.
//!
//! Every observable action in Ferron is represented as an [`Event`].
//! Sinks receive events and forward them to their respective backends
//! (logs to file, metrics to Prometheus, traces to OTLP, etc.).

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Size of a W3C traceparent trace ID (32 hex chars).
const TRACE_ID_LEN: usize = 32;
/// Size of a W3C traceparent span ID (16 hex chars).
const SPAN_ID_LEN: usize = 16;

pub use super::access::*;

/// A top-level observability event.
///
/// Modules construct and emit events through a [`CompositeEventSink`](crate::CompositeEventSink).
/// The sink dispatches each variant to the sinks that handle it.
#[derive(Clone)]
pub enum Event {
    /// A structured access log event (e.g. HTTP request/response).
    Access(Arc<dyn AccessEvent>),
    /// A traditional application log message.
    Log(LogEvent),
    /// A numeric metric (counter, gauge, histogram).
    Metric(MetricEvent),
    /// A distributed trace event (span start or end).
    Trace(TraceEvent),
}

/// A traditional full-text log message (used by `console` and `file` sinks, and by
/// OTLP `log_style legacy`).
///
/// In OTLP `log_style modern`, the `summary` field is used as the log body and
/// `attributes` are emitted as typed OpenTelemetry attributes instead.
#[derive(Clone)]
pub struct LogEvent {
    /// Severity level of this log message.
    pub level: LogLevel,
    /// Traditional full-text message. Always rendered by `console` and `file`
    /// sinks and by OTLP `log_style legacy`.
    pub message: String,
    /// Short summary used by OTLP `log_style modern` as the log body.
    pub summary: Cow<'static, str>,
    /// Module or component that produced this log (e.g. `"ferron_http_server"`).
    pub target: &'static str,
    /// Typed structured attributes. Emitted as OpenTelemetry log record
    /// attributes in OTLP `log_style modern`. Ignored by other sinks.
    pub attributes: Vec<(&'static str, LogAttributeValue)>,
    /// Optional trace context for correlating with trace events.
    pub trace_context: Option<EventTraceContext>,
}

/// Represents an attribute value for a log record.
/// Mirrors OTEL semantic convention attribute types.
#[derive(Clone, Debug, PartialEq)]
pub enum LogAttributeValue {
    /// String value
    String(String),

    /// Static string value (zero allocation)
    StaticStr(&'static str),

    /// Boolean value
    Bool(bool),

    /// Integer value
    I64(i64),

    /// Floating-point value
    F64(f64),
}

/// Log severity level.
#[derive(Copy, Clone)]
pub enum LogLevel {
    /// Error: something failed and requires attention.
    Error,
    /// Warn: something unexpected but non-fatal happened.
    Warn,
    /// Info: normal operational messages.
    Info,
    /// Debug: detailed diagnostic information.
    Debug,
}

/// Represents a metric with its name, attributes, and value.
#[derive(Clone)]
pub struct MetricEvent {
    /// Name of the metric
    pub name: &'static str,
    /// Attributes of the metric
    pub attributes: Vec<(&'static str, MetricAttributeValue)>,
    /// Type of the metric
    pub ty: MetricType,
    /// Value of the metric
    pub value: MetricValue,
    /// Optional unit of the metric
    pub unit: Option<&'static str>,
    /// Optional description of the metric
    pub description: Option<&'static str>,
    /// Optional trace context for the metric, useful for correlating with trace events.
    pub trace_context: Option<EventTraceContext>,
}

/// Represents a type of metric.
#[derive(Clone, Debug, PartialEq)]
pub enum MetricType {
    /// Increasing counter
    Counter,

    /// Gauge
    Gauge,

    /// Increasing or decreasing counter
    UpDownCounter,

    /// Histogram with optional buckets
    Histogram(Option<Cow<'static, [f64]>>),
}

/// A metric value.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum MetricValue {
    /// Floating-point value.
    F64(f64),
    /// Unsigned integer value.
    U64(u64),
    /// Signed integer value (for up-down counters).
    I64(i64),
}

/// Represents an attribute value for a metric.
#[derive(Clone, Debug, PartialEq)]
pub enum MetricAttributeValue {
    /// String value
    String(String),

    /// Static string value (zero allocation)
    StaticStr(&'static str),

    /// Boolean value
    Bool(bool),

    /// Integer value
    I64(i64),

    /// Floating-point value
    F64(f64),
}

/// Represents an attribute value for a trace span.
/// Mirrors OTEL semantic convention attribute types.
#[derive(Clone, Debug, PartialEq)]
pub enum TraceAttributeValue {
    /// String value
    String(String),

    /// Static string value (zero allocation)
    StaticStr(&'static str),

    /// Boolean value
    Bool(bool),

    /// Integer value
    I64(i64),

    /// Floating-point value
    F64(f64),
}

/// W3C trace context attached to an event.
///
/// Carries trace and span IDs as raw byte arrays (not hex-encoded) for
/// efficient storage. The hex encoding is done only when emitting to
/// backends that require string representations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventTraceContext {
    /// 32-byte W3C trace ID (hex-encoded as 32 chars).
    pub trace_id: [u8; TRACE_ID_LEN],
    /// 16-byte W3C parent span ID (hex-encoded as 16 chars).
    pub span_id: [u8; SPAN_ID_LEN],
    /// Baggage associated with the event.
    pub baggage: Option<String>,
    /// Whether the trace was sampled, if known.
    pub sampled: Option<bool>,
}

/// Identifies a span to link to, optionally with attributes.
///
/// Span links connect causally related spans that do not have a direct
/// parent-child relationship. For example, a control plane event that
/// triggers multiple data plane requests would link to each resulting
/// request span.
#[derive(Clone, Debug)]
pub struct SpanLink {
    /// The trace ID of the linked span (32 hex chars).
    pub trace_id: String,
    /// The span ID of the linked span (16 hex chars).
    pub span_id: String,
    /// Whether the linked span was sampled.
    pub sampled: Option<bool>,
    /// Attributes describing the relationship.
    pub attributes: Vec<(String, TraceAttributeValue)>,
}

/// Identifies a span's parent, either by lookup key or by explicit trace/span IDs.
#[derive(Clone)]
pub enum Parent {
    /// Reference a parent span by a logical key (resolved at emission time).
    ByKey(String),
    /// Reference a parent span by explicit trace and span IDs.
    ById {
        /// The trace ID of the parent span (32 hex chars).
        trace_id: String,
        /// The span ID of the parent span (16 hex chars).
        span_id: String,
        /// Whether the parent span was sampled.
        sampled: Option<bool>,
        /// W3C baggage associated with the parent span.
        baggage: Option<String>,
    },
}

/// A distributed trace event: either start or end a span.
#[derive(Clone)]
pub enum TraceEvent {
    /// Start a new span with the given name, optional parent, and attributes.
    ///
    /// `builder_attributes` are set on the `SpanBuilder` **before** the span is
    /// built, making them visible to the sampler. `attributes` are set **after**
    /// the span is built and are not visible to the sampler.
    /// `links` connect this span to causally related spans without a
    /// parent-child relationship.
    StartSpan {
        /// Unique key identifying this span (used to match StartSpan/EndSpan).
        key: Cow<'static, str>,
        /// Human-readable span name (e.g. `"GET /api/users"`).
        name: Cow<'static, str>,
        /// Parent span reference, if any.
        parent: Option<Parent>,
        /// W3C trace context for this span.
        trace_context: Option<EventTraceContext>,
        /// Attributes set on the SpanBuilder before building (visible to the sampler).
        builder_attributes: Vec<(Cow<'static, str>, TraceAttributeValue)>,
        /// Attributes set after the span is built (not visible to the sampler).
        attributes: Vec<(&'static str, TraceAttributeValue)>,
        /// Links to causally related spans.
        links: Vec<SpanLink>,
        /// Control plane metadata to include as `ferron.control_plane.*` attributes.
        control_plane_metadata: Option<Arc<BTreeMap<String, String>>>,
    },
    /// End the span with the given name, optional error description, and final attributes.
    /// Attributes here are merged with those from StartSpan and are useful for values
    /// only known at response time (e.g. `http.response.status_code`).
    EndSpan {
        /// Unique key identifying this span (must match the corresponding StartSpan).
        key: Cow<'static, str>,
        /// Human-readable span name (should match the corresponding StartSpan).
        name: Cow<'static, str>,
        /// Optional error description if the span ended with a failure.
        error: Option<String>,
        /// Final attributes merged with those from StartSpan (e.g. response status code).
        attributes: Vec<(&'static str, TraceAttributeValue)>,
        /// Control plane metadata to include as `ferron.control_plane.*` attributes.
        control_plane_metadata: Option<Arc<BTreeMap<String, String>>>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_attribute_value_partial_eq() {
        assert_eq!(LogAttributeValue::Bool(true), LogAttributeValue::Bool(true));
        assert_ne!(
            LogAttributeValue::Bool(true),
            LogAttributeValue::Bool(false)
        );
        assert_eq!(LogAttributeValue::I64(42), LogAttributeValue::I64(42));
        assert_eq!(
            LogAttributeValue::String("x".to_string()),
            LogAttributeValue::String("x".to_string())
        );
        assert_eq!(
            LogAttributeValue::StaticStr("x"),
            LogAttributeValue::StaticStr("x")
        );
        assert_eq!(LogAttributeValue::F64(1.5), LogAttributeValue::F64(1.5));
    }

    #[test]
    fn log_event_round_trip_struct_literal() {
        let event = LogEvent {
            level: LogLevel::Info,
            message: "full text message".to_string(),
            summary: "short".into(),
            target: "ferron-test",
            attributes: vec![(
                "client.address",
                LogAttributeValue::String("127.0.0.1".to_string()),
            )],
            trace_context: None,
        };
        assert_eq!(event.message, "full text message");
        assert_eq!(event.summary, "short");
        assert_eq!(event.target, "ferron-test");
        assert_eq!(event.attributes.len(), 1);
        assert_eq!(
            event.attributes[0],
            (
                "client.address",
                LogAttributeValue::String("127.0.0.1".to_string())
            )
        );
    }
}
