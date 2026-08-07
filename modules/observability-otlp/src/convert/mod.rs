mod access_log;
mod context;
mod logs;
mod metrics;
mod resource;
mod traces;

use std::time::{SystemTime, UNIX_EPOCH};

use crate::proto::opentelemetry::proto::common::v1::{any_value, AnyValue, KeyValue};

#[cfg(test)]
pub(crate) use access_log::{build_access_log_record, OtelAccessAttributeVisitor};
#[cfg(test)]
pub(crate) use context::CorrelationContext;
#[cfg(test)]
pub(crate) use logs::build_log_record;
#[cfg(test)]
pub(crate) use metrics::{metric_key_values, sanitize_label_value};
#[cfg(test)]
pub(crate) use resource::build_resource;
#[cfg(test)]
pub(crate) use traces::{end_span, start_span};

/// Convert a [`SystemTime`] into UNIX epoch nanoseconds.
pub(crate) fn nanos(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

/// Build a typed OTLP key-value pair.
pub(crate) fn kv(key: impl Into<String>, value: AnyValue) -> KeyValue {
    KeyValue {
        key: key.into(),
        value: Some(value),
        ..Default::default()
    }
}

/// Wrap a string into an OTLP `AnyValue`.
pub(crate) fn any_string(value: impl Into<String>) -> AnyValue {
    AnyValue {
        value: Some(any_value::Value::StringValue(value.into())),
    }
}

/// Wrap a boolean into an OTLP `AnyValue`.
pub(crate) fn any_bool(value: bool) -> AnyValue {
    AnyValue {
        value: Some(any_value::Value::BoolValue(value)),
    }
}

/// Wrap an integer into an OTLP `AnyValue`.
pub(crate) fn any_int(value: i64) -> AnyValue {
    AnyValue {
        value: Some(any_value::Value::IntValue(value)),
    }
}

/// Wrap a float into an OTLP `AnyValue`.
pub(crate) fn any_double(value: f64) -> AnyValue {
    AnyValue {
        value: Some(any_value::Value::DoubleValue(value)),
    }
}
