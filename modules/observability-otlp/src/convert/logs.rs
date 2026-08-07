use std::time::SystemTime;

use ferron_observability::baggage::{self, BaggageKeyPromotion, SignalSet};
use ferron_observability::{LogAttributeValue, LogEvent, LogLevel};

use crate::config::LogStyle;
use crate::proto::opentelemetry::proto::common::v1::KeyValue;
use crate::proto::opentelemetry::proto::logs::v1::{LogRecord, SeverityNumber};

use super::context::{decode_span_id, decode_trace_id};
use super::{any_bool, any_double, any_int, any_string, kv, nanos};

/// Build an OTLP log record from a domain log event.
///
/// In modern mode the log body is the short OTEL summary and per-event
/// attributes are published as typed values. In legacy mode the body is the
/// human-readable message and the attributes are not exposed (only the
/// `log.target` attribute is kept).
pub(crate) fn build_log_record(
    event: &LogEvent,
    promotions: &[BaggageKeyPromotion],
    log_style: LogStyle,
    now: SystemTime,
) -> LogRecord {
    let mut attrs: Vec<KeyValue> = Vec::with_capacity(event.attributes.len() + 2);
    attrs.push(kv("log.target", any_string(event.target)));

    let body = match log_style {
        LogStyle::Legacy => any_string(&event.message),
        LogStyle::Modern => {
            for (key, value) in &event.attributes {
                attrs.push(log_kv(key, value));
            }
            any_string(event.summary.as_ref())
        }
    };

    let (severity_number, severity_text) = match event.level {
        LogLevel::Error => (SeverityNumber::Error as i32, "ERROR"),
        LogLevel::Warn => (SeverityNumber::Warn as i32, "WARN"),
        LogLevel::Info => (SeverityNumber::Info as i32, "INFO"),
        LogLevel::Debug => (SeverityNumber::Debug as i32, "DEBUG"),
    };

    // A log record only carries a trace context when both IDs decode.
    let (mut trace_id, mut span_id, mut flags) = (Vec::new(), Vec::new(), 0u32);
    if let Some(trace_context) = &event.trace_context {
        if let (Some(t), Some(s)) = (
            decode_trace_id(&trace_context.trace_id),
            decode_span_id(&trace_context.span_id),
        ) {
            trace_id = t;
            span_id = s;
            flags = u32::from(trace_context.sampled.unwrap_or(false));
        }
    }

    // Promote configured baggage keys into log record attributes.
    if let Some(baggage_str) = event
        .trace_context
        .as_ref()
        .and_then(|c| c.baggage.as_deref())
    {
        let extracted = baggage::extract_promoted_keys(baggage_str, promotions, SignalSet::LOGS);
        for attr in extracted {
            attrs.push(kv(attr.attribute_name, any_string(attr.value)));
        }
    }

    let timestamp = nanos(now);
    LogRecord {
        time_unix_nano: timestamp,
        observed_time_unix_nano: timestamp,
        severity_number,
        severity_text: severity_text.to_string(),
        body: Some(body),
        attributes: attrs,
        dropped_attributes_count: 0,
        flags,
        trace_id,
        span_id,
        event_name: String::new(),
    }
}

/// Convert a [`LogAttributeValue`] into an OTLP key-value preserving its
/// underlying type (string, bool, integer, float).
fn log_kv(key: &'static str, value: &LogAttributeValue) -> KeyValue {
    match value {
        LogAttributeValue::String(s) => kv(key, any_string(s)),
        LogAttributeValue::StaticStr(s) => kv(key, any_string(*s)),
        LogAttributeValue::Bool(b) => kv(key, any_bool(*b)),
        LogAttributeValue::I64(i) => kv(key, any_int(*i)),
        LogAttributeValue::F64(f) => kv(key, any_double(*f)),
    }
}
