use std::borrow::Cow;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use ferron_core::pipeline::{PipelineError, Stage, StageHooks};
use ferron_http::access_log::CustomAccessLogField;
use ferron_http::span::HttpContextSpanExt;
use ferron_http::{trace_context, HttpRequest};
use ferron_observability::{
    AccessEvent, AccessVisitor, CompositeEventSink, Event, EventTraceContext, LogAttributeValue,
    MetricAttributeValue, Parent, TraceAttributeValue, TraceEvent,
};
use rustc_hash::{FxHashMap, FxHashSet};

pub use ferron_http::trace_context::to_event_trace_context;

static SPAN_KEY_COUNTER: AtomicU64 = AtomicU64::new(1);
/// List of sensitive fields to redact from log output by default (lower-case).
pub const SENSITIVE_FIELDS_REDACTED: &[&str] = &[
    "password",
    "secret", // Credentials
    "cookie", // HTTP cookies
    "key",
    "token",         // API keys and tokens
    "authorization", // HTTP auth headers
];

/// Per-stage hooks that emit trace spans around each pipeline stage.
pub(super) struct PerStageSpanHooks<'a> {
    events: &'a CompositeEventSink,
    has_traces: bool,
    parent_span_key: &'a str,
    stage_group: &'a str,
    keys: FxHashSet<(String, String)>,
    control_plane_metadata: Option<Arc<std::collections::BTreeMap<String, String>>>,
}

impl<'a> PerStageSpanHooks<'a> {
    #[inline]
    pub fn new(
        events: &'a CompositeEventSink,
        has_traces: bool,
        parent_span_key: &'a str,
        stage_group: &'a str,
        control_plane_metadata: Option<Arc<std::collections::BTreeMap<String, String>>>,
    ) -> Self {
        Self {
            events,
            has_traces,
            parent_span_key,
            stage_group,
            keys: FxHashSet::default(),
            control_plane_metadata,
        }
    }

    #[inline]
    fn stage_key(&self, stage_name: &str, inverse: bool) -> String {
        let suffix = if inverse { ":inverse" } else { "" };
        format!(
            "{}:{}:{}{}",
            self.parent_span_key, self.stage_group, stage_name, suffix
        )
    }

    #[inline]
    pub fn flush(&mut self) {
        if !self.has_traces {
            return;
        }
        for (event_key, event_name) in self.keys.drain() {
            self.events.emit(Event::Trace(TraceEvent::EndSpan {
                key: Cow::Owned(event_key),
                name: Cow::Owned(event_name),
                error: Some("Pipeline couldn't complete (timeout or error)".to_string()),
                attributes: vec![],
                control_plane_metadata: self.control_plane_metadata.clone(),
            }));
        }
    }
}

impl Drop for PerStageSpanHooks<'_> {
    #[inline]
    fn drop(&mut self) {
        self.flush();
    }
}

#[async_trait::async_trait(?Send)]
impl<C> StageHooks<C> for PerStageSpanHooks<'_>
where
    C: HttpContextSpanExt,
{
    #[inline]
    async fn before_stage(&mut self, stage: &dyn Stage<C>) {
        if !self.has_traces {
            return;
        }
        let stage_name = stage.name();
        let stage_name_otel = format!("ferron.stage.{}", stage_name);
        let stage_key = self.stage_key(stage_name, false);
        self.keys
            .insert((stage_key.clone(), stage_name_otel.clone()));
        self.events.emit(Event::Trace(TraceEvent::StartSpan {
            key: Cow::Owned(stage_key),
            name: Cow::Owned(stage_name_otel),
            parent: Some(Parent::ByKey(self.parent_span_key.to_string())),
            trace_context: None,
            builder_attributes: vec![],
            attributes: vec![(
                "ferron.stage.name",
                TraceAttributeValue::String(stage_name.to_string()),
            )],
            links: vec![],
            control_plane_metadata: self.control_plane_metadata.clone(),
        }));
    }

    #[inline]
    async fn after_stage(
        &mut self,
        stage: &dyn Stage<C>,
        result: &Result<bool, PipelineError>,
        ctx: &mut C,
    ) {
        if !self.has_traces {
            return;
        }
        let stage_name = stage.name();
        let stage_name_otel = format!("ferron.stage.{}", stage_name);
        let stage_key = self.stage_key(stage_name, false);
        self.keys
            .remove(&(stage_key.clone(), stage_name_otel.clone()));
        self.events.emit(Event::Trace(TraceEvent::EndSpan {
            key: Cow::Owned(stage_key),
            name: Cow::Owned(stage_name_otel),
            error: result.as_ref().err().map(|e| e.to_string()),
            attributes: ctx.remove_span_attributes(),
            control_plane_metadata: self.control_plane_metadata.clone(),
        }));
    }

    #[inline]
    async fn before_stage_inverse(&mut self, stage: &dyn Stage<C>) {
        if !self.has_traces {
            return;
        }
        let stage_name = stage.name();
        let stage_name_otel = format!("ferron.stage.{}.inverse", stage_name);
        let stage_key = self.stage_key(stage_name, true);
        self.keys
            .insert((stage_key.clone(), stage_name_otel.clone()));
        self.events.emit(Event::Trace(TraceEvent::StartSpan {
            key: Cow::Owned(stage_key),
            name: Cow::Owned(stage_name_otel),
            parent: Some(Parent::ByKey(self.parent_span_key.to_string())),
            trace_context: None,
            builder_attributes: vec![],
            attributes: vec![(
                "ferron.stage.name",
                TraceAttributeValue::String(stage_name.to_string()),
            )],
            links: vec![],
            control_plane_metadata: self.control_plane_metadata.clone(),
        }));
    }

    #[inline]
    async fn after_stage_inverse(
        &mut self,
        stage: &dyn Stage<C>,
        result: &Result<(), PipelineError>,
        ctx: &mut C,
    ) {
        if !self.has_traces {
            return;
        }
        let stage_name = stage.name();
        let stage_name_otel = format!("ferron.stage.{}.inverse", stage_name);
        let stage_key = self.stage_key(stage_name, true);
        self.keys
            .remove(&(stage_key.clone(), stage_name_otel.clone()));
        self.events.emit(Event::Trace(TraceEvent::EndSpan {
            key: Cow::Owned(stage_key),
            name: Cow::Owned(stage_name_otel),
            error: result.as_ref().err().map(|e| e.to_string()),
            attributes: ctx.remove_span_attributes(),
            control_plane_metadata: self.control_plane_metadata.clone(),
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
    pub client_ip: Option<String>,
    pub client_port: Option<u16>,
    pub client_ip_canonical: Option<String>,
    pub server_ip: Option<String>,
    pub server_port: Option<u16>,
    pub server_ip_canonical: Option<String>,
    pub auth_user: Option<String>,
    pub status: u16,
    pub content_length: Option<u64>,
    pub duration_secs: f64,
    pub request_headers: Vec<(String, String)>,
    pub timestamp: chrono::DateTime<chrono::Local>,
    pub trace_context: Option<EventTraceContext>,
    pub custom_fields: Option<FxHashMap<String, CustomAccessLogField>>,
    pub control_plane_metadata: Option<Arc<std::collections::BTreeMap<String, String>>>,
}

impl AccessEvent for HttpAccessLog {
    #[inline]
    fn protocol(&self) -> &'static str {
        "http"
    }

    #[inline]
    fn trace_context(&self) -> Option<&EventTraceContext> {
        self.trace_context.as_ref()
    }

    #[inline]
    fn event_time(&self) -> Option<std::time::SystemTime> {
        Some(self.timestamp.into())
    }

    #[inline]
    fn control_plane_metadata(&self) -> Option<&std::collections::BTreeMap<String, String>> {
        self.control_plane_metadata.as_deref()
    }

    #[inline]
    fn visit(&self, visitor: &mut dyn AccessVisitor) {
        visitor.field_string("path", &self.path);
        visitor.field_string("path_and_query", &self.path_and_query);
        visitor.field_string("method", &self.method);
        visitor.field_string("version", &self.version);
        visitor.field_string("scheme", &self.scheme);
        if let Some(f) = &self.client_ip {
            visitor.field_string("client_ip", f);
        }
        if let Some(f) = &self.client_port {
            visitor.field_u64("client_port", *f as u64);
        }
        if let Some(f) = &self.client_ip_canonical {
            visitor.field_string("client_ip_canonical", f);
        }
        if let Some(f) = &self.server_ip {
            visitor.field_string("server_ip", f);
        }
        if let Some(f) = &self.server_port {
            visitor.field_u64("server_port", *f as u64);
        }
        if let Some(f) = &self.server_ip_canonical {
            visitor.field_string("server_ip_canonical", f);
        }
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
            if SENSITIVE_FIELDS_REDACTED
                .iter()
                .any(|sfr| name.to_ascii_lowercase().contains(sfr))
            {
                // Don't add sensitive HTTP headers to protect the clients.
                continue;
            }
            visitor.field_string(
                &format!("header_{}", name.to_ascii_lowercase().replace("-", "_")),
                value,
            );
        }
        // Optionally include trace identifiers when available
        if let Some(trace_context) = &self.trace_context {
            if let (Ok(trace_id_str), Ok(span_id_str)) = (
                std::str::from_utf8(&trace_context.trace_id),
                std::str::from_utf8(&trace_context.span_id),
            ) {
                visitor.field_string("trace_id", trace_id_str);
                visitor.field_string("span_id", span_id_str);
            }
        }
        // Optionally include custom access log fields
        if let Some(custom_fields) = &self.custom_fields {
            for (name, value) in custom_fields.iter() {
                match value {
                    CustomAccessLogField::String(s) => visitor.field_string(name.as_str(), s),
                    CustomAccessLogField::U64(u) => visitor.field_u64(name.as_str(), *u),
                    CustomAccessLogField::F64(f) => visitor.field_f64(name.as_str(), *f),
                    CustomAccessLogField::Bool(b) => visitor.field_bool(name.as_str(), *b),
                }
            }
        }
    }
}

#[inline]
pub(super) fn next_span_key(prefix: &str) -> String {
    format!(
        "{prefix}:{}",
        SPAN_KEY_COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

#[inline]
pub fn resolve_request_trace_context(
    request: &HttpRequest,
    generate_enabled: bool,
    default_sampled: bool,
    trust_request: bool,
    has_traces: bool,
) -> (Option<trace_context::TraceContext>, Option<Parent>) {
    if trust_request {
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

        if let Some(mut context) = incoming {
            context.baggage = request
                .headers()
                .get("baggage")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);

            let parent_trace_id = context.trace_id.clone();
            let parent_span_id = context.span_id.clone();
            let parent_sampled = context.sampled;
            let baggage = context.baggage.clone();
            if has_traces {
                // Generate a new span ID for the request if tracing is enabled,
                // so to not break the trace context
                context.span_id = trace_context::generate_span_id();
            }
            return (
                Some(context),
                Some(Parent::ById {
                    trace_id: parent_trace_id,
                    span_id: parent_span_id,
                    sampled: Some(parent_sampled),
                    baggage,
                }),
            );
        }
    }

    if generate_enabled {
        let mut context = trace_context::generate_traceparent(default_sampled);
        context.baggage = request
            .headers()
            .get("baggage")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        return (Some(context), None);
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

/// Categorize an HTTP method into a bounded set for metric dimensions.
///
/// Standard methods are kept as-is; unknown methods are collapsed into `_other`
/// to prevent high-cardinality label explosion from custom/fuzzed HTTP methods.
fn categorize_http_method(method: &http::Method) -> &'static str {
    match *method {
        http::Method::GET => "GET",
        http::Method::HEAD => "HEAD",
        http::Method::POST => "POST",
        http::Method::PUT => "PUT",
        http::Method::DELETE => "DELETE",
        http::Method::CONNECT => "CONNECT",
        http::Method::OPTIONS => "OPTIONS",
        http::Method::TRACE => "TRACE",
        http::Method::PATCH => "PATCH",
        _ => "_other",
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
        MetricAttributeValue::StaticStr(categorize_http_method(request.method())),
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

#[inline]
pub fn get_error_log_attributes(
    error_type: &'static str,
    error_message: Option<String>,
    local_address: Option<std::net::SocketAddr>,
    remote_address: Option<std::net::SocketAddr>,
    unix_socket_path: Option<std::path::PathBuf>,
) -> Vec<(&'static str, LogAttributeValue)> {
    let mut attrs = Vec::with_capacity(6);
    attrs.push(("error.type", LogAttributeValue::StaticStr(error_type)));
    if let Some(error_message) = error_message {
        attrs.push(("error.message", LogAttributeValue::String(error_message)));
    }
    if let Some(remote_address) = remote_address {
        attrs.push((
            "client.address",
            LogAttributeValue::String(remote_address.ip().to_string()),
        ));
        attrs.push((
            "client.port",
            LogAttributeValue::I64(remote_address.port() as i64),
        ));
    }
    if let Some(local_address) = local_address {
        attrs.push((
            "server.address",
            LogAttributeValue::String(local_address.ip().to_string()),
        ));
        attrs.push((
            "server.port",
            LogAttributeValue::I64(local_address.port() as i64),
        ));
    }
    if let Some(unix_socket_path) = unix_socket_path {
        // OTel semantic conventions
        attrs.push((
            "server.address",
            LogAttributeValue::String(unix_socket_path.to_string_lossy().into_owned()),
        ));
    }

    attrs
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;

    #[test]
    fn method_cardinality_is_bounded() {
        // At most 10 unique label values (9 standard + 1 _other)
        let mut values = std::collections::HashSet::new();
        for method in &[
            http::Method::GET,
            http::Method::HEAD,
            http::Method::POST,
            http::Method::PUT,
            http::Method::DELETE,
            http::Method::CONNECT,
            http::Method::OPTIONS,
            http::Method::TRACE,
            http::Method::PATCH,
        ] {
            values.insert(categorize_http_method(method));
        }
        values.insert(categorize_http_method(
            &http::Method::from_bytes(b"CUSTOM").unwrap(),
        ));
        assert_eq!(values.len(), 10, "bounded to 9 standard + 1 _other");
    }

    #[test]
    fn build_metric_attributes_uses_bounded_method() {
        use bytes::Bytes;
        use http_body_util::Empty;

        let request = http::Request::builder()
            .method("FROBULATE")
            .body(
                Empty::<Bytes>::new()
                    .map_err(|_| unreachable!())
                    .boxed_unsync(),
            )
            .unwrap();

        let attrs = build_metric_attributes(&request, false, None);
        let method_attr = attrs
            .iter()
            .find(|(k, _)| *k == "http.request.method")
            .expect("method attr must exist");

        match &method_attr.1 {
            MetricAttributeValue::StaticStr(val) => assert_eq!(*val, "_other"),
            _ => panic!("method should be StaticStr after categorization"),
        }
    }
}
