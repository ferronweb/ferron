use ferron_observability::MetricAttributeValue;

use crate::proto::opentelemetry::proto::common::v1::KeyValue;

use super::{any_bool, any_double, any_int, any_string, kv};

/// Maximum length for a metric label value before it is hashed to prevent
/// cardinality explosion.
const MAX_LABEL_VALUE_LEN: usize = 128;

/// Sanitize a metric label value to prevent high-cardinality telemetry
/// poisoning.
///
/// Values longer than 128 characters are replaced with their hash.
/// Control characters are replaced with `?` to avoid log injection.
pub(crate) fn sanitize_label_value(s: &str) -> String {
    let s = s.trim();
    if s.len() <= MAX_LABEL_VALUE_LEN {
        s.chars()
            .map(|c| if c.is_control() { '?' } else { c })
            .collect()
    } else {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        s.hash(&mut hasher);
        format!("hash_{:x}", hasher.finish())
    }
}

/// Convert metric event attributes into typed OTLP key-values, sanitizing
/// string label values to prevent telemetry poisoning.
pub(crate) fn metric_key_values(
    attributes: &[(&'static str, MetricAttributeValue)],
) -> Vec<KeyValue> {
    attributes
        .iter()
        .map(|(key, value)| metric_kv(key, value))
        .collect()
}

fn metric_kv(key: &'static str, value: &MetricAttributeValue) -> KeyValue {
    match value {
        MetricAttributeValue::String(s) => kv(key, any_string(sanitize_label_value(s))),
        MetricAttributeValue::StaticStr(s) => {
            if s.len() > MAX_LABEL_VALUE_LEN {
                kv(key, any_string(sanitize_label_value(s)))
            } else {
                kv(key, any_string(*s))
            }
        }
        MetricAttributeValue::Bool(b) => kv(key, any_bool(*b)),
        MetricAttributeValue::I64(i) => kv(key, any_int(*i)),
        MetricAttributeValue::F64(f) => kv(key, any_double(*f)),
    }
}
