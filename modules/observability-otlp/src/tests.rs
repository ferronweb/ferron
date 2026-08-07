use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ferron_observability::baggage::{BaggageKeyPromotion, SignalSet};
use ferron_observability::{
    AccessEvent, AccessVisitor, EventTraceContext, LogAttributeValue, LogEvent, LogLevel,
    MetricAttributeValue, Parent, SpanLink, TraceAttributeValue, TraceEvent,
};

use crate::config::LogStyle;
use crate::convert::CorrelationContext;
use crate::convert::{
    build_access_log_record, build_log_record, build_resource, end_span, metric_key_values, nanos,
    sanitize_label_value, start_span, OtelAccessAttributeVisitor,
};
use crate::proto::opentelemetry::proto::common::v1::any_value::Value;
use crate::proto::opentelemetry::proto::common::v1::{AnyValue, KeyValue};
use crate::proto::opentelemetry::proto::logs::v1::SeverityNumber;
use crate::proto::opentelemetry::proto::trace::v1::{span, status};

const TRACE_ID_HEX: &str = "0123456789abcdef0123456789abcdef";
const SPAN_ID_HEX: &str = "0123456789abcdef";

fn now() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_700_000_000)
}

fn later() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_700_000_001)
}

fn start_event(name: &'static str) -> TraceEvent {
    TraceEvent::StartSpan {
        key: Cow::Borrowed(name),
        name: Cow::Borrowed(name),
        parent: None,
        trace_context: None,
        builder_attributes: vec![],
        attributes: vec![],
        links: vec![],
        control_plane_metadata: None,
    }
}

fn find_attr<'a>(attrs: &'a [KeyValue], key: &str) -> Option<&'a AnyValue> {
    attrs
        .iter()
        .find(|kv| kv.key == key)
        .and_then(|kv| kv.value.as_ref())
}

fn str_value(value: &AnyValue) -> Option<&str> {
    match value.value.as_ref()? {
        Value::StringValue(s) => Some(s),
        _ => None,
    }
}

fn int_value(value: &AnyValue) -> Option<i64> {
    match value.value.as_ref()? {
        Value::IntValue(i) => Some(*i),
        _ => None,
    }
}

fn bool_value(value: &AnyValue) -> Option<bool> {
    match value.value.as_ref()? {
        Value::BoolValue(b) => Some(*b),
        _ => None,
    }
}

fn double_value(value: &AnyValue) -> Option<f64> {
    match value.value.as_ref()? {
        Value::DoubleValue(d) => Some(*d),
        _ => None,
    }
}

#[test]
fn correlation_context_tracks_active_spans() {
    let mut ctx = CorrelationContext::new();
    let event = TraceEvent::StartSpan {
        key: Cow::Borrowed("ferron.request_handler"),
        name: Cow::Borrowed("ferron.request_handler"),
        parent: None,
        trace_context: Some(EventTraceContext {
            trace_id: [b'0'; 32],
            span_id: [b'0'; 16],
            baggage: Some("a=b".to_string()),
            sampled: Some(true),
        }),
        builder_attributes: vec![],
        attributes: vec![],
        links: vec![],
        control_plane_metadata: None,
    };
    start_span(&event, &mut ctx, &[], &None, now());

    let (trace_id, span_id, baggage) = ctx
        .get_parent_ids("ferron.request_handler")
        .expect("should have active span");
    assert_eq!(trace_id.len(), 16);
    assert_eq!(span_id.len(), 8);
    assert_eq!(baggage.as_deref(), Some("a=b"));
}

#[test]
fn start_span_stores_span_with_attributes() {
    let mut correlation = CorrelationContext::new();
    let event = TraceEvent::StartSpan {
        key: Cow::Borrowed("test.span"),
        name: Cow::Borrowed("test.span"),
        parent: None,
        trace_context: None,
        builder_attributes: vec![(
            Cow::Borrowed("builder.key"),
            TraceAttributeValue::String("builder-value".to_string()),
        )],
        attributes: vec![
            (
                "http.request.method",
                TraceAttributeValue::String("GET".to_string()),
            ),
            (
                "url.path",
                TraceAttributeValue::String("/api/test".to_string()),
            ),
            ("http.response.status_code", TraceAttributeValue::I64(200)),
            ("flag", TraceAttributeValue::Bool(true)),
            ("ratio", TraceAttributeValue::F64(0.5)),
        ],
        links: vec![],
        control_plane_metadata: None,
    };
    start_span(&event, &mut correlation, &[], &None, now());

    let span = correlation.get_span("test.span").expect("span stored");
    assert_eq!(span.name, "test.span");
    assert_eq!(span.kind, span::SpanKind::Internal as i32);
    assert_eq!(span.trace_id.len(), 16);
    assert_eq!(span.span_id.len(), 8);
    assert!(span.parent_span_id.is_empty());
    assert_eq!(span.start_time_unix_nano, nanos(now()));
    assert_eq!(span.end_time_unix_nano, 0);
    assert_eq!(span.flags, 1, "AlwaysOn sampler: span is sampled");

    assert_eq!(
        str_value(find_attr(&span.attributes, "builder.key").unwrap()).unwrap(),
        "builder-value"
    );
    assert_eq!(
        str_value(find_attr(&span.attributes, "http.request.method").unwrap()).unwrap(),
        "GET"
    );
    assert_eq!(
        str_value(find_attr(&span.attributes, "url.path").unwrap()).unwrap(),
        "/api/test"
    );
    assert_eq!(
        int_value(find_attr(&span.attributes, "http.response.status_code").unwrap()).unwrap(),
        200
    );
    assert!(bool_value(find_attr(&span.attributes, "flag").unwrap()).unwrap());
    assert_eq!(
        double_value(find_attr(&span.attributes, "ratio").unwrap()).unwrap(),
        0.5
    );
}

#[test]
fn start_span_ferron_request_uses_server_kind() {
    let mut correlation = CorrelationContext::new();
    let event = TraceEvent::StartSpan {
        key: Cow::Borrowed("ferron.request"),
        name: Cow::Borrowed("ferron.request"),
        parent: None,
        trace_context: None,
        builder_attributes: vec![],
        attributes: vec![],
        links: vec![],
        control_plane_metadata: None,
    };
    start_span(&event, &mut correlation, &[], &None, now());

    let span = correlation.get_span("ferron.request").unwrap();
    assert_eq!(span.kind, span::SpanKind::Server as i32);
}

#[test]
fn start_span_uses_requested_trace_and_span_ids() {
    let mut correlation = CorrelationContext::new();
    let event = TraceEvent::StartSpan {
        key: Cow::Borrowed("test.span"),
        name: Cow::Borrowed("test.span"),
        parent: None,
        trace_context: Some(EventTraceContext {
            trace_id: [b'0'; 32],
            span_id: [b'0'; 16],
            baggage: None,
            sampled: Some(true),
        }),
        builder_attributes: vec![],
        attributes: vec![],
        links: vec![],
        control_plane_metadata: None,
    };
    // Zero IDs are invalid: the span must fall back to generated IDs.
    start_span(&event, &mut correlation, &[], &None, now());
    let span = correlation.get_span("test.span").unwrap();
    assert_ne!(span.trace_id, vec![0u8; 16]);
    assert_ne!(span.span_id, vec![0u8; 8]);

    let mut correlation = CorrelationContext::new();
    let event = TraceEvent::StartSpan {
        key: Cow::Borrowed("test.span"),
        name: Cow::Borrowed("test.span"),
        parent: None,
        trace_context: Some(EventTraceContext {
            trace_id: TRACE_ID_HEX.as_bytes().try_into().unwrap(),
            span_id: SPAN_ID_HEX.as_bytes().try_into().unwrap(),
            baggage: None,
            sampled: Some(true),
        }),
        builder_attributes: vec![],
        attributes: vec![],
        links: vec![],
        control_plane_metadata: None,
    };
    start_span(&event, &mut correlation, &[], &None, now());
    let span = correlation.get_span("test.span").unwrap();
    assert_eq!(span.trace_id, hex::decode(TRACE_ID_HEX).unwrap());
    assert_eq!(span.span_id, hex::decode(SPAN_ID_HEX).unwrap());
    assert_eq!(span.flags, 1);
}

#[test]
fn start_span_child_by_key_inherits_trace_and_parent() {
    let mut correlation = CorrelationContext::new();
    start_span(
        &start_event("parent.span"),
        &mut correlation,
        &[],
        &None,
        now(),
    );
    let parent = correlation.get_span("parent.span").unwrap().clone();

    let child = TraceEvent::StartSpan {
        key: Cow::Borrowed("child.span"),
        name: Cow::Borrowed("child.span"),
        parent: Some(Parent::ByKey("parent.span".to_string())),
        trace_context: None,
        builder_attributes: vec![],
        attributes: vec![],
        links: vec![],
        control_plane_metadata: None,
    };
    start_span(&child, &mut correlation, &[], &None, later());

    let span = correlation.get_span("child.span").unwrap();
    assert_eq!(span.trace_id, parent.trace_id);
    assert_eq!(span.parent_span_id, parent.span_id);
    assert_ne!(span.span_id, parent.span_id);
}

#[test]
fn start_span_child_by_id_sets_parent_span_id() {
    let mut correlation = CorrelationContext::new();
    let child = TraceEvent::StartSpan {
        key: Cow::Borrowed("child.span"),
        name: Cow::Borrowed("child.span"),
        parent: Some(Parent::ById {
            trace_id: TRACE_ID_HEX.to_string(),
            span_id: SPAN_ID_HEX.to_string(),
            sampled: Some(true),
            baggage: None,
        }),
        trace_context: None,
        builder_attributes: vec![],
        attributes: vec![],
        links: vec![],
        control_plane_metadata: None,
    };
    start_span(&child, &mut correlation, &[], &None, now());

    let span = correlation.get_span("child.span").unwrap();
    assert_eq!(span.trace_id, hex::decode(TRACE_ID_HEX).unwrap());
    assert_eq!(span.parent_span_id, hex::decode(SPAN_ID_HEX).unwrap());
}

#[test]
fn start_span_promotes_baggage_per_signal() {
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
            signals: Some(SignalSet::LOGS),
            max_distinct: None,
        },
    ];

    let event = TraceEvent::StartSpan {
        key: Cow::Borrowed("test.span"),
        name: Cow::Borrowed("test.span"),
        parent: None,
        trace_context: Some(EventTraceContext {
            trace_id: TRACE_ID_HEX.as_bytes().try_into().unwrap(),
            span_id: SPAN_ID_HEX.as_bytes().try_into().unwrap(),
            baggage: Some("tenant.id=acme,user.role=admin,other=skip".to_string()),
            sampled: Some(true),
        }),
        builder_attributes: vec![],
        attributes: vec![],
        links: vec![],
        control_plane_metadata: None,
    };
    start_span(&event, &mut correlation, &promotions, &None, now());

    let span = correlation.get_span("test.span").unwrap();
    assert_eq!(
        str_value(find_attr(&span.attributes, "tenant.id").unwrap()).unwrap(),
        "acme"
    );
    // Promoted only to logs, not traces.
    assert!(find_attr(&span.attributes, "ferron.user_role").is_none());
    // Not a configured promotion.
    assert!(find_attr(&span.attributes, "other").is_none());
}

#[test]
fn start_span_includes_links_and_drops_malformed_ones() {
    let mut correlation = CorrelationContext::new();
    let event = TraceEvent::StartSpan {
        key: Cow::Borrowed("test.span"),
        name: Cow::Borrowed("test.span"),
        parent: None,
        trace_context: None,
        builder_attributes: vec![],
        attributes: vec![],
        links: vec![
            SpanLink {
                trace_id: TRACE_ID_HEX.to_string(),
                span_id: SPAN_ID_HEX.to_string(),
                sampled: Some(true),
                attributes: vec![(
                    "link.key",
                    TraceAttributeValue::String("link-value".to_string()),
                )],
            },
            SpanLink {
                trace_id: "zzzz".to_string(),
                span_id: "zzzz".to_string(),
                sampled: None,
                attributes: vec![],
            },
        ],
        control_plane_metadata: None,
    };
    start_span(&event, &mut correlation, &[], &None, now());

    let span = correlation.get_span("test.span").unwrap();
    assert_eq!(span.links.len(), 1);
    assert_eq!(span.links[0].trace_id, hex::decode(TRACE_ID_HEX).unwrap());
    assert_eq!(span.links[0].span_id, hex::decode(SPAN_ID_HEX).unwrap());
    assert_eq!(
        str_value(find_attr(&span.links[0].attributes, "link.key").unwrap()).unwrap(),
        "link-value"
    );
}

#[test]
fn start_span_includes_control_plane_metadata() {
    let mut correlation = CorrelationContext::new();
    let event_metadata = Some(Arc::new(BTreeMap::from([(
        "tenant".to_string(),
        "acme".to_string(),
    )])));
    start_span(
        &start_event("test.span"),
        &mut correlation,
        &[],
        &event_metadata,
        now(),
    );

    let span = correlation.get_span("test.span").unwrap();
    assert_eq!(
        str_value(find_attr(&span.attributes, "ferron.control_plane.tenant").unwrap()).unwrap(),
        "acme"
    );

    // Provider-level metadata applies when the event carries none.
    let mut correlation = CorrelationContext::new();
    let provider_metadata = Some(Arc::new(BTreeMap::from([(
        "region".to_string(),
        "eu".to_string(),
    )])));
    start_span(
        &start_event("test.span"),
        &mut correlation,
        &[],
        &provider_metadata,
        now(),
    );
    let span = correlation.get_span("test.span").unwrap();
    assert_eq!(
        str_value(find_attr(&span.attributes, "ferron.control_plane.region").unwrap()).unwrap(),
        "eu"
    );
}

#[test]
fn end_span_merges_attributes_and_error() {
    let mut correlation = CorrelationContext::new();
    start_span(
        &start_event("test.span"),
        &mut correlation,
        &[],
        &None,
        now(),
    );

    let end_event = TraceEvent::EndSpan {
        key: Cow::Borrowed("test.span"),
        name: Cow::Borrowed("test.span"),
        error: Some("test error".to_string()),
        attributes: vec![("http.response.status_code", TraceAttributeValue::I64(500))],
        control_plane_metadata: None,
    };
    let span = end_span(&end_event, &mut correlation, later()).expect("finished span");

    assert_eq!(span.start_time_unix_nano, nanos(now()));
    assert_eq!(span.end_time_unix_nano, nanos(later()));
    let status = span.status.as_ref().expect("error status set");
    assert_eq!(status.code, status::StatusCode::Error as i32);
    assert_eq!(status.message, "test error");
    assert_eq!(
        int_value(find_attr(&span.attributes, "http.response.status_code").unwrap()).unwrap(),
        500
    );
    assert!(correlation.get_span("test.span").is_none());
}

#[test]
fn end_span_without_error_keeps_status_unset() {
    let mut correlation = CorrelationContext::new();
    start_span(
        &start_event("test.span"),
        &mut correlation,
        &[],
        &None,
        now(),
    );

    let end_event = TraceEvent::EndSpan {
        key: Cow::Borrowed("test.span"),
        name: Cow::Borrowed("test.span"),
        error: None,
        attributes: vec![("http.response.status_code", TraceAttributeValue::I64(200))],
        control_plane_metadata: None,
    };
    let span = end_span(&end_event, &mut correlation, now()).expect("finished span");
    assert!(span.status.is_none(), "no error means unset status");
}

#[test]
fn end_span_on_unknown_key_does_nothing() {
    let mut correlation = CorrelationContext::new();
    let end_event = TraceEvent::EndSpan {
        key: Cow::Borrowed("unknown.span"),
        name: Cow::Borrowed("unknown.span"),
        error: Some("should be ignored".to_string()),
        attributes: vec![],
        control_plane_metadata: None,
    };
    assert!(end_span(&end_event, &mut correlation, now()).is_none());
}

#[test]
fn end_span_never_before_start_time() {
    let mut correlation = CorrelationContext::new();
    start_span(
        &start_event("test.span"),
        &mut correlation,
        &[],
        &None,
        later(),
    );

    let end_event = TraceEvent::EndSpan {
        key: Cow::Borrowed("test.span"),
        name: Cow::Borrowed("test.span"),
        error: None,
        attributes: vec![],
        control_plane_metadata: None,
    };
    let span = end_span(&end_event, &mut correlation, now()).expect("finished span");
    assert_eq!(span.end_time_unix_nano, span.start_time_unix_nano);
}

#[test]
fn start_span_with_end_event_is_noop() {
    let mut correlation = CorrelationContext::new();
    let end_event = TraceEvent::EndSpan {
        key: Cow::Borrowed("test.span"),
        name: Cow::Borrowed("test.span"),
        error: None,
        attributes: vec![],
        control_plane_metadata: None,
    };
    assert!(start_span(&end_event, &mut correlation, &[], &None, now()).is_none());
}

#[test]
fn correlation_context_evicts_oldest_span() {
    let mut correlation = CorrelationContext::new();
    let mut evicted = None;
    for i in 0..65537 {
        let event = TraceEvent::StartSpan {
            key: Cow::Owned(format!("span.{i}")),
            name: Cow::Borrowed("test.span"),
            parent: None,
            trace_context: None,
            builder_attributes: vec![],
            attributes: vec![],
            links: vec![],
            control_plane_metadata: None,
        };
        evicted = start_span(&event, &mut correlation, &[], &None, now());
    }
    let evicted = evicted.expect("first span evicted on overflow");
    assert_eq!(evicted.name, "test.span");
    assert_eq!(
        evicted.status.as_ref().unwrap().code,
        status::StatusCode::Error as i32
    );
    assert_eq!(evicted.end_time_unix_nano, nanos(now()));
}

#[test]
fn build_log_record_legacy_uses_message_as_body() {
    let event = LogEvent {
        level: LogLevel::Info,
        message: "Legacy message".to_string(),
        summary: "Summary ignored in legacy".into(),
        target: "ferron-http-proxy",
        attributes: vec![(
            "upstream.address",
            LogAttributeValue::String("backend.example:8080".to_string()),
        )],
        trace_context: None,
    };
    let record = build_log_record(&event, &[], LogStyle::Legacy, now());

    assert_eq!(
        str_value(record.body.as_ref().unwrap()).unwrap(),
        "Legacy message"
    );
    assert_eq!(
        str_value(find_attr(&record.attributes, "log.target").unwrap()).unwrap(),
        "ferron-http-proxy"
    );
    // Legacy mode does not expose typed attributes.
    assert!(find_attr(&record.attributes, "upstream.address").is_none());
    assert_eq!(record.time_unix_nano, nanos(now()));
    assert_eq!(record.observed_time_unix_nano, nanos(now()));
}

#[test]
fn build_log_record_modern_uses_summary_and_typed_attributes() {
    let event = LogEvent {
        level: LogLevel::Warn,
        message: "Ignored in modern".to_string(),
        summary: "Upstream circuit opened".into(),
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
    let record = build_log_record(&event, &[], LogStyle::Modern, now());

    assert_eq!(
        str_value(record.body.as_ref().unwrap()).unwrap(),
        "Upstream circuit opened"
    );
    assert_eq!(
        str_value(find_attr(&record.attributes, "attr.string").unwrap()).unwrap(),
        "value"
    );
    assert_eq!(
        str_value(find_attr(&record.attributes, "attr.str").unwrap()).unwrap(),
        "static"
    );
    assert!(bool_value(find_attr(&record.attributes, "attr.bool").unwrap()).unwrap());
    assert_eq!(
        int_value(find_attr(&record.attributes, "attr.i64").unwrap()).unwrap(),
        42
    );
    assert_eq!(
        double_value(find_attr(&record.attributes, "attr.f64").unwrap()).unwrap(),
        1.5
    );
}

#[test]
fn build_log_record_maps_severity() {
    let levels = [
        (LogLevel::Error, SeverityNumber::Error as i32, "ERROR"),
        (LogLevel::Warn, SeverityNumber::Warn as i32, "WARN"),
        (LogLevel::Info, SeverityNumber::Info as i32, "INFO"),
        (LogLevel::Debug, SeverityNumber::Debug as i32, "DEBUG"),
    ];
    for (level, number, text) in levels {
        let event = LogEvent {
            level,
            message: "m".to_string(),
            summary: "s".into(),
            target: "t",
            attributes: vec![],
            trace_context: None,
        };
        let record = build_log_record(&event, &[], LogStyle::Legacy, now());
        assert_eq!(record.severity_number, number);
        assert_eq!(record.severity_text, text);
    }
}

#[test]
fn build_log_record_sets_trace_context() {
    let event = LogEvent {
        level: LogLevel::Info,
        message: "m".to_string(),
        summary: "s".into(),
        target: "t",
        attributes: vec![],
        trace_context: Some(EventTraceContext {
            trace_id: TRACE_ID_HEX.as_bytes().try_into().unwrap(),
            span_id: SPAN_ID_HEX.as_bytes().try_into().unwrap(),
            baggage: None,
            sampled: Some(true),
        }),
    };
    let record = build_log_record(&event, &[], LogStyle::Modern, now());
    assert_eq!(record.trace_id, hex::decode(TRACE_ID_HEX).unwrap());
    assert_eq!(record.span_id, hex::decode(SPAN_ID_HEX).unwrap());
    assert_eq!(record.flags, 1);
}

#[test]
fn build_log_record_skips_malformed_trace_context() {
    let event = LogEvent {
        level: LogLevel::Info,
        message: "m".to_string(),
        summary: "s".into(),
        target: "t",
        attributes: vec![],
        trace_context: Some(EventTraceContext {
            trace_id: [b'z'; 32],
            span_id: [b'0'; 16],
            baggage: None,
            sampled: Some(true),
        }),
    };
    let record = build_log_record(&event, &[], LogStyle::Modern, now());
    assert!(record.trace_id.is_empty());
    assert!(record.span_id.is_empty());
    assert_eq!(record.flags, 0);
}

#[test]
fn build_log_record_promotes_baggage() {
    let promotions = vec![BaggageKeyPromotion {
        baggage_key: "tenant.id".to_string(),
        attribute_name: Some("ferron.tenant".to_string()),
        signals: None,
        max_distinct: None,
    }];
    let event = LogEvent {
        level: LogLevel::Info,
        message: "m".to_string(),
        summary: "s".into(),
        target: "t",
        attributes: vec![],
        trace_context: Some(EventTraceContext {
            trace_id: TRACE_ID_HEX.as_bytes().try_into().unwrap(),
            span_id: SPAN_ID_HEX.as_bytes().try_into().unwrap(),
            baggage: Some("tenant.id=acme".to_string()),
            sampled: Some(true),
        }),
    };
    let record = build_log_record(&event, &promotions, LogStyle::Legacy, now());
    assert_eq!(
        str_value(find_attr(&record.attributes, "ferron.tenant").unwrap()).unwrap(),
        "acme"
    );
}

struct DummyAccess {
    proto: &'static str,
    event_time: Option<SystemTime>,
}

impl AccessEvent for DummyAccess {
    fn protocol(&self) -> &'static str {
        self.proto
    }
    fn visit(&self, visitor: &mut dyn AccessVisitor) {
        visitor.field_string("path", "/");
        visitor.field_string("method", "GET");
        visitor.field_u64("status", 200);
    }
    fn event_time(&self) -> Option<SystemTime> {
        self.event_time
    }
}

fn dummy_access(proto: &'static str) -> Arc<dyn AccessEvent> {
    Arc::new(DummyAccess {
        proto,
        event_time: None,
    })
}

#[test]
fn build_access_log_record_modern_maps_semantic_conventions() {
    let registry = ferron_core::registry::Registry::new();
    let log_config = Arc::new(ferron_core::config::ServerConfigurationBlock::default());
    let record = build_access_log_record(
        &dummy_access("http"),
        &log_config,
        &registry,
        &[],
        LogStyle::Modern,
        &None,
        now(),
    );

    assert_eq!(
        str_value(record.body.as_ref().unwrap()).unwrap(),
        "Access log (http)"
    );
    assert_eq!(
        str_value(find_attr(&record.attributes, "url.path").unwrap()).unwrap(),
        "/"
    );
    assert_eq!(
        str_value(find_attr(&record.attributes, "http.request.method").unwrap()).unwrap(),
        "GET"
    );
    assert_eq!(
        int_value(find_attr(&record.attributes, "http.response.status_code").unwrap()).unwrap(),
        200
    );
    assert_eq!(record.time_unix_nano, nanos(now()));
}

#[test]
fn build_access_log_record_modern_uses_event_time() {
    let registry = ferron_core::registry::Registry::new();
    let log_config = Arc::new(ferron_core::config::ServerConfigurationBlock::default());
    let event: Arc<dyn AccessEvent> = Arc::new(DummyAccess {
        proto: "http",
        event_time: Some(UNIX_EPOCH),
    });
    let record = build_access_log_record(
        &event,
        &log_config,
        &registry,
        &[],
        LogStyle::Modern,
        &None,
        now(),
    );
    assert_eq!(record.time_unix_nano, 0);
    assert_eq!(record.observed_time_unix_nano, nanos(now()));
}

#[test]
fn build_access_log_record_legacy_falls_back_without_formatter() {
    let registry = ferron_core::registry::Registry::new();
    let log_config = Arc::new(ferron_core::config::ServerConfigurationBlock::default());
    let record = build_access_log_record(
        &dummy_access("http"),
        &log_config,
        &registry,
        &[],
        LogStyle::Legacy,
        &None,
        now(),
    );
    assert_eq!(
        str_value(record.body.as_ref().unwrap()).unwrap(),
        "<unknown access log>"
    );
}

#[test]
fn otel_access_attribute_visitor_maps_to_otel_semantic_conventions() {
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
    for key in &[
        "timestamp",
        "trace_id",
        "client_ip_canonical",
        "server_ip_canonical",
    ] {
        assert!(
            !attrs.contains_key(*key),
            "key {key} should be absent in modern mode"
        );
    }

    // Typed values survive the mapping.
    let status = visitor
        .attributes
        .iter()
        .find(|(k, _)| k == "http.response.status_code")
        .unwrap();
    assert_eq!(int_value(&status.1).unwrap(), 200);
    let duration = visitor
        .attributes
        .iter()
        .find(|(k, _)| k == "http.server.request.duration")
        .unwrap();
    assert_eq!(double_value(&duration.1).unwrap(), 0.123);
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

#[test]
fn metric_key_values_preserves_types_and_sanitizes_strings() {
    let long = "A".repeat(1000);
    let attrs = metric_key_values(&[
        (
            "http.request.method",
            MetricAttributeValue::String("GET\r\nX-Injected: true".to_string()),
        ),
        ("user_agent", MetricAttributeValue::String(long.clone())),
        ("static", MetricAttributeValue::StaticStr("plain")),
        ("flag", MetricAttributeValue::Bool(true)),
        ("count", MetricAttributeValue::I64(-7)),
        ("ratio", MetricAttributeValue::F64(0.25)),
    ]);

    assert_eq!(
        str_value(find_attr(&attrs, "http.request.method").unwrap()).unwrap(),
        "GET??X-Injected: true"
    );
    let ua = str_value(find_attr(&attrs, "user_agent").unwrap()).unwrap();
    assert!(ua.starts_with("hash_"));
    assert_eq!(
        str_value(find_attr(&attrs, "static").unwrap()).unwrap(),
        "plain"
    );
    assert!(bool_value(find_attr(&attrs, "flag").unwrap()).unwrap());
    assert_eq!(int_value(find_attr(&attrs, "count").unwrap()).unwrap(), -7);
    assert_eq!(
        double_value(find_attr(&attrs, "ratio").unwrap()).unwrap(),
        0.25
    );
}

#[test]
fn build_resource_includes_service_and_process_identity() {
    let resource = build_resource("test-service".to_string());
    assert_eq!(
        str_value(find_attr(&resource.attributes, "service.name").unwrap()).unwrap(),
        "test-service"
    );
    let pid = int_value(find_attr(&resource.attributes, "process.pid").unwrap()).unwrap();
    assert_eq!(pid, std::process::id() as i64);
    let start_time =
        int_value(find_attr(&resource.attributes, "process.start_time").unwrap()).unwrap();
    assert!(start_time > 0);
}

// ---------------------------------------------------------------------------
// Configuration parsing
// ---------------------------------------------------------------------------

use ferron_core::config::{
    ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationValue,
};

type SubDirective = (String, Vec<ServerConfigurationValue>);
type SignalDirective = (
    &'static str,
    Vec<ServerConfigurationValue>,
    Option<Vec<SubDirective>>,
);

/// Build an `observability`-style block from signal sub-blocks.
fn signal_block(signals: Vec<SignalDirective>) -> ServerConfigurationBlock {
    ServerConfigurationBlock {
        directives: Arc::new(
            signals
                .into_iter()
                .map(|(name, args, children)| {
                    (
                        name.to_string(),
                        vec![ServerConfigurationDirectiveEntry {
                            args,
                            children: children.map(|sub| ServerConfigurationBlock {
                                directives: Arc::new(
                                    sub.into_iter()
                                        .map(|(n, a)| {
                                            (
                                                n,
                                                vec![ServerConfigurationDirectiveEntry {
                                                    args: a,
                                                    children: None,
                                                    span: None,
                                                }],
                                            )
                                        })
                                        .collect(),
                                ),
                                matchers: Default::default(),
                                span: None,
                            }),
                            span: None,
                        }],
                    )
                })
                .collect(),
        ),
        matchers: Default::default(),
        span: None,
    }
}

#[test]
fn signal_config_parses_batch_tuning_directives() {
    use crate::config::OtlpBackendConfig;
    use ferron_core::config::ServerConfigurationValue;

    let block = signal_block(vec![
        (
            "logs",
            vec![ServerConfigurationValue::String(
                "http://collector:4318/v1/logs".to_string(),
                None,
            )],
            Some(vec![
                (
                    "export_interval".to_string(),
                    vec![ServerConfigurationValue::String("10s".to_string(), None)],
                ),
                (
                    "export_batch_size".to_string(),
                    vec![ServerConfigurationValue::Number(256, None)],
                ),
                ("gzip".to_string(), vec![]),
            ]),
        ),
        (
            "metrics",
            vec![ServerConfigurationValue::String(
                "http://collector:4318/v1/metrics".to_string(),
                None,
            )],
            Some(vec![(
                "read_interval".to_string(),
                vec![ServerConfigurationValue::String("60s".to_string(), None)],
            )]),
        ),
        (
            "traces",
            vec![ServerConfigurationValue::String(
                "http://collector:4318/v1/traces".to_string(),
                None,
            )],
            Some(vec![(
                "export_interval".to_string(),
                vec![ServerConfigurationValue::Number(5, None)],
            )]),
        ),
    ]);

    let config = OtlpBackendConfig::parse_config(&block);

    let logs = config.logs.unwrap();
    assert_eq!(logs.export_interval, Some(Duration::from_secs(10)));
    assert_eq!(logs.export_batch_size, Some(256));
    assert!(logs.gzip);

    let traces = config.traces.unwrap();
    assert_eq!(traces.export_interval, Some(Duration::from_secs(5)));
    assert!(!traces.gzip);

    let metrics = config.metrics.unwrap();
    assert_eq!(metrics.read_interval, Some(Duration::from_secs(60)));
    assert!(!metrics.gzip);
}

#[test]
fn signal_config_parses_gzip_flag_with_explicit_false() {
    use crate::config::OtlpBackendConfig;
    use ferron_core::config::ServerConfigurationValue;

    let block = signal_block(vec![(
        "traces",
        vec![ServerConfigurationValue::String(
            "http://collector:4318/v1/traces".to_string(),
            None,
        )],
        Some(vec![(
            "gzip".to_string(),
            vec![ServerConfigurationValue::Boolean(false, None)],
        )]),
    )]);

    let config = OtlpBackendConfig::parse_config(&block);
    assert!(!config.traces.unwrap().gzip);
}

#[test]
fn signal_config_parses_exemplars_flag_with_default_true() {
    use crate::config::OtlpBackendConfig;
    use ferron_core::config::ServerConfigurationValue;

    let block = signal_block(vec![(
        "metrics",
        vec![ServerConfigurationValue::String(
            "http://localhost:4318".to_string(),
            None,
        )],
        Some(vec![
            ("exemplars".to_string(), vec![]),
            ("gzip".to_string(), vec![]),
        ]),
    )]);

    let config = OtlpBackendConfig::parse_config(&block);
    let metrics = config.metrics.unwrap();
    assert_eq!(metrics.exemplars, Some(true));

    let block = signal_block(vec![(
        "metrics",
        vec![ServerConfigurationValue::String(
            "http://localhost:4318".to_string(),
            None,
        )],
        Some(vec![(
            "exemplars".to_string(),
            vec![ServerConfigurationValue::Boolean(false, None)],
        )]),
    )]);

    let config = OtlpBackendConfig::parse_config(&block);
    assert_eq!(config.metrics.unwrap().exemplars, Some(false));

    let block = signal_block(vec![(
        "metrics",
        vec![ServerConfigurationValue::String(
            "http://localhost:4318".to_string(),
            None,
        )],
        None,
    )]);

    let config = OtlpBackendConfig::parse_config(&block);
    assert_eq!(config.metrics.unwrap().exemplars, None);
}
