use std::borrow::Cow;
use std::sync::Arc;

/// Size of a W3C traceparent trace ID (32 hex chars).
const TRACE_ID_LEN: usize = 32;
/// Size of a W3C traceparent span ID (16 hex chars).
const SPAN_ID_LEN: usize = 16;

pub use super::access::*;

#[derive(Clone)]
pub enum Event {
    Access(Arc<dyn AccessEvent>),
    Log(LogEvent),
    Metric(MetricEvent),
    Trace(TraceEvent),
}

#[derive(Clone)]
pub struct LogEvent {
    pub level: LogLevel,
    pub message: String,
    pub target: &'static str, // "where this log came from"
    pub trace_context: Option<EventTraceContext>,
}

#[derive(Copy, Clone)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
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

/// Represents a value for a metric.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum MetricValue {
    F64(f64),
    U64(u64),
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventTraceContext {
    pub trace_id: [u8; TRACE_ID_LEN],
    pub span_id: [u8; SPAN_ID_LEN],
    pub sampled: Option<bool>,
}

/// Represents a trace event with its name, attributes, and optional span ID.
#[derive(Clone)]
pub enum Parent {
    ByKey(String),
    ById {
        trace_id: String,
        span_id: String,
        sampled: Option<bool>,
    },
}

#[derive(Clone)]
pub enum TraceEvent {
    /// Start a new span with the given name, optional parent, and attributes.
    StartSpan {
        key: Cow<'static, str>,
        name: Cow<'static, str>,
        parent: Option<Parent>,
        trace_context: Option<EventTraceContext>,
        attributes: Vec<(&'static str, TraceAttributeValue)>,
    },
    /// End the span with the given name, optional error description, and final attributes.
    /// Attributes here are merged with those from StartSpan and are useful for values
    /// only known at response time (e.g. `http.response.status_code`).
    EndSpan {
        key: Cow<'static, str>,
        name: Cow<'static, str>,
        error: Option<String>,
        attributes: Vec<(&'static str, TraceAttributeValue)>,
    },
}
