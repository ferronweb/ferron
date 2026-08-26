//! Logging and metric emission helpers.

use std::sync::Arc;

use ferron_observability::{
    CompositeEventSink, Event, LogAttributeValue, LogEvent, LogLevel, MetricAttributeValue,
    MetricEvent, MetricType, MetricValue,
};

pub(crate) fn emit_log(
    event_sink: &Option<Arc<CompositeEventSink>>,
    level: LogLevel,
    summary: &'static str,
    message: &str,
    target: &'static str,
    attributes: Vec<(&'static str, LogAttributeValue)>,
) {
    if let Some(ref sink) = event_sink {
        sink.emit(Event::Log(LogEvent {
            level,
            message: message.to_string(),
            summary: summary.into(),
            target,
            attributes,
            trace_context: None,
        }));
    }
}

pub(crate) fn emit_metric(
    event_sink: &Option<Arc<CompositeEventSink>>,
    name: &'static str,
    value: MetricValue,
    ty: MetricType,
    unit: Option<&'static str>,
    description: Option<&'static str>,
    attributes: Vec<(&'static str, MetricAttributeValue)>,
) {
    if let Some(ref sink) = event_sink {
        sink.emit(Event::Metric(MetricEvent {
            name,
            attributes,
            ty,
            value,
            unit,
            description,
            trace_context: None,
        }));
    }
}
