mod file_pipeline;
mod pipeline;
mod request_utils;

use std::borrow::Cow;
use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use bytes::Bytes;
use ferron_core::config::layer::LayeredConfiguration;
use ferron_core::pipeline::{Pipeline, PipelineError, Stage, StageHooks};
use ferron_http::trace_context;
use ferron_http::variables::canonicalize_ip;
use ferron_http::{HttpContext, HttpErrorContext, HttpFileContext, HttpRequest, HttpResponse};
use ferron_observability::{
    AccessEvent, AccessVisitor, CompositeEventSink, Event, EventTraceContext, MetricAttributeValue,
    MetricEvent, MetricType, MetricValue, Parent, TraceAttributeValue, TraceEvent,
};
use http::{HeaderValue, Response};
use http_body_util::Empty;
use http_body_util::{combinators::UnsyncBoxBody, BodyExt};
use rustc_hash::FxHashMap;
use typemap_rev::TypeMap;

use crate::config::ThreeStageResolver;
use crate::util::canonicalize_cache::canonicalize_path_routing_cached;
use crate::util::canonicalize_url::canonicalize_path;

#[cfg(any(test, feature = "bench"))]
pub use self::file_pipeline::bench_resolve_http_file_target;
pub(crate) use self::file_pipeline::set_path_resolve_cache_ttl_millis;
use self::pipeline::*;
use self::request_utils::{
    add_http3_alt_svc_header, builtin_error_response, check_backslash_in_path, emit_error,
    emit_error_with_trace, execute_error_pipeline, get_http_nested_boolean,
    is_options_star_request, normalize_host_header, normalize_http2_http3_request,
    sanitize_request_url,
};

const LOG_TARGET: &str = "ferron-http-server";
static SPAN_KEY_COUNTER: AtomicU64 = AtomicU64::new(1);

type ResponseBody = UnsyncBoxBody<Bytes, io::Error>;

/// Per-stage hooks that emit trace spans around each pipeline stage.
pub(super) struct PerStageSpanHooks<'a> {
    events: &'a CompositeEventSink,
    has_traces: bool,
    parent_span_key: &'a str,
    stage_group: &'a str,
}

impl<'a> PerStageSpanHooks<'a> {
    fn new(
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

    fn stage_key(&self, stage_name: &str, inverse: bool) -> String {
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
struct HttpAccessLog {
    path: String,
    path_and_query: String,
    method: String,
    version: Cow<'static, str>,
    scheme: Cow<'static, str>,
    client_ip: String,
    client_port: u16,
    client_ip_canonical: String,
    server_ip: String,
    server_port: u16,
    server_ip_canonical: String,
    auth_user: Option<String>,
    status: u16,
    content_length: Option<u64>,
    duration_secs: f64,
    request_headers: Vec<(String, String)>,
    timestamp: chrono::DateTime<chrono::Local>,
    trace_context: Option<EventTraceContext>,
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

fn to_event_trace_context(trace_context: &trace_context::TraceContext) -> EventTraceContext {
    EventTraceContext {
        trace_id: trace_context.trace_id.clone(),
        span_id: trace_context.span_id.clone(),
        sampled: Some(trace_context.sampled),
    }
}

fn resolve_request_trace_context(
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
fn http_version_access_string(version: http::Version) -> &'static str {
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
fn build_metric_attributes(
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

pub async fn bad_request_handler(
    is_timeout: bool,
    error_pipeline: Arc<Pipeline<HttpErrorContext>>,
    events: CompositeEventSink,
) -> Result<Response<ResponseBody>, io::Error> {
    let status_code = if is_timeout { 408 } else { 400 };
    ferron_core::admin::ADMIN_METRICS
        .requests_total
        .fetch_add(1, Ordering::Relaxed);
    let request_span_key = events.has_trace_sinks().then(|| next_span_key("request"));
    if let Some(request_span_key) = request_span_key.as_ref() {
        events.emit(Event::Trace(TraceEvent::StartSpan {
            key: Cow::Owned(request_span_key.clone()),
            name: Cow::Borrowed("ferron.request"),
            parent: None,
            trace_context: None,
            attributes: vec![(
                "ferron.http.request.stage",
                TraceAttributeValue::StaticStr("pre_handler"),
            )],
        }));
    }
    emit_error(
        &events,
        format!(
            "{} request error: {}",
            status_code,
            if is_timeout {
                "request timed out"
            } else {
                "bad request"
            }
        ),
    );
    events.emit(Event::Metric(MetricEvent {
        name: "ferron.http.server.pre_handler_request_count",
        attributes: vec![
            (
                "http.response.status_code",
                MetricAttributeValue::I64(status_code as i64),
            ),
            (
                "ferron.http.request.stage",
                MetricAttributeValue::StaticStr("pre_handler"),
            ),
            (
                "error.type",
                MetricAttributeValue::String(status_code.to_string()),
            ),
        ],
        ty: MetricType::Counter,
        value: MetricValue::U64(1),
        unit: Some("{request}"),
        description: Some(
            "Number of malformed or timed-out HTTP requests rejected before request handling.",
        ),
    }));
    let mut response = if let Some(response) = execute_error_pipeline(
        error_pipeline.as_ref(),
        status_code,
        None,
        LayeredConfiguration::default(),
        &events,
        request_span_key.as_deref(),
    )
    .await
    {
        response
    } else {
        builtin_error_response(status_code, None, None)
    };
    response
        .headers_mut()
        .insert(http::header::SERVER, HeaderValue::from_static("Ferron"));
    if let Some(request_span_key) = request_span_key {
        events.emit(Event::Trace(TraceEvent::EndSpan {
            key: Cow::Owned(request_span_key),
            name: Cow::Borrowed("ferron.request"),
            error: Some(format!("HTTP error {}", status_code)),
            attributes: vec![(
                "http.response.status_code",
                TraceAttributeValue::I64(status_code as i64),
            )],
        }));
    }
    Ok(response)
}

#[allow(clippy::too_many_arguments)]
pub async fn request_handler(
    request: HttpRequest,
    pipeline: Arc<Pipeline<HttpContext>>,
    file_pipeline: Arc<Pipeline<HttpFileContext>>,
    error_pipeline: Arc<Pipeline<HttpErrorContext>>,
    config_resolver: Arc<ThreeStageResolver>,
    local_address: SocketAddr,
    remote_address: SocketAddr,
    hostname: Option<String>,
    encrypted: bool,
    http3_alt_svc: bool,
    https_port: Option<u16>,
    events: CompositeEventSink,
) -> Result<Response<ResponseBody>, io::Error> {
    let has_events = !events.is_empty();
    let has_traces = events.has_trace_sinks();

    let scheme: &'static str = if encrypted { "https" } else { "http" };
    // Build observability payloads from the original request before consuming it.
    let metric_attrs = has_events.then(|| build_metric_attributes(&request, encrypted, None));
    let method = has_events.then(|| request.method().clone());
    let path = has_events.then(|| request.uri().path().to_string());
    let path_and_query = has_events.then(|| {
        request
            .uri()
            .path_and_query()
            .map_or_else(|| request.uri().path().to_string(), |pq| pq.to_string())
    });
    let version = has_events.then(|| http_version_access_string(request.version()));
    let server_ip = has_events.then(|| local_address.ip().to_string());
    let server_port = has_events.then_some(local_address.port());
    let server_ip_canonical = has_events.then(|| canonicalize_ip(local_address.ip()));
    let initial_client_ip_canonical = has_events.then(|| canonicalize_ip(remote_address.ip()));

    let (request_trace_context, external_parent) = if has_traces {
        let global_config = config_resolver.global();
        let trace_config_node = global_config.as_ref().and_then(|g| {
            g.directives
                .get("http")
                .and_then(|entries| entries.first())
                .and_then(|e| e.children.as_ref())
                .and_then(|c| c.directives.get("trace"))
                .and_then(|entries| entries.first())
                .and_then(|e| e.children.as_ref())
        });

        let generate_enabled = trace_config_node
            .and_then(|c| c.get_value("generate"))
            .and_then(|v| v.as_boolean())
            .unwrap_or(true);

        let default_sampled = trace_config_node
            .and_then(|c| c.get_value("sampled"))
            .and_then(|v| v.as_boolean())
            .unwrap_or(false);

        resolve_request_trace_context(&request, generate_enabled, default_sampled)
    } else {
        (None, None)
    };
    let request_span_key = has_traces.then(|| next_span_key("request"));

    // Start tracing span
    if let Some(request_span_key) = request_span_key.as_ref() {
        let method = method
            .as_ref()
            .expect("trace events require request metadata to be initialized");
        let path = path
            .as_ref()
            .expect("trace events require request metadata to be initialized");
        let server_ip = server_ip
            .as_ref()
            .expect("trace events require request metadata to be initialized");
        let server_port =
            server_port.expect("trace events require request metadata to be initialized");
        let initial_client_ip_canonical = initial_client_ip_canonical
            .as_ref()
            .expect("trace events require request metadata to be initialized");

        events.emit(Event::Trace(TraceEvent::StartSpan {
            key: Cow::Owned(request_span_key.clone()),
            name: Cow::Borrowed("ferron.request"),
            parent: external_parent.clone(),
            trace_context: request_trace_context.as_ref().map(to_event_trace_context),
            attributes: vec![
                (
                    "http.request.method",
                    TraceAttributeValue::String(method.as_str().to_string()),
                ),
                ("url.path", TraceAttributeValue::String(path.clone())),
                ("url.scheme", TraceAttributeValue::StaticStr(scheme)),
                (
                    "server.address",
                    TraceAttributeValue::String(server_ip.clone()),
                ),
                ("server.port", TraceAttributeValue::I64(server_port as i64)),
                (
                    "client.address",
                    TraceAttributeValue::String(initial_client_ip_canonical.clone()),
                ),
                (
                    "url.full",
                    TraceAttributeValue::String(request.uri().path_and_query().map_or_else(
                        || request.uri().path().to_string(),
                        |path_and_query| path_and_query.to_string(),
                    )),
                ),
            ],
        }));
    }

    ferron_core::admin::ADMIN_METRICS
        .requests_total
        .fetch_add(1, Ordering::Relaxed);

    // Increment active requests counter
    if let Some(metric_attrs) = metric_attrs.as_ref() {
        events.emit(Event::Metric(MetricEvent {
            name: "http.server.active_requests",
            attributes: metric_attrs.clone(),
            ty: MetricType::UpDownCounter,
            value: MetricValue::I64(1),
            unit: Some("{request}"),
            description: Some("Number of active HTTP server requests."),
        }));
    }

    let request_timer = std::time::Instant::now();

    // Collect request headers before moving `request` into handler_inner
    // (only needed for access logging — skip when no access sinks are configured,
    // even if metrics/traces sinks are present)
    let request_headers: Vec<(String, String)> = if events.has_access_sinks() {
        request
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|v| (name.to_string(), v.to_string()))
            })
            .collect()
    } else {
        Vec::new()
    };

    let (mut response_result, auth_user, final_remote_address) = request_handler_inner(
        request,
        pipeline,
        file_pipeline,
        error_pipeline,
        config_resolver,
        local_address,
        remote_address,
        hostname.clone(),
        encrypted,
        https_port,
        request_trace_context.clone(),
        request_span_key.clone(),
        events.clone(),
    )
    .await;

    if let Some(metric_attrs) = metric_attrs {
        // Compute duration and extract response info only when some sink may consume them.
        let duration_secs = request_timer.elapsed().as_secs_f64();
        let timestamp = chrono::Local::now();

        // Use the potentially modified remote_address (e.g. from X-Forwarded-For stage)
        // for access log fields, falling back to the original if not provided.
        let effective_remote = final_remote_address.unwrap_or(remote_address);
        let client_ip = effective_remote.ip().to_string();
        let client_port = effective_remote.port();
        let client_ip_canonical = canonicalize_ip(effective_remote.ip());
        let (status_code, content_length) = match &response_result {
            Ok(r) => {
                let status = r.status().as_u16();
                let content_length = r
                    .headers()
                    .get(http::header::CONTENT_LENGTH)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse().ok());
                (status, content_length)
            }
            Err(_) => (500, None),
        };

        // Build request_count-specific attributes
        let mut request_count_attrs = metric_attrs.clone();
        request_count_attrs.push((
            "http.response.status_code",
            MetricAttributeValue::I64(status_code as i64),
        ));
        if status_code >= 400 {
            request_count_attrs.push((
                "error.type",
                MetricAttributeValue::String(status_code.to_string()),
            ));
        }

        // Build duration-specific attributes (includes status_code for OTel compliance)
        let mut duration_attrs = metric_attrs.clone();
        duration_attrs.push((
            "http.response.status_code",
            MetricAttributeValue::I64(status_code as i64),
        ));
        if status_code >= 400 {
            duration_attrs.push((
                "error.type",
                MetricAttributeValue::String(status_code.to_string()),
            ));
        }

        // Decrement active requests
        events.emit(Event::Metric(MetricEvent {
            name: "http.server.active_requests",
            attributes: metric_attrs.clone(),
            ty: MetricType::UpDownCounter,
            value: MetricValue::I64(-1),
            unit: Some("{request}"),
            description: Some("Number of active HTTP server requests."),
        }));

        // Emit request duration histogram
        events.emit(Event::Metric(MetricEvent {
            name: "http.server.request.duration",
            attributes: duration_attrs,
            ty: MetricType::Histogram(Some(vec![
                0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1.0, 2.5, 5.0, 7.5, 10.0,
            ])),
            value: MetricValue::F64(duration_secs),
            unit: Some("s"),
            description: Some("Duration of HTTP server requests."),
        }));

        // Emit request count
        events.emit(Event::Metric(MetricEvent {
            name: "ferron.http.server.request_count",
            attributes: request_count_attrs,
            ty: MetricType::Counter,
            value: MetricValue::U64(1),
            unit: Some("{request}"),
            description: Some("Number of HTTP server requests."),
        }));

        // Emit access log
        events.emit(Event::Access(Arc::new(HttpAccessLog {
            path: path.expect("request metadata should be initialized when events are enabled"),
            path_and_query: path_and_query
                .expect("request metadata should be initialized when events are enabled"),
            method: method
                .expect("request metadata should be initialized when events are enabled")
                .as_str()
                .to_string(),
            version: Cow::Borrowed(
                version.expect("request metadata should be initialized when events are enabled"),
            ),
            scheme: Cow::Borrowed(scheme),
            client_ip,
            client_port,
            client_ip_canonical,
            server_ip: server_ip
                .expect("request metadata should be initialized when events are enabled"),
            server_port: server_port
                .expect("request metadata should be initialized when events are enabled"),
            server_ip_canonical: server_ip_canonical
                .expect("request metadata should be initialized when events are enabled"),
            auth_user,
            status: status_code,
            content_length,
            duration_secs,
            request_headers,
            timestamp,
            trace_context: request_trace_context.as_ref().map(to_event_trace_context),
        })));

        if let Some(request_span_key) = request_span_key {
            let error_description = response_result.as_ref().err().map(|e| e.to_string());
            let mut end_attrs = Vec::with_capacity(3);
            end_attrs.push((
                "http.response.status_code",
                TraceAttributeValue::I64(status_code as i64),
            ));
            end_attrs.push((
                "http.route",
                TraceAttributeValue::String(hostname.as_deref().unwrap_or("*").to_string()),
            ));
            if status_code >= 400 {
                end_attrs.push((
                    "error.type",
                    TraceAttributeValue::String(status_code.to_string()),
                ));
            }
            events.emit(Event::Trace(TraceEvent::EndSpan {
                key: Cow::Owned(request_span_key),
                name: Cow::Borrowed("ferron.request"),
                error: error_description,
                attributes: end_attrs,
            }));
        }
    }

    if let Ok(response) = &mut response_result {
        add_http3_alt_svc_header(
            response.headers_mut(),
            if http3_alt_svc && encrypted {
                Some(https_port.unwrap_or(local_address.port()))
            } else {
                None
            },
        );
        response
            .headers_mut()
            .insert(http::header::SERVER, HeaderValue::from_static("Ferron"));
    }
    response_result
}

#[allow(clippy::too_many_arguments)]
async fn request_handler_inner(
    mut request: HttpRequest,
    pipeline: Arc<Pipeline<HttpContext>>,
    file_pipeline: Arc<Pipeline<HttpFileContext>>,
    error_pipeline: Arc<Pipeline<HttpErrorContext>>,
    config_resolver: Arc<ThreeStageResolver>,
    local_address: SocketAddr,
    remote_address: SocketAddr,
    hostname: Option<String>,
    encrypted: bool,
    https_port: Option<u16>,
    request_trace_context: Option<trace_context::TraceContext>,
    request_span_key: Option<String>,
    events: CompositeEventSink,
) -> (
    Result<Response<ResponseBody>, io::Error>,
    Option<String>,
    Option<SocketAddr>,
) {
    // Normalize HTTP/2 and HTTP/3 requests
    if matches!(
        request.version(),
        http::Version::HTTP_2 | http::Version::HTTP_3
    ) {
        normalize_http2_http3_request(&mut request);
    }

    // Normalize "Host" header
    let request_log_trace_context = request_trace_context
        .as_ref()
        .map(trace_context::to_event_trace_context);
    if let Err(e) = normalize_host_header(&mut request, &events) {
        emit_error_with_trace(
            &events,
            format!("Host header normalization error: {}", e),
            request_log_trace_context.clone(),
        );
        if let Some(response) = execute_error_pipeline(
            error_pipeline.as_ref(),
            400,
            None,
            LayeredConfiguration::default(),
            &events,
            request_span_key.as_deref(),
        )
        .await
        {
            return (Ok(response), None, None);
        }
        return (
            Ok(builtin_error_response(
                400,
                None,
                config_resolver.global().and_then(|g| {
                    g.get_value("admin_email")
                        .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
                }),
            )),
            None,
            None,
        );
    }

    // Decode location for configuration resolution (routing-only, compute forwarding lazily)
    let (routing_str, _original_str) = match canonicalize_path_routing_cached(request.uri().path())
    {
        Ok((routing, original)) => (routing, original),
        Err(e) => {
            emit_error_with_trace(
                &events,
                format!("Invalid request URL pathname: {}", e),
                request_log_trace_context.clone(),
            );
            if let Some(response) = execute_error_pipeline(
                error_pipeline.as_ref(),
                400,
                None,
                LayeredConfiguration::default(),
                &events,
                request_span_key.as_deref(),
            )
            .await
            {
                return (Ok(response), None, None);
            }
            return (
                Ok(builtin_error_response(
                    400,
                    None,
                    config_resolver.global().and_then(|g| {
                        g.get_value("admin_email")
                            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
                    }),
                )),
                None,
                None,
            );
        }
    };

    // Reject backslashes in URL (unless disabled by configuration)
    let reject_backslash = config_resolver
        .global()
        .and_then(|g| get_http_nested_boolean(&g, "url_reject_backslash"))
        .unwrap_or(true);
    if let Err(e) = check_backslash_in_path(request.uri().path(), reject_backslash) {
        emit_error_with_trace(
            &events,
            format!("Invalid request URL: {}", e),
            request_log_trace_context.clone(),
        );
        if let Some(response) = execute_error_pipeline(
            error_pipeline.as_ref(),
            400,
            None,
            LayeredConfiguration::default(),
            &events,
            request_span_key.as_deref(),
        )
        .await
        {
            return (Ok(response), None, None);
        }
        return (
            Ok(builtin_error_response(
                400,
                None,
                config_resolver.global().and_then(|g| {
                    g.get_value("admin_email")
                        .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
                }),
            )),
            None,
            None,
        );
    }

    // Sanitize URL (unless disabled by configuration)
    let url_sanitize_enabled = config_resolver
        .global()
        .and_then(|g| get_http_nested_boolean(&g, "url_sanitize"))
        .unwrap_or(true);
    if url_sanitize_enabled {
        // Compute full canonicalized path (forwarding) only when sanitization is enabled.
        match canonicalize_path(request.uri().path()) {
            Ok(full_path) => {
                if let Err(e) = sanitize_request_url(&mut request, &full_path.forwarding) {
                    emit_error_with_trace(
                        &events,
                        format!("URL sanitization error: {}", e),
                        request_log_trace_context.clone(),
                    );
                    if let Some(response) = execute_error_pipeline(
                        error_pipeline.as_ref(),
                        400,
                        None,
                        LayeredConfiguration::default(),
                        &events,
                        request_span_key.as_deref(),
                    )
                    .await
                    {
                        return (Ok(response), None, None);
                    }
                    return (
                        Ok(builtin_error_response(
                            400,
                            None,
                            config_resolver.global().and_then(|g| {
                                g.get_value("admin_email")
                                    .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
                            }),
                        )),
                        None,
                        None,
                    );
                }
            }
            Err(e) => {
                emit_error_with_trace(
                    &events,
                    format!("Invalid request URL percent-encoding: {}", e),
                    request_log_trace_context.clone(),
                );
                if let Some(response) = execute_error_pipeline(
                    error_pipeline.as_ref(),
                    400,
                    None,
                    LayeredConfiguration::default(),
                    &events,
                    request_span_key.as_deref(),
                )
                .await
                {
                    return (Ok(response), None, None);
                }
                return (
                    Ok(builtin_error_response(
                        400,
                        None,
                        config_resolver.global().and_then(|g| {
                            g.get_value("admin_email")
                                .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
                        }),
                    )),
                    None,
                    None,
                );
            }
        }
    }

    // Create a partial HttpContext for variable resolution during config resolution.
    // This enables all interpolation variables (request.*, server.*, remote.*) to be
    // resolved dynamically from the context rather than pre-populated in a HashMap.
    let mut ctx = HttpContext {
        req: Some(request),
        res: None,
        events: events.clone(),
        configuration: LayeredConfiguration::default(),
        hostname: hostname.clone(),
        variables: FxHashMap::default(),
        previous_error: None,
        original_uri: None,
        routing_uri: routing_str.parse().ok(),
        encrypted,
        local_address,
        remote_address,
        auth_user: None,
        https_port,
        extensions: TypeMap::new(),
    };

    // Attach parsed or generated trace context to the HttpContext extensions so stages/modules can access it.
    if let Some(ref tc) = request_trace_context {
        ctx.insert::<trace_context::TraceContextKey>(tc.clone());
    }

    // When starting the top-level request span later, prefer external_parent if available
    let resolution = config_resolver.resolve(
        local_address.ip(),
        hostname.as_deref().unwrap_or(""),
        &routing_str,
        &ctx,
    );

    let Some(resolution) = resolution else {
        if let Some(response) = execute_error_pipeline(
            error_pipeline.as_ref(),
            404,
            None,
            LayeredConfiguration::default(),
            &events,
            request_span_key.as_deref(),
        )
        .await
        {
            return (Ok(response), None, None);
        }
        return (
            Ok(builtin_error_response(
                404,
                None,
                config_resolver.global().and_then(|g| {
                    g.get_value("admin_email")
                        .and_then(|v| v.as_string_with_interpolations(&ctx))
                }),
            )),
            None,
            None,
        );
    };

    // Fill in the resolved configuration
    ctx.configuration = resolution.configuration.clone();

    // Handle OPTIONS * requests (RFC 2616 Section 9.2)
    // Early response before pipeline execution
    if is_options_star_request(ctx.req.as_ref().expect("invalid HTTP context state")) {
        let allow_header = resolution
            .configuration
            .get_value("options_allowed_methods", false)
            .and_then(|v| v.as_string_with_interpolations(&ctx))
            .unwrap_or_else(|| "GET, HEAD, POST, OPTIONS".to_string());

        let response = Response::builder()
            .status(200)
            .header("Allow", &allow_header)
            .header("Content-Length", "0")
            .body(Empty::<Bytes>::new().map_err(|e| match e {}).boxed_unsync())
            .expect("failed to build OPTIONS * response");

        return (Ok(response), None, None);
    }

    let request = ctx.req.take().expect("invalid HTTP context state");
    let request_uri = request.uri().clone();
    let (request_parts, body) = request.into_parts();
    let cloned_request = http::Request::from_parts(
        request_parts.clone(),
        Empty::<Bytes>::new().map_err(|e| match e {}).boxed_unsync(),
    );
    let request = http::Request::from_parts(request_parts, body);

    let admin_email = resolution
        .configuration
        .get_value("admin_email", false)
        .and_then(|v| v.as_string_with_interpolations(&ctx));
    let resolution_configuration2 = resolution.configuration.clone();
    ctx.req = Some(request);
    ctx.original_uri = Some(request_uri);

    execute_pipeline_stages(
        &mut ctx,
        pipeline.as_ref(),
        file_pipeline.as_ref(),
        &events,
        "",
        &resolution.location_path.path_segments,
        request_span_key.as_deref(),
    )
    .await;

    // Handle error configurations for 4xx and 5xx responses
    if let HttpResponse::BuiltinError(status, _) = ctx
        .res
        .as_ref()
        .unwrap_or(&HttpResponse::BuiltinError(404, None))
    {
        let status = *status;
        if status >= 400 {
            ctx.previous_error = Some(status);
            ctx.req = Some(cloned_request);
            // Rebuild the resolver request from the current request in context
            if let Some(req) = ctx.req.take() {
                // Preserve the request for error resolution
                ctx.req = Some(req);
                let error_resolution = config_resolver.resolve_error_scoped(
                    local_address.ip(),
                    ctx.hostname.as_deref().unwrap_or(""),
                    &routing_str,
                    status,
                    &ctx,
                );

                if let Some(error_resolution) = error_resolution {
                    let execute_error_config = if let (Some(config1), Some(config2)) = (
                        error_resolution.configuration.layers.last(),
                        resolution_configuration2.layers.last(),
                    ) {
                        !Arc::ptr_eq(config1, config2)
                    } else {
                        false
                    };
                    if execute_error_config {
                        ctx.configuration = error_resolution.configuration;
                        ctx.res = None;

                        execute_pipeline_stages(
                            &mut ctx,
                            pipeline.as_ref(),
                            file_pipeline.as_ref(),
                            &events,
                            "Error ",
                            &resolution.location_path.path_segments,
                            request_span_key.as_deref(),
                        )
                        .await;
                    }
                }
            }
        }
    }

    let auth_user = ctx.auth_user.clone();
    let final_remote = ctx.remote_address;
    (
        match ctx.res.unwrap_or(HttpResponse::BuiltinError(404, None)) {
            HttpResponse::Custom(response) => Ok(response),
            HttpResponse::BuiltinError(status, headers) => {
                if let Some(response) = execute_error_pipeline(
                    error_pipeline.as_ref(),
                    status,
                    headers.clone(),
                    ctx.configuration.clone(),
                    &events,
                    request_span_key.as_deref(),
                )
                .await
                {
                    Ok(response)
                } else {
                    Ok(builtin_error_response(
                        status,
                        headers.as_ref(),
                        admin_email,
                    ))
                }
            }
            HttpResponse::Abort => Err(io::Error::other("Aborted")),
        },
        auth_user,
        Some(final_remote),
    )
}
