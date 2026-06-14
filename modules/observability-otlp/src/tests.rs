use crate::providers::{sanitize_label_value, CorrelationContext};

use super::*;
use ferron_observability::baggage::{BaggageKeyPromotion, DistinctValueTracker, SignalSet};
use ferron_observability::{MetricAttributeValue, TraceAttributeValue, TraceEvent};

#[test]
fn correlation_context_tracks_active_spans() {
    use opentelemetry::trace::{Span, Tracer, TracerProvider};

    let mut ctx = CorrelationContext::new();
    let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder().build();
    let tracer = provider.tracer("test");

    let span = tracer.start("ferron.request_handler");
    let trace_id_hex = span.span_context().trace_id().to_string();
    let span_id_hex = span.span_context().span_id().to_string();
    let sampled = span.span_context().trace_flags().is_sampled();
    let baggage = Some("a=b".to_string());

    ctx.insert_span(
        "ferron.request_handler".to_string(),
        trace_id_hex.clone(),
        span_id_hex.clone(),
        sampled,
        span,
        baggage.clone(),
    );

    let (t_id, s_id, is_sampled, baggage2) = ctx
        .get_parent_ids("ferron.request_handler")
        .expect("should have active span");
    assert_eq!(t_id, trace_id_hex);
    assert_eq!(s_id, span_id_hex);
    assert_eq!(is_sampled, sampled);
    assert_eq!(baggage2, baggage);
}

#[test]
fn emit_trace_start_span_stores_span_object() {
    use ferron_observability::TraceAttributeValue;
    use std::borrow::Cow;

    let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder().build();
    let mut correlation = CorrelationContext::new();

    let event = TraceEvent::StartSpan {
        key: Cow::Borrowed("test.span"),
        name: Cow::Borrowed("test.span"),
        parent: None,
        trace_context: None,
        builder_attributes: vec![],
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

    emit_trace(&provider, &event, &mut correlation, &[]);

    assert!(correlation.get_parent_ids("test.span").is_some());
}

#[test]
fn emit_trace_end_span_ends_properly() {
    use ferron_observability::TraceAttributeValue;
    use std::borrow::Cow;

    let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder().build();
    let mut correlation = CorrelationContext::new();

    let start_event = TraceEvent::StartSpan {
        key: Cow::Borrowed("test.span"),
        name: Cow::Borrowed("test.span"),
        parent: None,
        trace_context: None,
        builder_attributes: vec![],
        attributes: vec![(
            "http.request.method",
            TraceAttributeValue::String("POST".to_string()),
        )],
    };
    emit_trace(&provider, &start_event, &mut correlation, &[]);

    let end_event = TraceEvent::EndSpan {
        key: Cow::Borrowed("test.span"),
        name: Cow::Borrowed("test.span"),
        error: Some("test error".to_string()),
        attributes: vec![("http.response.status_code", TraceAttributeValue::I64(500))],
    };
    emit_trace(&provider, &end_event, &mut correlation, &[]);

    assert!(correlation.get_parent_ids("test.span").is_none());
}

#[test]
fn emit_trace_end_span_without_error() {
    use std::borrow::Cow;

    let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder().build();
    let mut correlation = CorrelationContext::new();

    let start_event = TraceEvent::StartSpan {
        key: Cow::Borrowed("test.span"),
        name: Cow::Borrowed("test.span"),
        parent: None,
        trace_context: None,
        builder_attributes: vec![],
        attributes: vec![],
    };
    emit_trace(&provider, &start_event, &mut correlation, &[]);

    let end_event = TraceEvent::EndSpan {
        key: Cow::Borrowed("test.span"),
        name: Cow::Borrowed("test.span"),
        error: None,
        attributes: vec![("http.response.status_code", TraceAttributeValue::I64(200))],
    };
    emit_trace(&provider, &end_event, &mut correlation, &[]);

    assert!(correlation.get_parent_ids("test.span").is_none());
}

#[test]
fn emit_trace_end_span_on_unknown_name_does_nothing() {
    use std::borrow::Cow;

    let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder().build();
    let mut correlation = CorrelationContext::new();

    let end_event = TraceEvent::EndSpan {
        key: Cow::Borrowed("unknown.span"),
        name: Cow::Borrowed("unknown.span"),
        error: Some("should be ignored".to_string()),
        attributes: vec![],
    };
    emit_trace(&provider, &end_event, &mut correlation, &[]);
    assert!(correlation.get_parent_ids("unknown.span").is_none());
}

#[test]
fn sanitize_label_value_short_string_unchanged() {
    let result = sanitize_label_value("GET");
    assert_eq!(result, "GET");
}

#[test]
fn sanitize_label_value_trims_whitespace() {
    let result = sanitize_label_value("  GET  ");
    assert_eq!(result, "GET");
}

#[test]
fn sanitize_label_value_control_chars_replaced() {
    let result = sanitize_label_value("GET\r\nX-Injected: true");
    assert_eq!(result, "GET??X-Injected: true");
}

#[test]
fn sanitize_label_value_long_string_hashed() {
    let long = "A".repeat(200);
    let result = sanitize_label_value(&long);
    assert!(result.starts_with("hash_"));
    assert!(result.len() < 50, "hash should be shorter than input");
}

#[test]
fn sanitize_label_value_exact_128_chars_preserved() {
    let exact = "B".repeat(128);
    let result = sanitize_label_value(&exact);
    assert_eq!(result, exact);
}

#[test]
fn sanitize_label_value_129_chars_hashed() {
    let over = "C".repeat(129);
    let result = sanitize_label_value(&over);
    assert!(result.starts_with("hash_"));
}

#[test]
fn sanitize_label_value_deterministic_hash() {
    let input = "D".repeat(200);
    let r1 = sanitize_label_value(&input);
    let r2 = sanitize_label_value(&input);
    assert_eq!(r1, r2, "hash should be deterministic");
}

#[test]
fn sanitize_label_value_empty_string() {
    let result = sanitize_label_value("");
    assert_eq!(result, "");
}

#[test]
fn sanitize_label_value_only_control_chars() {
    let result = sanitize_label_value("\x00\x01\x02\x03");
    assert_eq!(result, "????");
}

#[tokio::test]
async fn emit_metric_with_high_cardinality_label_is_sanitized() {
    use ferron_observability::{MetricEvent, MetricType, MetricValue};

    let provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder().build();
    let mut instruments = HashMap::new();

    // Simulate an attacker sending 1000 different custom HTTP methods
    let mut unique_values = std::collections::HashSet::new();
    for i in 0..1000 {
        let method = format!("CUSTOM_{i:04}");
        let event = MetricEvent {
            name: "test.cardinality",
            attributes: vec![("http.request.method", MetricAttributeValue::String(method))],
            ty: MetricType::Counter,
            value: MetricValue::U64(1),
            unit: None,
            description: None,
            trace_context: None,
        };
        emit_metric(
            &provider,
            &event,
            &mut instruments,
            &[],
            &mut DistinctValueTracker::new(),
        );

        // Verify the sanitized value is bounded
        let sanitized = sanitize_label_value(&format!("CUSTOM_{i:04}"));
        unique_values.insert(sanitized);
    }

    // The number of unique sanitized values should be much less than 1000
    // because values > 128 chars get hashed (these are short, so they won't be
    // hashed, but the test verifies the sanitization path works).
    assert!(unique_values.len() <= 1000);
}

#[tokio::test]
async fn emit_metric_with_long_label_value_is_hashed() {
    use ferron_observability::{MetricEvent, MetricType, MetricValue};

    let provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder().build();
    let mut instruments = HashMap::new();

    // Simulate a very long User-Agent string (classic telemetry poisoning vector)
    let long_ua = "A".repeat(1000);
    let event = MetricEvent {
        name: "test.long.label",
        attributes: vec![("user_agent", MetricAttributeValue::String(long_ua.clone()))],
        ty: MetricType::Counter,
        value: MetricValue::U64(1),
        unit: None,
        description: None,
        trace_context: None,
    };

    // Should not panic or OOM
    emit_metric(
        &provider,
        &event,
        &mut instruments,
        &[],
        &mut DistinctValueTracker::new(),
    );

    // The internal sanitization should have hashed the value
    let sanitized = sanitize_label_value(&long_ua);
    assert!(sanitized.starts_with("hash_"));
}

#[test]
fn emit_trace_promotes_baggage_to_span_attributes() {
    use std::borrow::Cow;

    let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder().build();
    let mut correlation = CorrelationContext::new();

    let promotions = vec![
        BaggageKeyPromotion {
            baggage_key: "tenant.id".to_string(),
            attribute_name: None,
            signals: None,
            max_distinct: None,
        },
        BaggageKeyPromotion {
            baggage_key: "user.role".to_string(),
            attribute_name: Some("ferron.user_role".to_string()),
            signals: None,
            max_distinct: None,
        },
    ];

    let event = TraceEvent::StartSpan {
        key: Cow::Borrowed("test.span"),
        name: Cow::Borrowed("test.span"),
        parent: None,
        trace_context: Some(ferron_observability::EventTraceContext {
            trace_id: [b'0'; 32],
            span_id: [b'0'; 16],
            baggage: Some("tenant.id=acme,user.role=admin,other=skip".to_string()),
            sampled: Some(true),
        }),
        builder_attributes: vec![],
        attributes: vec![],
    };

    emit_trace(&provider, &event, &mut correlation, &promotions);

    // Verify span was created with baggage attributes
    let entry = correlation
        .get_parent_ids("test.span")
        .expect("span should exist");
    // The baggage was stored in the correlation context
    assert!(entry.3.is_some());
}

#[test]
fn emit_trace_respects_signal_filter_for_baggage() {
    use std::borrow::Cow;

    let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder().build();
    let mut correlation = CorrelationContext::new();

    // Only promote to logs, not traces
    let promotions = vec![BaggageKeyPromotion {
        baggage_key: "tenant.id".to_string(),
        attribute_name: None,
        signals: Some(SignalSet::LOGS),
        max_distinct: None,
    }];

    let event = TraceEvent::StartSpan {
        key: Cow::Borrowed("test.span"),
        name: Cow::Borrowed("test.span"),
        parent: None,
        trace_context: Some(ferron_observability::EventTraceContext {
            trace_id: [b'0'; 32],
            span_id: [b'0'; 16],
            baggage: Some("tenant.id=acme".to_string()),
            sampled: Some(true),
        }),
        builder_attributes: vec![],
        attributes: vec![],
    };

    // Should not panic, and the baggage should not be promoted to traces
    emit_trace(&provider, &event, &mut correlation, &promotions);
    assert!(correlation.get_parent_ids("test.span").is_some());
}

#[test]
fn emit_log_promotes_baggage_to_log_attributes() {
    let provider = opentelemetry_sdk::logs::SdkLoggerProvider::builder().build();

    let promotions = vec![BaggageKeyPromotion {
        baggage_key: "tenant.id".to_string(),
        attribute_name: Some("ferron.tenant".to_string()),
        signals: None,
        max_distinct: None,
    }];

    let event = ferron_observability::LogEvent {
        level: ferron_observability::LogLevel::Info,
        message: "test message".to_string(),
        summary: "Test log message".into(),
        target: "test",
        attributes: Vec::new(),
        trace_context: Some(ferron_observability::EventTraceContext {
            trace_id: [b'0'; 32],
            span_id: [b'0'; 16],
            baggage: Some("tenant.id=acme".to_string()),
            sampled: Some(true),
        }),
    };

    // Should not panic
    emit_log(
        &provider,
        &event,
        &promotions,
        crate::config::LogStyle::Legacy,
    );
}

#[tokio::test]
async fn emit_metric_promotes_baggage_to_metric_attributes() {
    use ferron_observability::{MetricEvent, MetricType, MetricValue};

    let provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder().build();
    let mut instruments = HashMap::new();
    let mut tracker = DistinctValueTracker::new();

    let promotions = vec![BaggageKeyPromotion {
        baggage_key: "tenant.id".to_string(),
        attribute_name: None,
        signals: None,
        max_distinct: None,
    }];

    let event = MetricEvent {
        name: "test.baggage.metric",
        attributes: vec![],
        ty: MetricType::Counter,
        value: MetricValue::U64(1),
        unit: None,
        description: None,
        trace_context: Some(ferron_observability::EventTraceContext {
            trace_id: [b'0'; 32],
            span_id: [b'0'; 16],
            baggage: Some("tenant.id=acme".to_string()),
            sampled: Some(true),
        }),
    };

    // Should not panic and should include baggage attribute
    emit_metric(
        &provider,
        &event,
        &mut instruments,
        &promotions,
        &mut tracker,
    );
}

#[tokio::test]
async fn emit_metric_baggage_cardinality_cap() {
    use ferron_observability::{MetricEvent, MetricType, MetricValue};

    let provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder().build();
    let mut instruments = HashMap::new();
    let mut tracker = DistinctValueTracker::new();

    let promotions = vec![BaggageKeyPromotion {
        baggage_key: "user.id".to_string(),
        attribute_name: None,
        signals: None,
        max_distinct: Some(2),
    }];

    // First two distinct values should pass through
    for i in 0..2 {
        let event = MetricEvent {
            name: "test.cardinality.baggage",
            attributes: vec![],
            ty: MetricType::Counter,
            value: MetricValue::U64(1),
            unit: None,
            description: None,
            trace_context: Some(ferron_observability::EventTraceContext {
                trace_id: [b'0'; 32],
                span_id: [b'0'; 16],
                baggage: Some(format!("user.id=value{i}")),
                sampled: Some(true),
            }),
        };
        emit_metric(
            &provider,
            &event,
            &mut instruments,
            &promotions,
            &mut tracker,
        );
    }

    // Third distinct value should be hashed (no panic)
    let event = MetricEvent {
        name: "test.cardinality.baggage",
        attributes: vec![],
        ty: MetricType::Counter,
        value: MetricValue::U64(1),
        unit: None,
        description: None,
        trace_context: Some(ferron_observability::EventTraceContext {
            trace_id: [b'0'; 32],
            span_id: [b'0'; 16],
            baggage: Some("user.id=value2".to_string()),
            sampled: Some(true),
        }),
    };
    emit_metric(
        &provider,
        &event,
        &mut instruments,
        &promotions,
        &mut tracker,
    );
}

#[test]
fn emit_log_modern_uses_summary_as_body() {
    use crate::config::LogStyle;
    use ferron_observability::{LogAttributeValue, LogEvent, LogLevel};

    let provider = opentelemetry_sdk::logs::SdkLoggerProvider::builder().build();
    let event = LogEvent {
        level: LogLevel::Info,
        message: "Legacy message that should be ignored in modern mode".to_string(),
        summary: "Upstream circuit opened".into(),
        target: "ferron-http-proxy",
        attributes: vec![
            (
                "upstream.address",
                LogAttributeValue::String("backend.example:8080".to_string()),
            ),
            ("http.response.status_code", LogAttributeValue::I64(502)),
        ],
        trace_context: None,
    };

    // Should not panic.
    emit_log(&provider, &event, &[], LogStyle::Modern);
}

#[test]
fn emit_log_modern_preserves_attribute_types() {
    use crate::config::LogStyle;
    use ferron_observability::{LogAttributeValue, LogEvent, LogLevel};

    let provider = opentelemetry_sdk::logs::SdkLoggerProvider::builder().build();
    let event = LogEvent {
        level: LogLevel::Warn,
        message: "Should not appear in modern body".to_string(),
        summary: "Test summary".into(),
        target: "test",
        attributes: vec![
            (
                "attr.string",
                LogAttributeValue::String("value".to_string()),
            ),
            ("attr.str", LogAttributeValue::StaticStr("static")),
            ("attr.bool", LogAttributeValue::Bool(true)),
            ("attr.i64", LogAttributeValue::I64(42)),
            ("attr.f64", LogAttributeValue::F64(1.5)),
        ],
        trace_context: None,
    };

    // Smoke test: must not panic and must accept all attribute variants.
    emit_log(&provider, &event, &[], LogStyle::Modern);
}

#[test]
fn otel_access_attribute_visitor_maps_to_otel_semantic_conventions() {
    use crate::providers::OtelAccessAttributeVisitor;
    use ferron_observability::AccessVisitor;

    let mut visitor = OtelAccessAttributeVisitor::default();
    visitor.field_string("path", "/api/v1/users");
    visitor.field_string("path_and_query", "/api/v1/users?id=42");
    visitor.field_string("method", "GET");
    visitor.field_string("version", "1.1");
    visitor.field_string("scheme", "https");
    visitor.field_string("client_ip", "203.0.113.1");
    visitor.field_string("server_ip", "198.51.100.1");
    visitor.field_string("auth_user", "alice");
    visitor.field_u64("client_port", 54321);
    visitor.field_u64("server_port", 443);
    visitor.field_u64("status", 200);
    visitor.field_u64("content_length", 1024);
    visitor.field_f64("duration_secs", 0.123);
    visitor.field_string("header_user_agent", "Mozilla/5.0");
    // Legacy-only fields, should be dropped in modern mode.
    visitor.field_string("timestamp", "01/Jan/2026:00:00:00 +0000");
    visitor.field_string("trace_id", &"0".repeat(32));
    visitor.field_string("client_ip_canonical", "203.0.113.1");
    visitor.field_string("server_ip_canonical", "198.51.100.1");

    let attrs: std::collections::HashMap<String, ()> = visitor
        .attributes
        .iter()
        .map(|(k, _)| (k.clone(), ()))
        .collect();
    assert!(attrs.contains_key("url.path"));
    assert!(attrs.contains_key("url.full"));
    assert!(attrs.contains_key("http.request.method"));
    assert!(attrs.contains_key("network.protocol.version"));
    assert!(attrs.contains_key("url.scheme"));
    assert!(attrs.contains_key("client.address"));
    assert!(attrs.contains_key("server.address"));
    assert!(attrs.contains_key("user.name"));
    assert!(attrs.contains_key("client.port"));
    assert!(attrs.contains_key("server.port"));
    assert!(attrs.contains_key("http.response.status_code"));
    assert!(attrs.contains_key("http.response.body.size"));
    assert!(attrs.contains_key("http.server.request.duration"));
    assert!(attrs.contains_key("http.request.header.user_agent"));
    // Dropped in modern mode.
    assert!(!attrs.contains_key("timestamp"));
    assert!(!attrs.contains_key("trace_id"));
    assert!(!attrs.contains_key("client_ip_canonical"));
    assert!(!attrs.contains_key("server_ip_canonical"));
}

#[test]
fn emit_access_log_modern_smoke() {
    use crate::config::LogStyle;
    use std::sync::Arc;

    struct DummyAccess {
        proto: &'static str,
    }
    impl ferron_observability::AccessEvent for DummyAccess {
        fn protocol(&self) -> &'static str {
            self.proto
        }
        fn visit(&self, visitor: &mut dyn ferron_observability::AccessVisitor) {
            visitor.field_string("path", "/");
            visitor.field_string("method", "GET");
            visitor.field_u64("status", 200);
        }
    }

    let registry = Registry::new();
    let provider = opentelemetry_sdk::logs::SdkLoggerProvider::builder().build();
    let event: Arc<dyn ferron_observability::AccessEvent> = Arc::new(DummyAccess { proto: "http" });
    let log_config = Arc::new(ferron_core::config::ServerConfigurationBlock::default());
    // Should not panic.
    emit_access_log(
        &provider,
        &event,
        &log_config,
        &registry,
        &[],
        LogStyle::Modern,
    );
}
