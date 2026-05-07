use crate::providers::CorrelationContext;

use super::*;
use ferron_core::config::{
    ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationValue,
};
use ferron_observability::{TraceAttributeValue, TraceEvent};

#[allow(dead_code)]
fn make_block(
    directives: HashMap<String, Vec<ServerConfigurationDirectiveEntry>>,
) -> ServerConfigurationBlock {
    ServerConfigurationBlock {
        directives: Arc::new(directives),
        matchers: HashMap::new(),
        span: None,
    }
}

#[allow(dead_code)]
fn directive_string(name: &str, value: &str) -> (String, Vec<ServerConfigurationDirectiveEntry>) {
    (
        name.to_string(),
        vec![ServerConfigurationDirectiveEntry {
            args: vec![ServerConfigurationValue::String(value.to_string(), None)],
            children: None,
            span: None,
        }],
    )
}

#[allow(dead_code)]
fn directive_bool(name: &str, value: bool) -> (String, Vec<ServerConfigurationDirectiveEntry>) {
    (
        name.to_string(),
        vec![ServerConfigurationDirectiveEntry {
            args: vec![ServerConfigurationValue::Boolean(value, None)],
            children: None,
            span: None,
        }],
    )
}

#[allow(dead_code)]
fn directive_with_children(
    name: &str,
    value: &str,
    children: HashMap<String, Vec<ServerConfigurationDirectiveEntry>>,
) -> (String, Vec<ServerConfigurationDirectiveEntry>) {
    (
        name.to_string(),
        vec![ServerConfigurationDirectiveEntry {
            args: vec![ServerConfigurationValue::String(value.to_string(), None)],
            children: Some(ServerConfigurationBlock {
                directives: Arc::new(children),
                matchers: HashMap::new(),
                span: None,
            }),
            span: None,
        }],
    )
}

#[test]
fn parses_otlp_config_with_all_signals() {
    let mut directives = HashMap::new();
    directives.insert(
        "service_name".to_string(),
        vec![ferron_core::config::ServerConfigurationDirectiveEntry {
            args: vec![ferron_core::config::ServerConfigurationValue::String(
                "test-service".to_string(),
                None,
            )],
            children: None,
            span: None,
        }],
    );
    directives.insert(
        "no_verify".to_string(),
        vec![ferron_core::config::ServerConfigurationDirectiveEntry {
            args: vec![ferron_core::config::ServerConfigurationValue::Boolean(
                true, None,
            )],
            children: None,
            span: None,
        }],
    );

    let mut logs_children = HashMap::new();
    logs_children.insert(
        "protocol".to_string(),
        vec![ferron_core::config::ServerConfigurationDirectiveEntry {
            args: vec![ferron_core::config::ServerConfigurationValue::String(
                "http/protobuf".to_string(),
                None,
            )],
            children: None,
            span: None,
        }],
    );
    directives.insert(
        "logs".to_string(),
        vec![ferron_core::config::ServerConfigurationDirectiveEntry {
            args: vec![ferron_core::config::ServerConfigurationValue::String(
                "https://collector:4318/v1/logs".to_string(),
                None,
            )],
            children: Some(ServerConfigurationBlock {
                directives: Arc::new(logs_children),
                matchers: HashMap::new(),
                span: None,
            }),
            span: None,
        }],
    );

    let block = ServerConfigurationBlock {
        directives: Arc::new(directives),
        matchers: HashMap::new(),
        span: None,
    };

    let config = OtlpBackendConfig::parse_config(&block);
    assert_eq!(config.service_name, "test-service");
    assert!(config.no_verify);
    assert!(config.logs.is_some());
    assert!(config.metrics.is_none());
    assert!(config.traces.is_none());

    let logs = config.logs.unwrap();
    assert_eq!(logs.endpoint, "https://collector:4318/v1/logs");
    assert_eq!(logs.protocol, "http/protobuf");
}

#[test]
fn parses_otlp_config_minimal() {
    let directives = HashMap::new();
    let block = ServerConfigurationBlock {
        directives: Arc::new(directives),
        matchers: HashMap::new(),
        span: None,
    };

    let config = OtlpBackendConfig::parse_config(&block);
    assert_eq!(config.service_name, "ferron");
    assert!(!config.no_verify);
    assert!(config.logs.is_none());
    assert!(config.metrics.is_none());
    assert!(config.traces.is_none());
}

#[test]
fn correlation_context_tracks_active_spans() {
    use opentelemetry::trace::{Span, Tracer, TracerProvider};

    let ctx = CorrelationContext::new();
    let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder().build();
    let tracer = provider.tracer("test");

    let span = tracer.start("ferron.request_handler");
    let trace_id_hex = span.span_context().trace_id().to_string();
    let span_id_hex = span.span_context().span_id().to_string();

    ctx.insert_span(
        "ferron.request_handler".to_string(),
        trace_id_hex.clone(),
        span,
    );

    let (t_id, s_id) = ctx
        .get_parent_ids("ferron.request_handler")
        .expect("should have active span");
    assert_eq!(t_id, trace_id_hex);
    assert_eq!(s_id, span_id_hex);

    let result = ctx.remove_span("ferron.request_handler");
    assert!(result.is_some());

    assert!(ctx.get_parent_ids("ferron.request_handler").is_none());
}

#[test]
fn emit_trace_start_span_stores_span_object() {
    use ferron_observability::TraceAttributeValue;
    use std::borrow::Cow;

    let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder().build();
    let correlation = CorrelationContext::new();

    let event = TraceEvent::StartSpan {
        name: Cow::Borrowed("test.span"),
        parent: None,
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
        name: Cow::Borrowed("test.span"),
        parent: None,
        attributes: vec![(
            "http.request.method",
            TraceAttributeValue::String("POST".to_string()),
        )],
    };
    emit_trace(&provider, &start_event, &correlation);

    let end_event = TraceEvent::EndSpan {
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
        name: Cow::Borrowed("test.span"),
        parent: None,
        attributes: vec![],
    };
    emit_trace(&provider, &start_event, &correlation);

    let end_event = TraceEvent::EndSpan {
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
        name: Cow::Borrowed("unknown.span"),
        error: Some("should be ignored".to_string()),
        attributes: vec![],
    };
    emit_trace(&provider, &end_event, &correlation);
    assert!(correlation.get_parent_ids("unknown.span").is_none());
}
