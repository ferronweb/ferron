use ferron_observability::MetricAttributeValue;

use crate::proto::opentelemetry::proto::common::v1::KeyValue;

use super::{any_bool, any_double, any_int, any_string, kv};

/// Sanitize a metric label value to prevent high-cardinality telemetry
/// poisoning.
///
/// Control characters are replaced with `?` to avoid log injection.
#[inline]
pub(crate) fn sanitize_label_value(s: &str) -> String {
    let s = s.trim();
    s.chars()
        .map(|c| if c.is_control() { '?' } else { c })
        .collect()
}

/// Convert metric event attributes into typed OTLP key-values, sanitizing
/// string label values to prevent telemetry poisoning.
#[inline]
pub(crate) fn metric_key_values(
    attributes: &[(&'static str, MetricAttributeValue)],
) -> Vec<KeyValue> {
    attributes
        .iter()
        .map(|(key, value)| metric_kv(key, value))
        .collect()
}

#[inline]
fn metric_kv(key: &'static str, value: &MetricAttributeValue) -> KeyValue {
    match value {
        MetricAttributeValue::String(s) => kv(key, any_string(sanitize_label_value(s))),
        MetricAttributeValue::StaticStr(s) => kv(key, any_string(*s)),
        MetricAttributeValue::Bool(b) => kv(key, any_bool(*b)),
        MetricAttributeValue::I64(i) => kv(key, any_int(*i)),
        MetricAttributeValue::F64(f) => kv(key, any_double(*f)),
    }
}
