use std::borrow::Cow;

use ferron_observability::{
    baggage::{self, BaggageKeyPromotion, SignalSet},
    TraceAttributeValue, TraceEvent,
};
use opentelemetry::{
    trace::{Span, SpanBuilder, SpanKind, Tracer, TracerProvider},
    KeyValue,
};
use opentelemetry_sdk::trace::SdkTracerProvider;

use super::context::{build_parent_context, CorrelationContext};

pub(crate) fn emit_trace(
    provider: &SdkTracerProvider,
    event: &TraceEvent,
    correlation: &mut CorrelationContext,
    promotions: &[BaggageKeyPromotion],
) {
    let tracer = provider.tracer("ferron");

    match event {
        TraceEvent::StartSpan {
            key,
            name,
            parent,
            trace_context,
            builder_attributes,
            attributes,
        } => {
            let mut builder = SpanBuilder::from_name(name.clone());

            // Set SpanKind::Server for HTTP request spans
            if name.as_ref() == "ferron.request" {
                builder = builder.with_kind(SpanKind::Server);
            }

            // Set builder-level attributes (visible to the sampler)
            if !builder_attributes.is_empty() {
                let otel_attrs: Vec<KeyValue> = builder_attributes
                    .iter()
                    .map(|(k, v)| trace_kv(k.clone(), v))
                    .collect();
                builder = builder.with_attributes(otel_attrs);
            }

            let requested_ids = trace_context
                .as_ref()
                .and_then(super::context::parse_requested_ids);
            let mut span = super::context::with_requested_ids(requested_ids, || {
                if let Some(parent_val) = parent {
                    if let Some(parent_cx) = build_parent_context(correlation, parent_val) {
                        tracer.build_with_context(builder, &parent_cx)
                    } else {
                        tracer.build(builder)
                    }
                } else {
                    tracer.build(builder)
                }
            });

            // Set semantic convention attributes (post-build, not visible to sampler)
            for (key, value) in attributes {
                span.set_attribute(trace_kv((*key).into(), value));
            }

            // Promote configured baggage keys into span attributes
            if let Some(baggage_str) = trace_context.as_ref().and_then(|c| c.baggage.as_deref()) {
                let extracted =
                    baggage::extract_promoted_keys(baggage_str, promotions, SignalSet::TRACES);
                for attr in extracted {
                    span.set_attribute(KeyValue::new(attr.attribute_name, attr.value));
                }
            }

            let trace_id_hex = span.span_context().trace_id().to_string();
            let span_id_hex = span.span_context().span_id().to_string();
            let sampled = span.span_context().trace_flags().is_sampled();
            let baggage = trace_context.as_ref().and_then(|c| c.baggage.clone());
            correlation.insert_span(
                key.clone(),
                trace_id_hex,
                span_id_hex,
                sampled,
                span,
                baggage,
            );
        }
        TraceEvent::EndSpan {
            key,
            name: _,
            error,
            attributes,
        } => {
            if let Some(mut active_span) = correlation.remove_span(key) {
                // Apply any final attributes (e.g. http.response.status_code)
                for (key, value) in attributes {
                    active_span
                        .span
                        .set_attribute(trace_kv((*key).into(), value));
                }
                if let Some(error_desc) = error {
                    active_span
                        .span
                        .set_status(opentelemetry::trace::Status::error(error_desc.clone()));
                }
                active_span.span.end();
            }
        }
    }
}

/// Convert a TraceAttributeValue into an OTEL KeyValue.
fn trace_kv(key: Cow<'static, str>, value: &TraceAttributeValue) -> KeyValue {
    match value {
        TraceAttributeValue::String(s) => KeyValue::new(key, s.clone()),
        TraceAttributeValue::StaticStr(s) => KeyValue::new(key, *s),
        TraceAttributeValue::Bool(b) => KeyValue::new(key, *b),
        TraceAttributeValue::I64(i) => KeyValue::new(key, *i),
        TraceAttributeValue::F64(f) => KeyValue::new(key, *f),
    }
}
