use std::sync::Arc;

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
    Histogram(Option<Vec<f64>>),
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

    /// Boolean value
    Bool(bool),

    /// Integer value
    I64(i64),

    /// Floating-point value
    F64(f64),
}

/// Represents a trace event with its name, attributes, and optional span ID.
#[derive(Clone)]
pub enum TraceEvent {
    /// Start a new span with the given name.
    StartSpan(String),
    /// End the span with the given name and optional error description.
    EndSpan(String, Option<String>),
}
