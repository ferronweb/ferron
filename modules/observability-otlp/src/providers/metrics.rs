use std::collections::HashMap;

use ferron_observability::baggage::{self, BaggageKeyPromotion, DistinctValueTracker, SignalSet};
use ferron_observability::{MetricAttributeValue, MetricEvent, MetricType, MetricValue};
use opentelemetry::KeyValue;

/// Maximum length for a metric label value before it is hashed to prevent cardinality explosion.
const MAX_LABEL_VALUE_LEN: usize = 128;

pub enum CachedInstrument {
    F64Counter(opentelemetry::metrics::Counter<f64>),
    F64Gauge(opentelemetry::metrics::Gauge<f64>),
    F64Histogram(opentelemetry::metrics::Histogram<f64>),
    F64UpDownCounter(opentelemetry::metrics::UpDownCounter<f64>),
    I64Gauge(opentelemetry::metrics::Gauge<i64>),
    I64UpDownCounter(opentelemetry::metrics::UpDownCounter<i64>),
    U64Counter(opentelemetry::metrics::Counter<u64>),
    U64Gauge(opentelemetry::metrics::Gauge<u64>),
    U64Histogram(opentelemetry::metrics::Histogram<u64>),
}

/// Sanitize a metric label value to prevent high-cardinality telemetry poisoning.
///
/// Values longer than 128 characters are replaced with its hash.
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

pub(crate) fn emit_metric(
    provider: &opentelemetry_sdk::metrics::SdkMeterProvider,
    event: &MetricEvent,
    instruments: &mut HashMap<&'static str, CachedInstrument>,
    promotions: &[BaggageKeyPromotion],
    tracker: &mut DistinctValueTracker,
) {
    use opentelemetry::metrics::MeterProvider;

    let meter = provider.meter("ferron");
    let mut attrs: Vec<KeyValue> = event
        .attributes
        .iter()
        .map(|(k, v)| {
            let value = match v {
                MetricAttributeValue::F64(val) => opentelemetry::Value::from(*val),
                MetricAttributeValue::I64(val) => opentelemetry::Value::from(*val),
                MetricAttributeValue::String(val) => {
                    opentelemetry::Value::from(sanitize_label_value(val))
                }
                MetricAttributeValue::StaticStr(val) => {
                    if val.len() > MAX_LABEL_VALUE_LEN {
                        opentelemetry::Value::from(sanitize_label_value(val))
                    } else {
                        opentelemetry::Value::from(*val)
                    }
                }
                MetricAttributeValue::Bool(val) => opentelemetry::Value::from(*val),
            };
            KeyValue::new(*k, value)
        })
        .collect();

    // Promote configured baggage keys into metric attributes
    if let Some(baggage_str) = event
        .trace_context
        .as_ref()
        .and_then(|c| c.baggage.as_deref())
    {
        let extracted = baggage::extract_promoted_keys(baggage_str, promotions, SignalSet::METRICS);
        for attr in extracted {
            let value = tracker.canonicalize(
                &attr.attribute_name,
                &attr.value,
                promotions
                    .iter()
                    .find(|p| p.effective_attribute_name() == attr.attribute_name)
                    .and_then(|p| p.max_distinct),
            );
            attrs.push(KeyValue::new(attr.attribute_name, value));
        }
    }

    match (&event.ty, event.value) {
        (MetricType::Counter, MetricValue::F64(val)) => {
            let instrument = instruments.entry(event.name).or_insert_with(|| {
                let mut b = meter.f64_counter(event.name);
                if let Some(u) = event.unit {
                    b = b.with_unit(u);
                }
                if let Some(d) = event.description {
                    b = b.with_description(d);
                }
                CachedInstrument::F64Counter(b.build())
            });
            if let CachedInstrument::F64Counter(i) = instrument {
                i.add(val, &attrs);
            }
        }
        (MetricType::Counter, MetricValue::U64(val)) => {
            let instrument = instruments.entry(event.name).or_insert_with(|| {
                let mut b = meter.u64_counter(event.name);
                if let Some(u) = event.unit {
                    b = b.with_unit(u);
                }
                if let Some(d) = event.description {
                    b = b.with_description(d);
                }
                CachedInstrument::U64Counter(b.build())
            });
            if let CachedInstrument::U64Counter(i) = instrument {
                i.add(val, &attrs);
            }
        }
        (MetricType::UpDownCounter, MetricValue::F64(val)) => {
            let instrument = instruments.entry(event.name).or_insert_with(|| {
                let mut b = meter.f64_up_down_counter(event.name);
                if let Some(u) = event.unit {
                    b = b.with_unit(u);
                }
                if let Some(d) = event.description {
                    b = b.with_description(d);
                }
                CachedInstrument::F64UpDownCounter(b.build())
            });
            if let CachedInstrument::F64UpDownCounter(i) = instrument {
                i.add(val, &attrs);
            }
        }
        (MetricType::UpDownCounter, MetricValue::I64(val)) => {
            let instrument = instruments.entry(event.name).or_insert_with(|| {
                let mut b = meter.i64_up_down_counter(event.name);
                if let Some(u) = event.unit {
                    b = b.with_unit(u);
                }
                if let Some(d) = event.description {
                    b = b.with_description(d);
                }
                CachedInstrument::I64UpDownCounter(b.build())
            });
            if let CachedInstrument::I64UpDownCounter(i) = instrument {
                i.add(val, &attrs);
            }
        }
        (MetricType::Gauge, MetricValue::F64(val)) => {
            let instrument = instruments.entry(event.name).or_insert_with(|| {
                let mut b = meter.f64_gauge(event.name);
                if let Some(u) = event.unit {
                    b = b.with_unit(u);
                }
                if let Some(d) = event.description {
                    b = b.with_description(d);
                }
                CachedInstrument::F64Gauge(b.build())
            });
            if let CachedInstrument::F64Gauge(i) = instrument {
                i.record(val, &attrs);
            }
        }
        (MetricType::Gauge, MetricValue::I64(val)) => {
            let instrument = instruments.entry(event.name).or_insert_with(|| {
                let mut b = meter.i64_gauge(event.name);
                if let Some(u) = event.unit {
                    b = b.with_unit(u);
                }
                if let Some(d) = event.description {
                    b = b.with_description(d);
                }
                CachedInstrument::I64Gauge(b.build())
            });
            if let CachedInstrument::I64Gauge(i) = instrument {
                i.record(val, &attrs);
            }
        }
        (MetricType::Gauge, MetricValue::U64(val)) => {
            let instrument = instruments.entry(event.name).or_insert_with(|| {
                let mut b = meter.u64_gauge(event.name);
                if let Some(u) = event.unit {
                    b = b.with_unit(u);
                }
                if let Some(d) = event.description {
                    b = b.with_description(d);
                }
                CachedInstrument::U64Gauge(b.build())
            });
            if let CachedInstrument::U64Gauge(i) = instrument {
                i.record(val, &attrs);
            }
        }
        (MetricType::Histogram(buckets), MetricValue::F64(val)) => {
            let instrument = instruments.entry(event.name).or_insert_with(|| {
                let mut b = meter.f64_histogram(event.name);
                if let Some(u) = event.unit {
                    b = b.with_unit(u);
                }
                if let Some(ref bkt) = buckets {
                    b = b.with_boundaries(bkt.to_vec());
                }
                if let Some(d) = event.description {
                    b = b.with_description(d);
                }
                CachedInstrument::F64Histogram(b.build())
            });
            if let CachedInstrument::F64Histogram(i) = instrument {
                i.record(val, &attrs);
            }
        }
        (MetricType::Histogram(buckets), MetricValue::U64(val)) => {
            let instrument = instruments.entry(event.name).or_insert_with(|| {
                let mut b = meter.u64_histogram(event.name);
                if let Some(u) = event.unit {
                    b = b.with_unit(u);
                }
                if let Some(ref bkt) = buckets {
                    b = b.with_boundaries(bkt.to_vec());
                }
                if let Some(d) = event.description {
                    b = b.with_description(d);
                }
                CachedInstrument::U64Histogram(b.build())
            });
            if let CachedInstrument::U64Histogram(i) = instrument {
                i.record(val, &attrs);
            }
        }
        _ => {}
    }
}
