use std::collections::BTreeMap;
use std::sync::Arc;

use ferron_observability::baggage::{self, BaggageKeyPromotion, SignalSet};
use ferron_observability::{LogAttributeValue, LogEvent, LogLevel};
use opentelemetry::logs::AnyValue;

use crate::config::LogStyle;

use super::context::trace_flags;

pub(crate) fn emit_log(
    provider: &opentelemetry_sdk::logs::SdkLoggerProvider,
    event: &LogEvent,
    promotions: &[BaggageKeyPromotion],
    log_style: LogStyle,
    control_plane_metadata: &Option<Arc<BTreeMap<String, String>>>,
) {
    use opentelemetry::logs::{LogRecord, Logger, LoggerProvider, Severity};

    let logger = provider.logger("ferron");
    let mut record = logger.create_log_record();

    // In modern mode the log body is the short OTEL summary and per-event
    // attributes are published as typed AnyValues. In legacy mode the body is
    // the human-readable message and attributes are not exposed.
    match log_style {
        LogStyle::Legacy => {
            record.set_body(AnyValue::String(event.message.clone().into()));
            record.add_attribute("log.target", event.target);
        }
        LogStyle::Modern => {
            record.set_body(AnyValue::String(event.summary.as_ref().to_string().into()));
            record.add_attribute("log.target", event.target);
            for (key, value) in &event.attributes {
                record.add_attribute(*key, log_attribute_to_anyvalue(value));
            }
        }
    }

    record.set_severity_number(match event.level {
        LogLevel::Error => Severity::Error,
        LogLevel::Warn => Severity::Warn,
        LogLevel::Info => Severity::Info,
        LogLevel::Debug => Severity::Debug,
    });
    record.set_severity_text(match event.level {
        LogLevel::Error => "ERROR",
        LogLevel::Warn => "WARN",
        LogLevel::Info => "INFO",
        LogLevel::Debug => "DEBUG",
    });
    if let Some(trace_context) = &event.trace_context {
        if let (Ok(trace_id_str), Ok(span_id_str)) = (
            std::str::from_utf8(&trace_context.trace_id),
            std::str::from_utf8(&trace_context.span_id),
        ) {
            if let (Ok(trace_id), Ok(span_id)) = (
                opentelemetry::TraceId::from_hex(trace_id_str),
                opentelemetry::SpanId::from_hex(span_id_str),
            ) {
                record.set_trace_context(trace_id, span_id, trace_flags(trace_context.sampled));
            }
        }
    }

    // Promote configured baggage keys into log record attributes
    if let Some(baggage_str) = event
        .trace_context
        .as_ref()
        .and_then(|c| c.baggage.as_deref())
    {
        let extracted = baggage::extract_promoted_keys(baggage_str, promotions, SignalSet::LOGS);
        for attr in extracted {
            record.add_attribute(attr.attribute_name, AnyValue::String(attr.value.into()));
        }
    }

    // Inject control plane metadata as log record attributes
    // Prefer event-level metadata over provider-level metadata
    let effective_metadata = event
        .control_plane_metadata
        .as_ref()
        .or(control_plane_metadata.as_ref());
    if let Some(metadata) = effective_metadata {
        for (key, value) in metadata.iter() {
            let attr_key = format!("ferron.control_plane.{}", key);
            record.add_attribute(attr_key, AnyValue::String(value.clone().into()));
        }
    }

    logger.emit(record);
}

/// Convert a [`LogAttributeValue`] into an OTEL [`AnyValue`] preserving its
/// underlying type (string, bool, integer, float).
pub(crate) fn log_attribute_to_anyvalue(value: &LogAttributeValue) -> AnyValue {
    match value {
        LogAttributeValue::String(s) => AnyValue::String(s.clone().into()),
        LogAttributeValue::StaticStr(s) => AnyValue::String((*s).into()),
        LogAttributeValue::Bool(b) => AnyValue::Boolean(*b),
        LogAttributeValue::I64(i) => AnyValue::Int(*i),
        LogAttributeValue::F64(f) => AnyValue::Double(*f),
    }
}
