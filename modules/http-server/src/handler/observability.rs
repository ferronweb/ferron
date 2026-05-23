use std::borrow::Cow;
use std::sync::atomic::{AtomicU64, Ordering};

use ferron_core::pipeline::{PipelineError, Stage, StageHooks};
use ferron_http::trace_context;
use ferron_http::HttpRequest;
use ferron_observability::{
    AccessEvent, AccessVisitor, CompositeEventSink, Event, EventTraceContext, MetricAttributeValue,
    Parent, TraceAttributeValue, TraceEvent,
};

static SPAN_KEY_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Per-stage hooks that emit trace spans around each pipeline stage.
pub(super) struct PerStageSpanHooks<'a> {
    events: &'a CompositeEventSink,
    has_traces: bool,
    parent_span_key: &'a str,
    stage_group: &'a str,
}

impl<'a> PerStageSpanHooks<'a> {
    pub fn new(
        events: &'a CompositeEventSink,
        has_traces: bool,
        parent_span_key: &'a str,
        stage_group: &'a str,
    ) -> Self {
        Self {
            events,
            has_traces,
            parent_span_key,
            stage_group,
        }
    }

    pub fn stage_key(&self, stage_name: &str, inverse: bool) -> String {
        let suffix = if inverse { ":inverse" } else { "" };
        format!(
            "{}:{}:{}{}",
            self.parent_span_key, self.stage_group, stage_name, suffix
        )
    }
}

#[async_trait::async_trait(?Send)]
impl<C> StageHooks<C> for PerStageSpanHooks<'_> {
    #[inline]
    async fn before_stage(&mut self, stage: &dyn Stage<C>) {
        if !self.has_traces {
            return;
        }
        let stage_name = stage.name();
        self.events.emit(Event::Trace(TraceEvent::StartSpan {
            key: Cow::Owned(self.stage_key(stage_name, false)),
            name: Cow::Owned(format!("ferron.stage.{}", stage_name)),
            parent: Some(Parent::ByKey(self.parent_span_key.to_string())),
            trace_context: None,
            attributes: vec![(
                "stage.name",
                TraceAttributeValue::String(stage_name.to_string()),
            )],
        }));
    }

    #[inline]
    async fn after_stage(&mut self, stage: &dyn Stage<C>, result: &Result<bool, PipelineError>) {
        if !self.has_traces {
            return;
        }
        self.events.emit(Event::Trace(TraceEvent::EndSpan {
            key: Cow::Owned(self.stage_key(stage.name(), false)),
            name: Cow::Owned(format!("ferron.stage.{}", stage.name())),
            error: result.as_ref().err().map(|e| e.to_string()),
            attributes: vec![],
        }));
    }

    #[inline]
    async fn before_stage_inverse(&mut self, stage: &dyn Stage<C>) {
        if !self.has_traces {
            return;
        }
        let stage_name = stage.name();
        self.events.emit(Event::Trace(TraceEvent::StartSpan {
            key: Cow::Owned(self.stage_key(stage_name, true)),
            name: Cow::Owned(format!("ferron.stage.{}.inverse", stage_name)),
            parent: Some(Parent::ByKey(self.parent_span_key.to_string())),
            trace_context: None,
            attributes: vec![(
                "stage.name",
                TraceAttributeValue::String(stage_name.to_string()),
            )],
        }));
    }

    #[inline]
    async fn after_stage_inverse(
        &mut self,
        stage: &dyn Stage<C>,
        result: &Result<(), PipelineError>,
    ) {
        if !self.has_traces {
            return;
        }
        self.events.emit(Event::Trace(TraceEvent::EndSpan {
            key: Cow::Owned(self.stage_key(stage.name(), true)),
            name: Cow::Owned(format!("ferron.stage.{}.inverse", stage.name())),
            error: result.as_ref().err().map(|e| e.to_string()),
            attributes: vec![],
        }));
    }
}

/// Access log event emitted at request completion.
pub(super) struct HttpAccessLog {
    pub path: String,
    pub path_and_query: String,
    pub method: String,
    pub version: Cow<'static, str>,
    pub scheme: Cow<'static, str>,
    pub client_ip: String,
    pub client_port: u16,
    pub client_ip_canonical: String,
    pub server_ip: String,
    pub server_port: u16,
    pub server_ip_canonical: String,
    pub auth_user: Option<String>,
    pub status: u16,
    pub content_length: Option<u64>,
    pub duration_secs: f64,
    pub request_headers: Vec<(String, String)>,
    pub timestamp: chrono::DateTime<chrono::Local>,
    pub trace_context: Option<EventTraceContext>,
}

impl AccessEvent for HttpAccessLog {
    fn protocol(&self) -> &'static str {
        "http"
    }

    fn trace_context(&self) -> Option<&EventTraceContext> {
        self.trace_context.as_ref()
    }

    fn visit(&self, visitor: &mut dyn AccessVisitor) {
        visitor.field_string("path", &self.path);
        visitor.field_string("path_and_query", &self.path_and_query);
        visitor.field_string("method", &self.method);
        visitor.field_string("version", &self.version);
        visitor.field_string("scheme", &self.scheme);
        visitor.field_string("client_ip", &self.client_ip);
        visitor.field_u64("client_port", self.client_port as u64);
        visitor.field_string("client_ip_canonical", &self.client_ip_canonical);
        visitor.field_string("server_ip", &self.server_ip);
        visitor.field_u64("server_port", self.server_port as u64);
        visitor.field_string("server_ip_canonical", &self.server_ip_canonical);
        if let Some(user) = &self.auth_user {
            visitor.field_string("auth_user", user);
        } else {
            visitor.field_string("auth_user", "-");
        }
        visitor.field_u64("status", self.status as u64);
        if let Some(cl) = self.content_length {
            visitor.field_u64("content_length", cl);
        } else {
            visitor.field_string("content_length", "-");
        }
        visitor.field_f64("duration_secs", self.duration_secs);
        visitor.field_string(
            "timestamp",
            &self.timestamp.format("%d/%b/%Y:%H:%M:%S %z").to_string(),
        );
        for (name, value) in &self.request_headers {
            visitor.field_string(
                &format!("header_{}", name.to_ascii_lowercase().replace("-", "_")),
                value,
            );
        }
        // Optionally include trace identifiers when available
        if let Some(trace_context) = &self.trace_context {
            visitor.field_string("trace_id", &trace_context.trace_id);
            visitor.field_string("span_id", &trace_context.span_id);
        }
    }
}

pub(super) fn next_span_key(prefix: &str) -> String {
    format!(
        "{prefix}:{}",
        SPAN_KEY_COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

pub(super) fn to_event_trace_context(
    trace_context: &trace_context::TraceContext,
) -> EventTraceContext {
    EventTraceContext {
        trace_id: trace_context.trace_id.clone(),
        span_id: trace_context.span_id.clone(),
        sampled: Some(trace_context.sampled),
    }
}

pub fn resolve_request_trace_context(
    request: &HttpRequest,
    generate_enabled: bool,
    default_sampled: bool,
) -> (Option<trace_context::TraceContext>, Option<Parent>) {
    let incoming = request
        .headers()
        .get("traceparent")
        .and_then(|tp_val| tp_val.to_str().ok())
        .and_then(trace_context::parse_traceparent)
        .map(|mut trace_context| {
            trace_context.tracestate = request
                .headers()
                .get("tracestate")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            trace_context
        });

    if let Some(parent_context) = incoming {
        let request_context = trace_context::TraceContext {
            trace_id: parent_context.trace_id.clone(),
            span_id: trace_context::generate_span_id(),
            sampled: parent_context.sampled,
            tracestate: parent_context.tracestate.clone(),
        };
        return (
            Some(request_context),
            Some(Parent::ById {
                trace_id: parent_context.trace_id,
                span_id: parent_context.span_id,
                sampled: Some(parent_context.sampled),
            }),
        );
    }

    if generate_enabled {
        return (
            Some(trace_context::generate_traceparent(default_sampled)),
            None,
        );
    }

    (None, None)
}

/// Format HTTP version as a string (e.g. `HTTP/1.1`).
#[inline]
pub(super) fn http_version_access_string(version: http::Version) -> &'static str {
    match version {
        http::Version::HTTP_09 => "HTTP/0.9",
        http::Version::HTTP_10 => "HTTP/1.0",
        http::Version::HTTP_11 => "HTTP/1.1",
        http::Version::HTTP_2 => "HTTP/2.0",
        http::Version::HTTP_3 => "HTTP/3.0",
        _ => "HTTP/unknown",
    }
}

/// HTTP version string for metric attributes.
#[inline]
fn http_version_string(version: http::Version) -> Option<&'static str> {
    match version {
        http::Version::HTTP_09 => Some("0.9"),
        http::Version::HTTP_10 => Some("1.0"),
        http::Version::HTTP_11 => Some("1.1"),
        http::Version::HTTP_2 => Some("2"),
        http::Version::HTTP_3 => Some("3"),
        _ => None,
    }
}

/// Build the common metric attributes shared across all HTTP metrics.
#[inline]
pub fn build_metric_attributes(
    request: &HttpRequest,
    encrypted: bool,
    previous_error: Option<u16>,
) -> Vec<(&'static str, MetricAttributeValue)> {
    let mut attrs = Vec::with_capacity(5);
    attrs.push((
        "http.request.method",
        MetricAttributeValue::String(request.method().as_str().to_owned()),
    ));
    attrs.push((
        "url.scheme",
        MetricAttributeValue::StaticStr(if encrypted { "https" } else { "http" }),
    ));
    attrs.push((
        "network.protocol.name",
        MetricAttributeValue::StaticStr("http"),
    ));
    if let Some(http_ver) = http_version_string(request.version()) {
        attrs.push((
            "network.protocol.version",
            MetricAttributeValue::StaticStr(http_ver),
        ));
    }
    if let Some(error_code) = previous_error {
        attrs.push((
            "ferron.http.request.error_status_code",
            MetricAttributeValue::I64(error_code as i64),
        ));
    }
    attrs
}
