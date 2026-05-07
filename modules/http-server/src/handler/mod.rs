mod file_pipeline;
mod request_utils;

use std::borrow::Cow;
use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use ferron_core::config::layer::LayeredConfiguration;
use ferron_core::pipeline::{Pipeline, PipelineError, Stage, StageHooks};
use ferron_core::util::parse_duration;
use ferron_http::trace_context;
use ferron_http::variables::canonicalize_ip;
use ferron_http::{HttpContext, HttpErrorContext, HttpFileContext, HttpRequest, HttpResponse};
use ferron_observability::{
    AccessEvent, AccessVisitor, CompositeEventSink, Event, MetricAttributeValue, MetricEvent,
    MetricType, MetricValue, Parent, TraceAttributeValue, TraceEvent,
};
use http::{HeaderValue, Response};
use http_body_util::Empty;
use http_body_util::{combinators::UnsyncBoxBody, BodyExt};
use rustc_hash::FxHashMap;
use typemap_rev::TypeMap;

use crate::config::ThreeStageResolver;
use crate::util::canonicalize_cache::canonicalize_path_routing_cached;
use crate::util::canonicalize_url::canonicalize_path;

use self::file_pipeline::{
    execute_http_file_pipeline, strip_matched_path_prefix, FilePipelineExecutionError,
};
use self::request_utils::{
    add_http3_alt_svc_header, builtin_error_response, check_backslash_in_path, emit_error,
    emit_warn, execute_error_pipeline, get_http_nested_boolean, is_options_star_request,
    normalize_host_header, normalize_http2_http3_request, sanitize_request_url,
};

const LOG_TARGET: &str = "ferron-http-server";

type ResponseBody = UnsyncBoxBody<Bytes, io::Error>;

pub fn set_path_resolve_cache_ttl_millis(ms: u64) {
    file_pipeline::set_path_resolve_cache_ttl_millis(ms);
}

#[cfg(any(test, feature = "bench"))]
pub async fn bench_resolve_http_file_target(
    root_path: &std::path::Path,
    request_path: &str,
    index_files: Option<&[String]>,
) -> Result<bool, String> {
    file_pipeline::bench_resolve_http_file_target(root_path, request_path, index_files).await
}

/// Per-stage hooks that emit trace spans around each pipeline stage.
struct PerStageSpanHooks<'a> {
    events: &'a CompositeEventSink,
    has_traces: bool,
}

#[async_trait::async_trait(?Send)]
impl StageHooks<HttpContext> for PerStageSpanHooks<'_> {
    #[inline]
    async fn before_stage(&mut self, stage: &dyn Stage<HttpContext>) {
        if !self.has_traces {
            return;
        }
        let stage_name = stage.name();
        self.events.emit(Event::Trace(TraceEvent::StartSpan {
            name: Cow::Owned(format!("ferron.stage.{}", stage_name)),
            parent: None,
            attributes: vec![(
                "stage.name",
                TraceAttributeValue::String(stage_name.to_string()),
            )],
        }));
    }

    #[inline]
    async fn after_stage(
        &mut self,
        stage: &dyn Stage<HttpContext>,
        result: &Result<bool, PipelineError>,
    ) {
        if !self.has_traces {
            return;
        }
        self.events.emit(Event::Trace(TraceEvent::EndSpan {
            name: Cow::Owned(format!("ferron.stage.{}", stage.name())),
            error: result.as_ref().err().map(|e| e.to_string()),
            attributes: vec![],
        }));
    }

    #[inline]
    async fn before_stage_inverse(&mut self, stage: &dyn Stage<HttpContext>) {
        if !self.has_traces {
            return;
        }
        let stage_name = stage.name();
        self.events.emit(Event::Trace(TraceEvent::StartSpan {
            name: Cow::Owned(format!("ferron.stage.{}.inverse", stage_name)),
            parent: None,
            attributes: vec![(
                "stage.name",
                TraceAttributeValue::String(stage_name.to_string()),
            )],
        }));
    }

    #[inline]
    async fn after_stage_inverse(
        &mut self,
        stage: &dyn Stage<HttpContext>,
        result: &Result<(), PipelineError>,
    ) {
        if !self.has_traces {
            return;
        }
        self.events.emit(Event::Trace(TraceEvent::EndSpan {
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
}

impl AccessEvent for HttpAccessLog {
    fn protocol(&self) -> &'static str {
        "http"
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
    }
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
    let mut response = if let Some(response) = execute_error_pipeline(
        error_pipeline.as_ref(),
        status_code,
        None,
        LayeredConfiguration::default(),
        &events,
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

    // Determine external parent from incoming headers if present (do not generate here)
    let mut external_parent_outer: Option<Parent> = None;
    if has_traces {
        if let Some(tp_val) = request.headers().get("traceparent") {
            if let Ok(tp_str) = tp_val.to_str() {
                if let Some(tc) = trace_context::parse_traceparent(tp_str) {
                    external_parent_outer = Some(Parent::ById {
                        trace_id: tc.trace_id.clone(),
                        span_id: tc.span_id.clone(),
                    });
                }
            }
        }
    }

    // Start tracing span
    if has_traces {
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
            name: Cow::Borrowed("ferron.request_handler"),
            parent: external_parent_outer.clone(),
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
            ],
        }));
    }

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
        })));

        if has_traces {
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
                name: Cow::Borrowed("ferron.request_handler"),
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
    events: CompositeEventSink,
) -> (
    Result<Response<ResponseBody>, io::Error>,
    Option<String>,
    Option<SocketAddr>,
) {
    // Increment request counter for admin API /status endpoint
    ferron_core::admin::ADMIN_METRICS
        .requests_total
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    // Normalize HTTP/2 and HTTP/3 requests
    if matches!(
        request.version(),
        http::Version::HTTP_2 | http::Version::HTTP_3
    ) {
        normalize_http2_http3_request(&mut request);
    }

    // Normalize "Host" header
    if let Err(e) = normalize_host_header(&mut request, &events) {
        emit_error(&events, format!("Host header normalization error: {}", e));
        if let Some(response) = execute_error_pipeline(
            error_pipeline.as_ref(),
            400,
            None,
            LayeredConfiguration::default(),
            &events,
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
            emit_error(
                &events,
                format!("Invalid request URL percent-encoding: {}", e),
            );
            if let Some(response) = execute_error_pipeline(
                error_pipeline.as_ref(),
                400,
                None,
                LayeredConfiguration::default(),
                &events,
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
        emit_error(&events, format!("Invalid request URL: {}", e));
        if let Some(response) = execute_error_pipeline(
            error_pipeline.as_ref(),
            400,
            None,
            LayeredConfiguration::default(),
            &events,
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
                    emit_error(&events, format!("URL sanitization error: {}", e));
                    if let Some(response) = execute_error_pipeline(
                        error_pipeline.as_ref(),
                        400,
                        None,
                        LayeredConfiguration::default(),
                        &events,
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
                emit_error(
                    &events,
                    format!("Invalid request URL percent-encoding: {}", e),
                );
                if let Some(response) = execute_error_pipeline(
                    error_pipeline.as_ref(),
                    400,
                    None,
                    LayeredConfiguration::default(),
                    &events,
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
    // Parse incoming W3C traceparent (trace-context) headers early so spans can be started with external parents
    let mut parsed_trace_context: Option<trace_context::TraceContext> = None;
    let mut external_parent: Option<Parent> = None;
    if events.has_trace_sinks() {
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

        if let Some(tp_val) = request.headers().get("traceparent") {
            if let Ok(tp_str) = tp_val.to_str() {
                if let Some(tc) = trace_context::parse_traceparent(tp_str) {
                    external_parent = Some(Parent::ById {
                        trace_id: tc.trace_id.clone(),
                        span_id: tc.span_id.clone(),
                    });
                    parsed_trace_context = Some(tc);
                }
            }
        }
        // If no incoming context, generate a new one if enabled.
        if parsed_trace_context.is_none() && generate_enabled {
            let gen = trace_context::generate_traceparent(default_sampled);
            external_parent = Some(Parent::ById {
                trace_id: gen.trace_id.clone(),
                span_id: gen.span_id.clone(),
            });
            parsed_trace_context = Some(gen);
        }
    }

    // Start the request span with external parent if available
    if events.has_trace_sinks() {
        events.emit(Event::Trace(TraceEvent::StartSpan {
            name: std::borrow::Cow::Borrowed("ferron.request"),
            parent: external_parent,
            attributes: vec![
                (
                    "http.request.method",
                    TraceAttributeValue::String(request.method().to_string()),
                ),
                (
                    "url.full",
                    TraceAttributeValue::String(request.uri().to_string()),
                ),
            ],
        }));
    }

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
    if let Some(tc) = parsed_trace_context {
        ctx.insert::<trace_context::TraceContextKey>(tc);
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
                        )
                        .await;
                    }
                }
            }
        }
    }

    let auth_user = ctx.auth_user.clone();
    let final_remote = ctx.remote_address;

    if events.has_trace_sinks() {
        let status = match ctx
            .res
            .as_ref()
            .unwrap_or(&HttpResponse::BuiltinError(404, None))
        {
            HttpResponse::BuiltinError(status, _) => *status as i64,
            HttpResponse::Custom(resp) => resp.status().as_u16() as i64,
            HttpResponse::Abort => 0,
        };
        events.emit(Event::Trace(TraceEvent::EndSpan {
            name: std::borrow::Cow::Borrowed("ferron.request"),
            attributes: vec![(
                "http.response.status_code",
                TraceAttributeValue::I64(status),
            )],
            error: if status >= 400 {
                Some(format!("HTTP error {}", status))
            } else {
                None
            },
        }));
    }

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

async fn execute_pipeline_stages(
    ctx: &mut HttpContext,
    pipeline: &Pipeline<HttpContext>,
    file_pipeline: &Pipeline<HttpFileContext>,
    events: &CompositeEventSink,
    log_prefix: &str,
    path_segments: &[String],
) {
    let has_traces = events.has_trace_sinks();

    // Start pipeline execution span
    if has_traces {
        events.emit(Event::Trace(TraceEvent::StartSpan {
            name: Cow::Borrowed("ferron.pipeline.execute"),
            parent: Some(Parent::ByName("ferron.request".to_string())),
            attributes: vec![(
                "ferron.pipeline.log_prefix",
                TraceAttributeValue::String(log_prefix.to_string()),
            )],
        }));
    }

    // Remove the base URL if path segments were matched
    if !path_segments.is_empty() {
        if let Some(req) = ctx.req.take() {
            let (mut parts, body) = req.into_parts();
            let mut uri_parts = parts.uri.into_parts();
            if let Some(path_and_query) = uri_parts.path_and_query {
                uri_parts.path_and_query =
                    strip_matched_path_prefix(&path_and_query, path_segments.len());
                if uri_parts.path_and_query.is_none() {
                    ctx.res = Some(HttpResponse::BuiltinError(400, None));
                    return;
                }
            }
            let Ok(new_uri) = http::Uri::from_parts(uri_parts) else {
                ctx.res = Some(HttpResponse::BuiltinError(400, None));
                return;
            };
            parts.uri = new_uri;
            ctx.req = Some(http::Request::from_parts(parts, body));
        }
    }

    let timeout_duration = ctx.configuration.get_value("timeout", false).map_or(
        Some(Duration::from_secs(300)),
        |value| {
            if !value.as_boolean().unwrap_or(true) {
                None
            } else if let Some(s) = value.as_string_with_interpolations(&HashMap::new()) {
                match parse_duration(&s) {
                    Ok(d) => Some(d),
                    Err(e) => {
                        ferron_core::log_warn!("Invalid timeout duration '{}': {}", s, e);
                        Some(Duration::from_secs(300))
                    }
                }
            } else {
                value
                    .as_number()
                    .map(|n| Duration::from_millis(n as u64))
                    .or_else(|| Some(Duration::from_secs(300)))
            }
        },
    );
    let instant = std::time::Instant::now();

    // Per-stage span hooks — emit StartSpan/EndSpan around each stage
    let mut stage_hooks = PerStageSpanHooks { events, has_traces };

    let executed_stages = match if let Some(timeout_duration) =
        timeout_duration.map(|d| d.saturating_sub(instant.elapsed()))
    {
        vibeio::time::timeout(
            timeout_duration,
            pipeline.execute_without_inverse_with_hooks(ctx, &mut stage_hooks),
        )
        .await
    } else {
        Ok(pipeline
            .execute_without_inverse_with_hooks(ctx, &mut stage_hooks)
            .await)
    } {
        Ok(Ok(executed_stages)) => Some(executed_stages),
        Ok(Err(error)) => {
            emit_error(
                events,
                format!("{log_prefix}Pipeline execution error: {error}"),
            );
            ctx.res = Some(HttpResponse::BuiltinError(500, None));
            None
        }
        Err(_) => {
            emit_error(events, format!("{log_prefix}Pipeline execution timeout"));
            ctx.res = Some(HttpResponse::BuiltinError(408, None));
            None
        }
    };

    if let Some(executed_stages) = executed_stages {
        if ctx.res.is_none() {
            match execute_http_file_pipeline(
                ctx,
                file_pipeline,
                timeout_duration.map(|d| d.saturating_sub(instant.elapsed())),
            )
            .await
            {
                Ok(()) => {}
                Err(FilePipelineExecutionError::Forbidden) => {
                    ctx.res = Some(HttpResponse::BuiltinError(403, None));
                }
                Err(FilePipelineExecutionError::BadRequest) => {
                    ctx.res = Some(HttpResponse::BuiltinError(400, None));
                }
                Err(FilePipelineExecutionError::Timeout) => {
                    ctx.res = Some(HttpResponse::BuiltinError(404, None));
                }
                Err(FilePipelineExecutionError::Io(error)) => {
                    emit_error(
                        events,
                        format!("{log_prefix}HTTP file resolution error: {error}"),
                    );
                    ctx.res = Some(HttpResponse::BuiltinError(500, None));
                }
                Err(FilePipelineExecutionError::Pipeline(error)) => {
                    emit_error(
                        events,
                        format!("{log_prefix}Pipeline execution error: {error}"),
                    );
                    ctx.res = Some(HttpResponse::BuiltinError(500, None));
                }
                Err(FilePipelineExecutionError::WebrootNotFound) => {
                    if let Some(webroot) = ctx
                        .configuration
                        .get_value("root", true)
                        .and_then(|v| v.as_string_with_interpolations(ctx))
                    {
                        emit_warn(events, format!("{log_prefix}Webroot not found: {webroot}"));
                    }
                    ctx.res = Some(HttpResponse::BuiltinError(404, None));
                }
            }
        }

        if let Err(error) = pipeline
            .execute_inverse_with_hooks(ctx, executed_stages, &mut stage_hooks)
            .await
        {
            emit_error(
                events,
                format!("{log_prefix}Pipeline inverse execution error: {error}"),
            );
            ctx.res = Some(HttpResponse::BuiltinError(500, None));
        }
    }

    // End pipeline execution span
    if has_traces {
        events.emit(Event::Trace(TraceEvent::EndSpan {
            name: Cow::Borrowed("ferron.pipeline.execute"),
            error: ctx.res.as_ref().and_then(|r| match r {
                HttpResponse::BuiltinError(s, _) if *s >= 400 => {
                    Some(format!("builtin error {}", s))
                }
                _ => None,
            }),
            attributes: vec![],
        }));
    }
}
