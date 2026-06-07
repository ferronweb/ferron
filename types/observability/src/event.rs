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

/// A traditional full-text log message (used by `console` and `file` sinks, and by
/// OTLP `log_style legacy`).
///
/// In OTLP `log_style modern`, the `summary` field is used as the log body and
/// `attributes` are emitted as typed OpenTelemetry attributes instead.
#[derive(Clone)]
pub struct LogEvent {
    pub level: LogLevel,
    /// Traditional full-text message. Always rendered by `console` and `file`
    /// sinks and by OTLP `log_style legacy`.
    pub message: String,
    /// Short summary used by OTLP `log_style modern` as the log body.
    pub summary: Cow<'static, str>,
    pub target: &'static str, // "where this log came from"
    /// Typed structured attributes. Emitted as OpenTelemetry log record
    /// attributes in OTLP `log_style modern`. Ignored by other sinks.
    pub attributes: Vec<(&'static str, LogAttributeValue)>,
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
    /// Baggage associated with the event.
    pub baggage: Option<String>,
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
        baggage: Option<String>,
    },
}

#[derive(Clone)]
pub enum TraceEvent {
    /// Start a new span with the given name, optional parent, and attributes.
    ///
    /// `builder_attributes` are set on the `SpanBuilder` **before** the span is
    /// built, making them visible to the sampler. `attributes` are set **after**
    /// the span is built and are not visible to the sampler.
    StartSpan {
        key: Cow<'static, str>,
        name: Cow<'static, str>,
        parent: Option<Parent>,
        trace_context: Option<EventTraceContext>,
        builder_attributes: Vec<(Cow<'static, str>, TraceAttributeValue)>,
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
