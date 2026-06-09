use std::{cell::RefCell, collections::HashMap};

use ferron_observability::Parent;
use opentelemetry::{
    baggage::BaggageExt,
    trace::{SpanContext, SpanId, TraceContextExt, TraceFlags, TraceId, TraceState},
    Context, Key, StringValue,
};
use opentelemetry_sdk::Resource;

/// Correlation context: tracks active spans per host sink instance.
pub struct CorrelationContext {
    /// Active spans: span_key -> active span entry
    active_spans: HashMap<String, ActiveSpan>,
}

pub(crate) struct ActiveSpan {
    pub(crate) trace_id_hex: String,
    pub(crate) span_id_hex: String,
    pub(crate) sampled: bool,
    pub(crate) span: opentelemetry_sdk::trace::Span,
    pub(crate) baggage: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct RequestedIdGenerator;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RequestedIds {
    pub(crate) trace_id: Option<opentelemetry::TraceId>,
    pub(crate) span_id: Option<opentelemetry::SpanId>,
}

thread_local! {
    pub(crate) static REQUESTED_IDS: RefCell<Option<RequestedIds>> = const { RefCell::new(None) };
}

impl CorrelationContext {
    pub fn new() -> Self {
        Self {
            active_spans: HashMap::new(),
        }
    }

    pub fn insert_span(
        &mut self,
        key: impl Into<String>,
        trace_id_hex: String,
        span_id_hex: String,
        sampled: bool,
        span: opentelemetry_sdk::trace::Span,
        baggage: Option<String>,
    ) {
        self.active_spans.insert(
            key.into(),
            ActiveSpan {
                trace_id_hex,
                span_id_hex,
                sampled,
                span,
                baggage,
            },
        );
    }

    pub(crate) fn remove_span(&mut self, key: &str) -> Option<ActiveSpan> {
        self.active_spans.remove(key)
    }

    /// Look up an active span's trace and span ID for use as a parent.
    pub fn get_parent_ids(&self, key: &str) -> Option<(String, String, bool, Option<String>)> {
        self.active_spans.get(key).map(|span| {
            (
                span.trace_id_hex.clone(),
                span.span_id_hex.clone(),
                span.sampled,
                span.baggage.clone(),
            )
        })
    }
}

impl opentelemetry_sdk::trace::IdGenerator for RequestedIdGenerator {
    fn new_trace_id(&self) -> opentelemetry::TraceId {
        if let Some(trace_id) = REQUESTED_IDS.with(|requested| {
            requested
                .borrow_mut()
                .as_mut()
                .and_then(|requested| requested.trace_id.take())
        }) {
            return trace_id;
        }

        opentelemetry_sdk::trace::RandomIdGenerator::default().new_trace_id()
    }

    fn new_span_id(&self) -> opentelemetry::SpanId {
        if let Some(span_id) = REQUESTED_IDS.with(|requested| {
            requested
                .borrow_mut()
                .as_mut()
                .and_then(|requested| requested.span_id.take())
        }) {
            return span_id;
        }

        opentelemetry_sdk::trace::RandomIdGenerator::default().new_span_id()
    }
}

/// Build an OTLP resource from the service name
pub(crate) fn build_resource(service_name: String) -> Resource {
    Resource::builder().with_service_name(service_name).build()
}

pub(crate) fn build_parent_context(
    correlation: &CorrelationContext,
    parent: &Parent,
) -> Option<Context> {
    let (trace_id_hex, span_id_hex, sampled, baggage) = match parent {
        Parent::ByKey(parent_key) => {
            let (trace_id_hex, span_id_hex, sampled, baggage) =
                correlation.get_parent_ids(parent_key)?;
            (trace_id_hex, span_id_hex, Some(sampled), baggage)
        }
        Parent::ById {
            trace_id,
            span_id,
            sampled,
            baggage,
        } => (trace_id.clone(), span_id.clone(), *sampled, baggage.clone()),
    };

    let (trace_id, span_id) = (
        TraceId::from_hex(&trace_id_hex).ok()?,
        SpanId::from_hex(&span_id_hex).ok()?,
    );
    let parent_ctx = SpanContext::new(
        trace_id,
        span_id,
        trace_flags(sampled).unwrap_or_default(),
        true,
        TraceState::default(),
    );
    let mut context = Context::new().with_remote_span_context(parent_ctx);
    if let Some(baggage) = baggage {
        // Parse baggage values
        let mut baggage_vec = Vec::new();
        for item in baggage.split(',') {
            let item = item.trim();
            if !item.is_empty() {
                let (kv, metadata) = if let Some(idx) = item.find(';') {
                    (&item[..idx], Some(&item[idx + 1..]))
                } else {
                    (item, None)
                };
                let Some((key, value)) = kv.split_once("=") else {
                    continue;
                };
                let metadata = if let Some(metadata) = metadata {
                    opentelemetry::baggage::BaggageMetadata::from(metadata)
                } else {
                    opentelemetry::baggage::BaggageMetadata::default()
                };
                let key = Key::from(key.trim_end().to_owned());
                let Some(value) = urlencoding::decode(value.trim_start())
                    .ok()
                    .map(|v| StringValue::from(v.to_string()))
                else {
                    continue;
                };
                baggage_vec.push((key, (value, metadata)));
            }
        }
        context = context.with_baggage(opentelemetry::baggage::Baggage::from_iter(baggage_vec));
    }
    Some(context)
}

pub(crate) fn trace_flags(sampled: Option<bool>) -> Option<opentelemetry::TraceFlags> {
    sampled.map(|sampled| {
        if sampled {
            opentelemetry::TraceFlags::SAMPLED
        } else {
            TraceFlags::default()
        }
    })
}

pub(crate) fn parse_requested_ids(
    trace_context: &ferron_observability::EventTraceContext,
) -> Option<RequestedIds> {
    let trace_id_str = std::str::from_utf8(&trace_context.trace_id).ok()?;
    let span_id_str = std::str::from_utf8(&trace_context.span_id).ok()?;
    Some(RequestedIds {
        trace_id: opentelemetry::TraceId::from_hex(trace_id_str).ok(),
        span_id: opentelemetry::SpanId::from_hex(span_id_str).ok(),
    })
}

pub(crate) fn with_requested_ids<T>(
    requested_ids: Option<RequestedIds>,
    f: impl FnOnce() -> T,
) -> T {
    REQUESTED_IDS.with(|current| {
        let previous = current.replace(requested_ids);
        let result = f();
        current.replace(previous);
        result
    })
}
