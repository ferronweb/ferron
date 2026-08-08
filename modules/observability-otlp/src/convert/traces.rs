use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::SystemTime;

use ferron_observability::baggage::{self, BaggageKeyPromotion, SignalSet};
use ferron_observability::{Parent, SpanLink, TraceAttributeValue, TraceEvent};

use crate::proto::opentelemetry::proto::common::v1::KeyValue;
use crate::proto::opentelemetry::proto::trace::v1::{span, status, Span, Status};

use super::context::{
    decode_span_id, decode_trace_id, generate_span_id, generate_trace_id, parse_requested_ids,
    CorrelationContext, StoredSpan,
};
use super::{any_bool, any_double, any_int, any_string, kv, nanos};

/// The span flags field carries the W3C trace flags in its low 8 bits. The
/// SDK configured `Sampler::AlwaysOn`, so every span is sampled.
const SPAN_FLAGS_SAMPLED: u32 = 1;

/// Handle a `TraceEvent::StartSpan`: build the proto span and track it in the
/// correlation context so `Parent::ByKey` and `EndSpan` can resolve it.
///
/// Returns the span evicted from the correlation context when the LRU cache
/// overflows (finished with an error status so it is not silently lost).
#[inline]
pub(crate) fn start_span(
    event: &TraceEvent,
    correlation: &mut CorrelationContext,
    promotions: &[BaggageKeyPromotion],
    control_plane_metadata: &Option<Arc<BTreeMap<String, String>>>,
    now: SystemTime,
) -> Option<Span> {
    let TraceEvent::StartSpan {
        key,
        name,
        parent,
        trace_context,
        builder_attributes,
        attributes,
        links,
        control_plane_metadata: event_metadata,
    } = event
    else {
        return None;
    };

    let requested = trace_context.as_ref().map(parse_requested_ids);
    let parent_ref = resolve_parent(correlation, parent.as_ref());

    let trace_id: Vec<u8> = if let Some(id) = requested.as_ref().and_then(|r| r.trace_id) {
        id.to_vec()
    } else if let Some(parent) = &parent_ref {
        parent.trace_id.clone()
    } else {
        generate_trace_id().to_vec()
    };
    let span_id: Vec<u8> = requested
        .as_ref()
        .and_then(|r| r.span_id)
        .map(|id| id.to_vec())
        .unwrap_or_else(|| generate_span_id().to_vec());
    let parent_span_id = parent_ref.map(|p| p.span_id).unwrap_or_default();

    let mut attrs: Vec<KeyValue> =
        Vec::with_capacity(builder_attributes.len() + attributes.len() + 4);
    for (key, value) in builder_attributes {
        attrs.push(trace_kv(key.clone(), value));
    }
    // Prefer event-level metadata over provider-level metadata.
    let effective_metadata = event_metadata.as_ref().or(control_plane_metadata.as_ref());
    if let Some(metadata) = effective_metadata {
        for (attr_key, value) in metadata.iter() {
            attrs.push(kv(
                format!("ferron.control_plane.{attr_key}"),
                any_string(value),
            ));
        }
    }
    for (key, value) in attributes {
        attrs.push(trace_kv(Cow::Borrowed(*key), value));
    }
    // Promote configured baggage keys into span attributes.
    if let Some(baggage_str) = trace_context.as_ref().and_then(|c| c.baggage.as_deref()) {
        for attr in baggage::extract_promoted_keys(baggage_str, promotions, SignalSet::TRACES) {
            attrs.push(kv(attr.attribute_name, any_string(attr.value)));
        }
    }

    let kind = if name.as_ref() == "ferron.request" {
        span::SpanKind::Server as i32
    } else {
        span::SpanKind::Internal as i32
    };

    let span_links: Vec<span::Link> = links.iter().filter_map(build_link).collect();

    let span = Span {
        trace_id,
        span_id,
        trace_state: String::new(),
        parent_span_id,
        name: name.to_string(),
        kind,
        start_time_unix_nano: nanos(now),
        end_time_unix_nano: 0,
        attributes: attrs,
        dropped_attributes_count: 0,
        events: Vec::new(),
        dropped_events_count: 0,
        links: span_links,
        dropped_links_count: 0,
        status: None,
        flags: SPAN_FLAGS_SAMPLED,
    };

    let stored = StoredSpan {
        span,
        baggage: trace_context.as_ref().and_then(|c| c.baggage.clone()),
    };
    correlation
        .insert_span(key.to_string(), stored)
        .map(|mut evicted| {
            evicted.span.status = Some(Status {
                message: "Span evicted to prevent unbound memory growth".to_string(),
                code: status::StatusCode::Error as i32,
            });
            evicted.span.end_time_unix_nano = nanos(now);
            evicted.span
        })
}

/// Handle a `TraceEvent::EndSpan`: finish the tracked span and return it.
///
/// The span start time is the `StartSpan` ingestion time, the end time is
/// the `EndSpan` ingestion time (never before the start time).
#[inline]
pub(crate) fn end_span(
    event: &TraceEvent,
    correlation: &mut CorrelationContext,
    now: SystemTime,
) -> Option<Span> {
    let TraceEvent::EndSpan {
        key,
        error,
        attributes,
        ..
    } = event
    else {
        return None;
    };

    let mut stored = correlation.remove_span(key)?;
    for (key, value) in attributes {
        stored
            .span
            .attributes
            .push(trace_kv(Cow::Borrowed(*key), value));
    }
    if let Some(error_desc) = error {
        stored.span.status = Some(Status {
            message: error_desc.clone(),
            code: status::StatusCode::Error as i32,
        });
    }
    stored.span.end_time_unix_nano = nanos(now).max(stored.span.start_time_unix_nano);
    Some(stored.span)
}

/// Convert a [`TraceAttributeValue`] into an OTLP key-value.
#[inline]
fn trace_kv(key: Cow<'static, str>, value: &TraceAttributeValue) -> KeyValue {
    match value {
        TraceAttributeValue::String(s) => kv(key, any_string(s)),
        TraceAttributeValue::StaticStr(s) => kv(key, any_string(*s)),
        TraceAttributeValue::Bool(b) => kv(key, any_bool(*b)),
        TraceAttributeValue::I64(i) => kv(key, any_int(*i)),
        TraceAttributeValue::F64(f) => kv(key, any_double(*f)),
    }
}

/// Resolve the parent span IDs for a new span.
///
/// `Parent::ByKey` looks the active span up in the correlation context;
/// `Parent::ById` uses the IDs carried by the event. Malformed IDs resolve
/// to no parent (the span becomes a root span).
#[inline]
fn resolve_parent(
    correlation: &mut CorrelationContext,
    parent: Option<&Parent>,
) -> Option<ParentRef> {
    match parent {
        Some(Parent::ByKey(key)) => {
            let (trace_id, span_id, _) = correlation.get_parent_ids(key)?;
            Some(ParentRef { trace_id, span_id })
        }
        Some(Parent::ById {
            trace_id, span_id, ..
        }) => Some(ParentRef {
            trace_id: decode_trace_id(trace_id.as_bytes())?,
            span_id: decode_span_id(span_id.as_bytes())?,
        }),
        None => None,
    }
}

struct ParentRef {
    trace_id: Vec<u8>,
    span_id: Vec<u8>,
}

/// Convert a [`SpanLink`] into an OTLP link. Links with malformed IDs are
/// dropped.
#[inline]
fn build_link(link: &SpanLink) -> Option<span::Link> {
    let trace_id = decode_trace_id(link.trace_id.as_bytes())?;
    let span_id = decode_span_id(link.span_id.as_bytes())?;
    let attributes: Vec<KeyValue> = link
        .attributes
        .iter()
        .map(|(key, value)| trace_kv(Cow::Owned(key.clone()), value))
        .collect();
    Some(span::Link {
        trace_id,
        span_id,
        trace_state: String::new(),
        attributes,
        dropped_attributes_count: 0,
        flags: SPAN_FLAGS_SAMPLED,
    })
}
