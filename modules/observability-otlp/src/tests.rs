use crate::providers::CorrelationContext;

use super::*;
use ferron_observability::{TraceAttributeValue, TraceEvent};

#[test]
fn correlation_context_tracks_active_spans() {
    use opentelemetry::trace::{Span, Tracer, TracerProvider};

    let ctx = CorrelationContext::new();
    let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder().build();
    let tracer = provider.tracer("test");

    let span = tracer.start("ferron.request_handler");
    let trace_id_hex = span.span_context().trace_id().to_string();
    let span_id_hex = span.span_context().span_id().to_string();
    let sampled = span.span_context().trace_flags().is_sampled();

    ctx.insert_span(
        "ferron.request_handler".to_string(),
        trace_id_hex.clone(),
        span_id_hex.clone(),
        sampled,
        span,
    );

    let (t_id, s_id, is_sampled) = ctx
        .get_parent_ids("ferron.request_handler")
        .expect("should have active span");
    assert_eq!(t_id, trace_id_hex);
    assert_eq!(s_id, span_id_hex);
    assert_eq!(is_sampled, sampled);
}

#[test]
fn emit_trace_start_span_stores_span_object() {
    use ferron_observability::TraceAttributeValue;
    use std::borrow::Cow;

    let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder().build();
    let correlation = CorrelationContext::new();

    let event = TraceEvent::StartSpan {
        key: Cow::Borrowed("test.span"),
        name: Cow::Borrowed("test.span"),
        parent: None,
        trace_context: None,
        attributes: vec![
            (
                "http.request.method",
                TraceAttributeValue::String("GET".to_string()),
            ),
            (
                "url.path",
                TraceAttributeValue::String("/api/test".to_string()),
            ),
        ],
    };

    emit_trace(&provider, &event, &correlation);

    assert!(correlation.get_parent_ids("test.span").is_some());
}

#[test]
fn emit_trace_end_span_ends_properly() {
    use ferron_observability::TraceAttributeValue;
    use std::borrow::Cow;

    let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder().build();
    let correlation = CorrelationContext::new();

    let start_event = TraceEvent::StartSpan {
        key: Cow::Borrowed("test.span"),
        name: Cow::Borrowed("test.span"),
        parent: None,
        trace_context: None,
        attributes: vec![(
            "http.request.method",
            TraceAttributeValue::String("POST".to_string()),
        )],
    };
    emit_trace(&provider, &start_event, &correlation);

    let end_event = TraceEvent::EndSpan {
        key: Cow::Borrowed("test.span"),
        name: Cow::Borrowed("test.span"),
        error: Some("test error".to_string()),
        attributes: vec![("http.response.status_code", TraceAttributeValue::I64(500))],
    };
    emit_trace(&provider, &end_event, &correlation);

    assert!(correlation.get_parent_ids("test.span").is_none());
}

#[test]
fn emit_trace_end_span_without_error() {
    use std::borrow::Cow;

    let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder().build();
    let correlation = CorrelationContext::new();

    let start_event = TraceEvent::StartSpan {
        key: Cow::Borrowed("test.span"),
        name: Cow::Borrowed("test.span"),
        parent: None,
        trace_context: None,
        attributes: vec![],
    };
    emit_trace(&provider, &start_event, &correlation);

    let end_event = TraceEvent::EndSpan {
        key: Cow::Borrowed("test.span"),
        name: Cow::Borrowed("test.span"),
        error: None,
        attributes: vec![("http.response.status_code", TraceAttributeValue::I64(200))],
    };
    emit_trace(&provider, &end_event, &correlation);

    assert!(correlation.get_parent_ids("test.span").is_none());
}

#[test]
fn emit_trace_end_span_on_unknown_name_does_nothing() {
    use std::borrow::Cow;

    let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder().build();
    let correlation = CorrelationContext::new();

    let end_event = TraceEvent::EndSpan {
        key: Cow::Borrowed("unknown.span"),
        name: Cow::Borrowed("unknown.span"),
        error: Some("should be ignored".to_string()),
        attributes: vec![],
    };
    emit_trace(&provider, &end_event, &correlation);
    assert!(correlation.get_parent_ids("unknown.span").is_none());
}
