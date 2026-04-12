use std::borrow::Cow;
use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use ferron_core::config::layer::LayeredConfiguration;
use ferron_core::pipeline::{Pipeline, PipelineError, Stage, StageHooks};
use ferron_core::util::parse_duration;
use ferron_http::{HttpContext, HttpErrorContext, HttpFileContext, HttpRequest, HttpResponse};
use ferron_observability::{
    AccessEvent, AccessVisitor, CompositeEventSink, Event, LogEvent, LogLevel,
    MetricAttributeValue, MetricEvent, MetricType, MetricValue, TraceAttributeValue, TraceEvent,
};
use http::{HeaderMap, HeaderValue, Response, StatusCode};
use http_body_util::Empty;
use http_body_util::{combinators::UnsyncBoxBody, BodyExt, Full};
use rustc_hash::FxHashMap;
use typemap_rev::TypeMap;

use crate::config::ThreeStageResolver;
use crate::util::canonicalize_url::canonicalize_path;
use crate::util::canonicalize_cache::canonicalize_path_routing_cached;
use crate::util::error_pages::generate_default_error_page;

const LOG_TARGET: &str = "ferron-http-server";

type ResponseBody = UnsyncBoxBody<Bytes, io::Error>;

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
            parent_span_id: None,
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
            parent_span_id: None,
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

/// Cache for path canonicalization results.
/// Keys: (canonical_root, request_path), Value: Timestamped<ResolvedHttpFile>
/// TTL default: 100 milliseconds to balance performance with filesystem change detection.
static PATH_RESOLVE_CACHE_TTL_MILLIS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(100);

fn path_resolve_cache_ttl() -> Duration {
    Duration::from_millis(PATH_RESOLVE_CACHE_TTL_MILLIS.load(std::sync::atomic::Ordering::Relaxed))
}

pub fn set_path_resolve_cache_ttl_millis(ms: u64) {
    PATH_RESOLVE_CACHE_TTL_MILLIS.store(ms, std::sync::atomic::Ordering::Relaxed);
}

static PATH_RESOLVE_CACHE: std::sync::LazyLock<
    quick_cache::sync::Cache<
        (PathBuf, String),
        Timestamped<ResolvedHttpFile>,
        PathResolveCacheWeighter,
    >,
> = std::sync::LazyLock::new(|| {
    quick_cache::sync::Cache::with_weighter(
        1024,             // initial capacity
        64 * 1024 * 1024, // max weight (approx 64MB)
        PathResolveCacheWeighter,
    )
});

/// Wraps a value with an insertion timestamp for TTL-based expiry.
#[derive(Debug, Clone)]
struct Timestamped<T> {
    inserted_at: Instant,
    value: T,
}

impl<T> Timestamped<T> {
    fn new(value: T) -> Self {
        Self {
            inserted_at: Instant::now(),
            value,
        }
    }

    fn is_expired(&self, ttl: Duration) -> bool {
        self.inserted_at.elapsed() >= ttl
    }
}

/// Weighter for the path resolve cache.
/// Each entry costs approximately: size_of_key + size_of_value + overhead
#[derive(Clone)]
struct PathResolveCacheWeighter;

impl quick_cache::Weighter<(PathBuf, String), Timestamped<ResolvedHttpFile>>
    for PathResolveCacheWeighter
{
    fn weight(&self, key: &(PathBuf, String), val: &Timestamped<ResolvedHttpFile>) -> u64 {
        let key_size = key.0.as_os_str().len() + key.1.len();
        let value_size = val.value.file_path.as_os_str().len()
            + val.value.path_info.as_ref().map_or(0, |s| s.len())
            + val.value.etag.len()
            + size_of::<vibeio::fs::Metadata>();
        (key_size + value_size) as u64
    }
}

#[derive(Debug, Clone)]
struct ResolvedHttpFile {
    metadata: vibeio::fs::Metadata,
    file_path: PathBuf,
    path_info: Option<String>,
    /// Pre-computed ETag at cache insertion time, avoiding recomputation per request.
    etag: String,
}

impl ResolvedHttpFile {
    /// Compute the ETag from the file path, size, and modification time.
    fn compute_etag(&self) -> String {
        let mtime_secs = self
            .metadata
            .modified()
            .ok()
            .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let cache_key = format!(
            "{}-{}-{}",
            self.file_path.to_string_lossy(),
            self.metadata.len(),
            mtime_secs,
        );
        format!("{:016x}", xxhash_rust::xxh3::xxh3_64(cache_key.as_bytes()))
    }
}

#[derive(Debug)]
enum FilePipelineExecutionError {
    Forbidden,
    BadRequest,
    Timeout,
    Io(io::Error),
    Pipeline(PipelineError),
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

/// Canonicalize an IP address: convert IPv4-mapped IPv6 (`::ffff:x.x.x.x`) to IPv4.
#[inline]
fn canonicalize_ip(ip: std::net::IpAddr) -> String {
    match ip {
        std::net::IpAddr::V4(_) => ip.to_string(),
        std::net::IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                v4.to_string()
            } else {
                ip.to_string()
            }
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
        MetricAttributeValue::String(request.method().as_str().to_string()),
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
            parent_span_id: None,
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
    // (only needed for access logging later — skip if no access sinks configured)
    let request_headers: Vec<(String, String)> = if !events.is_empty() {
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
        // TODO: add Alt-Svc for HTTP/3
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
    let (routing_str, _original_str) = match canonicalize_path_routing_cached(request.uri().path()) {
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

    let mut variables = HashMap::with_capacity(6);
    if let Some(hostname) = hostname.as_ref() {
        variables.insert("request.host".to_string(), hostname.clone());
    }
    variables.insert(
        "request.scheme".to_string(),
        if encrypted { "https" } else { "http" }.to_string(),
    );
    variables.insert("server.ip".to_string(), local_address.ip().to_string());
    variables.insert("server.port".to_string(), local_address.port().to_string());
    variables.insert("remote.ip".to_string(), remote_address.ip().to_string());
    variables.insert("remote.port".to_string(), remote_address.port().to_string());

    let resolver_variables = (request, variables);
    let resolution = config_resolver.resolve(
        local_address.ip(),
        hostname.as_deref().unwrap_or(""),
        &routing_str,
        &resolver_variables,
    );
    let request = resolver_variables.0;
    let variables = resolver_variables.1;

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
                        .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
                }),
            )),
            None,
            None,
        );
    };

    // Handle OPTIONS * requests (RFC 2616 Section 9.2)
    // Early response before pipeline execution
    if is_options_star_request(&request) {
        let allow_header = resolution
            .configuration
            .get_value("options_allowed_methods", false)
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .unwrap_or_else(|| "GET, HEAD, POST, OPTIONS".to_string());

        let response = Response::builder()
            .status(200)
            .header("Allow", &allow_header)
            .header("Content-Length", "0")
            .body(Empty::<Bytes>::new().map_err(|e| match e {}).boxed_unsync())
            .expect("failed to build OPTIONS * response");

        return (Ok(response), None, None);
    }

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
        .and_then(|v| v.as_string_with_interpolations(&HashMap::new()));
    let resolution_configuration2 = resolution.configuration.clone();
    let mut ctx = HttpContext {
        req: Some(request),
        res: None,
        events: events.clone(),
        configuration: resolution.configuration,
        hostname,
        variables: variables.into_iter().collect(),
        previous_error: None,
        original_uri: Option::from(request_uri),
        routing_uri: routing_str.parse().ok(),
        encrypted,
        local_address,
        remote_address,
        auth_user: None,
        https_port,
        extensions: TypeMap::new(),
    };

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
    if let Some(HttpResponse::BuiltinError(status, _)) = ctx.res {
        if status >= 400 {
            ctx.previous_error = Some(status);
            ctx.req = Some(cloned_request);
            // Rebuild the resolver request from the current request in context
            if let Some(req) = ctx.req.take() {
                let error_resolver_variables = (
                    req,
                    ctx.variables.clone().into_iter().collect::<HashMap<_, _>>(),
                );
                let error_resolution = config_resolver.resolve_error_scoped(
                    local_address.ip(),
                    ctx.hostname.as_deref().unwrap_or(""),
                    &routing_str,
                    status,
                    &error_resolver_variables,
                );
                ctx.req = Some(error_resolver_variables.0);

                if let Some(error_resolution) = error_resolution {
                    let execute_error_config = if let (Some(config1), Some(config2)) = (
                        error_resolution.configuration.layers.last(),
                        resolution_configuration2.layers.last(),
                    ) {
                        Arc::ptr_eq(config1, config2)
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
            parent_span_id: None,
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
            }
        }
        // TODO: execute with timeout END

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

async fn execute_http_file_pipeline(
    ctx: &mut HttpContext,
    file_pipeline: &Pipeline<HttpFileContext>,
    timeout: Option<std::time::Duration>,
) -> Result<(), FilePipelineExecutionError> {
    let Some(request_path_encoded) = ctx
        .req
        .as_ref()
        .map(|request| request.uri().path().to_string())
    else {
        return Ok(());
    };
    let request_path = urlencoding::decode(&request_path_encoded)
        .map_err(|_| FilePipelineExecutionError::BadRequest)?
        .to_string();
    let Some(root_path) = resolve_webroot(ctx)? else {
        return Ok(());
    };

    // Get index file configuration for directory resolution
    let index_files = resolve_index_files(ctx);

    // Check cache first
    let cache_key = (root_path.clone(), request_path.clone());
    let resolved_file = match PATH_RESOLVE_CACHE.get(&cache_key) {
        Some(timestamped) if !timestamped.is_expired(path_resolve_cache_ttl()) => {
            // Re-validate metadata to detect file changes/deletions
            let cache_path = &timestamped.value.file_path;
            match vibeio::fs::metadata(cache_path).await {
                Ok(current_metadata)
                    if current_metadata.len() == timestamped.value.metadata.len()
                        && current_metadata.modified().ok()
                            == timestamped.value.metadata.modified().ok() =>
                {
                    // Metadata unchanged — safe to use cached value
                    timestamped.value.clone()
                }
                _ => {
                    // Metadata changed or file deleted — resolve fresh
                    let Some(resolved) =
                        resolve_and_cache(&root_path, &request_path, Some(&index_files)).await?
                    else {
                        return Ok(());
                    };
                    resolved
                }
            }
        }
        _ => {
            // Cache miss or expired — resolve from filesystem
            let Some(resolved) =
                resolve_and_cache(&root_path, &request_path, Some(&index_files)).await?
            else {
                return Ok(());
            };
            resolved
        }
    };

    // Handle trailing slash redirect for directories
    if resolved_file.metadata.is_dir() {
        let trailing_slash_redirect_enabled = ctx
            .configuration
            .get_value("trailing_slash_redirect", true)
            .map(|v| v.as_boolean())
            .unwrap_or(Some(true))
            .unwrap_or(true);

        if trailing_slash_redirect_enabled && !request_path.ends_with('/') {
            // Build redirect URL with trailing slash
            let redirect_path = format!("{request_path}/");
            let uri = match ctx.req.as_ref() {
                Some(req) => {
                    let mut uri_parts = req.uri().clone().into_parts();
                    if let Some(path_and_query) = &uri_parts.path_and_query {
                        let new_path_and_query = format!(
                            "{redirect_path}{}",
                            if let Some(q) = path_and_query.query() {
                                format!("?{q}")
                            } else {
                                String::new()
                            }
                        );
                        uri_parts.path_and_query = new_path_and_query.try_into().ok();
                    }
                    if uri_parts.path_and_query.is_some() {
                        http::Uri::from_parts(uri_parts).ok()
                    } else {
                        None
                    }
                }
                None => None,
            };

            if let Some(redirect_uri) = uri {
                ctx.res = Some(HttpResponse::Custom(
                    http::Response::builder()
                        .status(http::StatusCode::MOVED_PERMANENTLY)
                        .header(http::header::LOCATION, redirect_uri.to_string())
                        .body(
                            http_body_util::Full::new(bytes::Bytes::from(format!(
                                "Moved Permanently to {redirect_path}",
                            )))
                            .map_err(|_| unreachable!())
                            .boxed_unsync(),
                        )
                        .expect("failed to build redirect response"),
                ));
                return Ok(());
            }
        }
    }

    apply_resolved_file_to_context(ctx, resolved_file, file_pipeline, timeout, root_path).await
}

/// Resolve a file path and insert into cache with timestamp.
async fn resolve_and_cache(
    root_path: &Path,
    request_path: &str,
    index_files: Option<&[String]>,
) -> Result<Option<ResolvedHttpFile>, FilePipelineExecutionError> {
    let Some(resolved_file) =
        resolve_http_file_target(root_path, request_path, index_files).await?
    else {
        return Ok(None);
    };

    // Pre-compute ETag once at cache insertion time
    let mut resolved_file = resolved_file;
    resolved_file.etag = resolved_file.compute_etag();

    let cache_key = (root_path.to_path_buf(), request_path.to_string());
    PATH_RESOLVE_CACHE.insert(cache_key, Timestamped::new(resolved_file.clone()));
    Ok(Some(resolved_file))
}

/// Apply a resolved file to the HTTP context and execute the file pipeline.
async fn apply_resolved_file_to_context(
    ctx: &mut HttpContext,
    resolved_file: ResolvedHttpFile,
    file_pipeline: &Pipeline<HttpFileContext>,
    timeout: Option<std::time::Duration>,
    root_path: PathBuf,
) -> Result<(), FilePipelineExecutionError> {
    if let Some(path_info) = resolved_file.path_info.as_ref() {
        ctx.variables
            .insert("request.path_info".to_string(), path_info.clone());
    } else {
        ctx.variables.remove("request.path_info");
    }

    let placeholder = HttpContext {
        req: None,
        res: None,
        events: ctx.events.clone(),
        configuration: ctx.configuration.clone(),
        hostname: ctx.hostname.clone(),
        variables: FxHashMap::default(),
        previous_error: None,
        original_uri: None,
        routing_uri: None,
        encrypted: ctx.encrypted,
        local_address: ctx.local_address,
        remote_address: ctx.remote_address,
        auth_user: None,
        https_port: ctx.https_port,
        extensions: TypeMap::new(),
    };
    let http_ctx = std::mem::replace(ctx, placeholder);
    let mut file_ctx = HttpFileContext {
        http: http_ctx,
        metadata: resolved_file.metadata,
        file_path: resolved_file.file_path,
        path_info: resolved_file.path_info,
        file_root: root_path,
        etag: resolved_file.etag,
    };

    let pipeline_result = if let Some(timeout) = timeout {
        vibeio::time::timeout(timeout, file_pipeline.execute(&mut file_ctx)).await
    } else {
        Ok(file_pipeline.execute(&mut file_ctx).await)
    };

    *ctx = file_ctx.http;

    match pipeline_result {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(FilePipelineExecutionError::Pipeline(e)),
        Err(_) => Err(FilePipelineExecutionError::Timeout),
    }
}

fn resolve_webroot(ctx: &HttpContext) -> Result<Option<PathBuf>, FilePipelineExecutionError> {
    let root_entries = ctx.configuration.get_entries("root", true);
    let Some(root_entry) = root_entries.first() else {
        return Ok(None);
    };
    let Some(root_path) = root_entry
        .get_value()
        .and_then(|value| value.as_string_with_interpolations(ctx))
    else {
        return Err(FilePipelineExecutionError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "HTTP root must be a string",
        )));
    };

    Ok(Some(PathBuf::from(root_path)))
}

/// Get the list of configured index files for a directory.
/// Returns the default list if not explicitly configured.
fn resolve_index_files(ctx: &HttpContext) -> Vec<String> {
    let entries = ctx.configuration.get_entries("index", true);
    if entries.is_empty() {
        vec![
            "index.html".into(),
            "index.htm".into(),
            "index.xhtml".into(),
        ]
    } else {
        entries
            .iter()
            .flat_map(|entry| {
                entry
                    .args
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
            })
            .collect()
    }
}

/// Resolve an HTTP file target from a request path.
/// If `index_files` is provided and the resolved path is a directory,
/// tries each index file in order until one is found.
async fn resolve_http_file_target(
    root_path: &Path,
    request_path: &str,
    index_files: Option<&[String]>,
) -> Result<Option<ResolvedHttpFile>, FilePipelineExecutionError> {
    if !request_path.starts_with('/') {
        return Ok(None);
    }

    let canonical_root = vibeio::fs::canonicalize(root_path)
        .await
        .map_err(FilePipelineExecutionError::Io)?;

    let request_segments = request_path_segments(request_path)?;
    let mut candidate_depth = request_segments.len();
    let trailing_slash = request_path.ends_with('/') && request_path != "/";

    loop {
        let candidate_path =
            build_candidate_path(&canonical_root, &request_segments[..candidate_depth]);
        match vibeio::fs::metadata(&candidate_path).await {
            Ok(metadata) => {
                let candidate_path = vibeio::fs::canonicalize(&candidate_path)
                    .await
                    .map_err(FilePipelineExecutionError::Io)?;
                if !candidate_path.starts_with(&canonical_root) {
                    return Err(FilePipelineExecutionError::Forbidden);
                }

                // If it's a directory and index_files are provided, try to find an index file
                if metadata.is_dir() {
                    if let Some(index_files) = index_files {
                        if let Some(index_file) =
                            try_resolve_index_files(&candidate_path, index_files).await?
                        {
                            return Ok(Some(ResolvedHttpFile {
                                metadata: index_file.metadata,
                                file_path: index_file.file_path,
                                path_info: build_path_info(
                                    &request_segments[candidate_depth..],
                                    trailing_slash,
                                ),
                                etag: String::new(), // Will be set after construction
                            }));
                        }
                    }
                }

                let resolved = ResolvedHttpFile {
                    metadata,
                    file_path: candidate_path,
                    path_info: build_path_info(
                        &request_segments[candidate_depth..],
                        trailing_slash,
                    ),
                    etag: String::new(), // Will be set after construction
                };
                return Ok(Some(resolved));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) if is_not_directory_like(&error) && candidate_depth > 0 => {
                candidate_depth -= 1;
            }
            Err(error) => return Err(FilePipelineExecutionError::Io(error)),
        }
    }
}

/// Try to resolve an index file in a directory.
/// Returns the first index file that exists and is a regular file.
async fn try_resolve_index_files(
    directory: &Path,
    index_files: &[String],
) -> Result<Option<ResolvedHttpFile>, FilePipelineExecutionError> {
    for index in index_files {
        let index_path = directory.join(index);
        match vibeio::fs::metadata(&index_path).await {
            Ok(metadata) if metadata.is_file() => {
                // Verify the index file is within the webroot
                let canonical = vibeio::fs::canonicalize(&index_path)
                    .await
                    .map_err(FilePipelineExecutionError::Io)?;

                return Ok(Some(ResolvedHttpFile {
                    metadata,
                    file_path: canonical,
                    path_info: None,
                    etag: String::new(), // Will be set after construction
                }));
            }
            Ok(_) => continue, // Directory or other type
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) if error.kind() == io::ErrorKind::NotADirectory => continue,
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                return Err(FilePipelineExecutionError::Forbidden);
            }
            Err(error) => {
                return Err(FilePipelineExecutionError::Io(error));
            }
        }
    }

    Ok(None)
}

fn request_path_segments(request_path: &str) -> Result<Vec<String>, FilePipelineExecutionError> {
    let mut segments = Vec::new();

    for component in Path::new(request_path).components() {
        match component {
            Component::RootDir => {}
            Component::Normal(segment) => segments.push(segment.to_string_lossy().into_owned()),
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => {
                return Err(FilePipelineExecutionError::Forbidden);
            }
        }
    }

    Ok(segments)
}

fn build_candidate_path(root_path: &Path, request_segments: &[String]) -> PathBuf {
    let mut candidate_path = root_path.to_path_buf();
    for segment in request_segments {
        candidate_path.push(segment);
    }
    candidate_path
}

fn build_path_info(request_segments: &[String], trailing_slash: bool) -> Option<String> {
    if request_segments.is_empty() {
        return None;
    }

    let mut path_info = String::new();
    for segment in request_segments {
        path_info.push('/');
        path_info.push_str(segment);
    }
    if trailing_slash {
        path_info.push('/');
    }

    Some(path_info)
}

fn strip_matched_path_prefix(
    path_and_query: &http::uri::PathAndQuery,
    matched_segments: usize,
) -> Option<http::uri::PathAndQuery> {
    if matched_segments == 0 {
        return Some(path_and_query.clone());
    }

    let path = path_and_query.path();
    let path_bytes = path.as_bytes();
    let mut offset = 0;

    for _ in 0..matched_segments {
        if offset >= path_bytes.len() || path_bytes[offset] != b'/' {
            return None;
        }
        offset += 1;
        while offset < path_bytes.len() && path_bytes[offset] != b'/' {
            offset += 1;
        }
    }

    let stripped_path = if offset >= path.len() {
        "/"
    } else {
        &path[offset..]
    };
    let stripped_path_and_query = if let Some(query) = path_and_query.query() {
        format!("{stripped_path}?{query}")
    } else {
        stripped_path.to_string()
    };

    stripped_path_and_query.try_into().ok()
}

fn is_not_directory_like(error: &io::Error) -> bool {
    #[cfg(unix)]
    if error.raw_os_error() == Some(20) {
        return true;
    }

    #[cfg(windows)]
    if error.raw_os_error() == Some(267) {
        return true;
    }

    false
}

/// Normalize HTTP/2 and HTTP/3 requests
///
/// For HTTP/2 and HTTP/3, the Host header is not transmitted; instead, it's encoded
/// in the :authority pseudo-header. This function sets the Host header from the authority
/// and normalizes the Cookie header (combining multiple values).
fn normalize_http2_http3_request(request: &mut HttpRequest) {
    // Set "Host" request header from authority for HTTP/2 and HTTP/3 connections
    if let Some(authority) = request.uri().authority() {
        let authority = authority.to_owned();
        let headers = request.headers_mut();
        if !headers.contains_key(http::header::HOST) {
            if let Ok(authority_value) = HeaderValue::from_bytes(authority.as_str().as_bytes()) {
                headers.append(http::header::HOST, authority_value);
            }
        }
    }

    // Normalize the Cookie header for HTTP/2 and HTTP/3
    // Combine multiple cookie headers into a single one with "; " separator
    let mut cookie_normalized = String::new();
    let mut cookie_set = false;
    let headers = request.headers_mut();
    for cookie in headers.get_all(http::header::COOKIE) {
        if let Ok(cookie) = cookie.to_str() {
            if cookie_set {
                cookie_normalized.push_str("; ");
            }
            cookie_set = true;
            cookie_normalized.push_str(cookie);
        }
    }
    if cookie_set {
        if let Ok(cookie_value) = HeaderValue::from_bytes(cookie_normalized.as_bytes()) {
            headers.insert(http::header::COOKIE, cookie_value);
        }
    }
}

/// Normalize the "Host" header
///
/// - Converts the host to lowercase
/// - Removes trailing dot (FQDN notation)
/// - Validates the resulting header value
fn normalize_host_header(
    request: &mut HttpRequest,
    _events: &CompositeEventSink,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut host_header_option = request.headers().get_all(http::header::HOST).iter();
    if let Some(header_data) = host_header_option.next() {
        if host_header_option.next().is_some() {
            Err(anyhow::anyhow!("Multiple Host headers found"))?;
        }
        let host_header = header_data.to_str()?;
        let host_header_lower_case = host_header.to_lowercase();
        let host_header_without_dot = host_header_lower_case
            .strip_suffix('.')
            .unwrap_or(host_header_lower_case.as_str());

        if host_header_without_dot != host_header {
            let host_header_value = HeaderValue::from_str(host_header_without_dot)?;
            request
                .headers_mut()
                .insert(http::header::HOST, host_header_value);
        }
    }
    Ok(())
}

#[cfg(any(test, feature = "bench"))]
pub async fn bench_resolve_http_file_target(
    root_path: &std::path::Path,
    request_path: &str,
    index_files: Option<&[String]>,
) -> Result<bool, String> {
    match resolve_http_file_target(root_path, request_path, index_files).await {
        Ok(opt) => Ok(opt.is_some()),
        Err(e) => Err(format!("{:?}", e)),
    }
}

/// Get a nested boolean value from an HTTP configuration block.
///
/// Looks up `http.<directive>` within the given configuration block
/// and returns it as a boolean if present.
fn get_http_nested_boolean(
    block: &ferron_core::config::ServerConfigurationBlock,
    directive: &str,
) -> Option<bool> {
    block
        .directives
        .get("http")
        .and_then(|entries| entries.first())
        .and_then(|http_entry| http_entry.children.as_ref())
        .and_then(|http_block| {
            http_block
                .directives
                .get(directive)
                .and_then(|entries| entries.first())
                .and_then(|entry| entry.args.first())
                .and_then(|value| value.as_boolean())
        })
}

/// Sanitize the request URL path
///
/// Removes dangerous sequences like path traversal attempts (../, .\\, etc.)
/// and normalizes slashes and percent-encoding.
fn sanitize_request_url(
    request: &mut HttpRequest,
    decoded_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let url_pathname = request.uri().path();

    if decoded_path != url_pathname {
        // We need to reconstruct the URI with the sanitized path
        let orig_uri = request.uri().clone();
        let mut uri_parts = orig_uri.into_parts();

        // Reconstruct the path_and_query with sanitized path and original query
        let new_path_and_query = format!(
            "{}{}",
            decoded_path,
            uri_parts
                .path_and_query
                .as_ref()
                .and_then(|pq| pq.query())
                .map_or("".to_string(), |q| format!("?{q}"))
        );

        uri_parts.path_and_query = Some(new_path_and_query.parse()?);
        let new_uri = http::Uri::from_parts(uri_parts)?;

        // Use the http::Request extension to set the URI
        *request.uri_mut() = new_uri;
    }

    Ok(())
}

/// Check if this is an OPTIONS * request (server-wide OPTIONS per RFC 2616 Section 9.2)
#[inline]
fn is_options_star_request(request: &HttpRequest) -> bool {
    request.method() == http::Method::OPTIONS && request.uri().path() == "*"
}

#[inline]
fn builtin_error_response(
    status: u16,
    headers: Option<&HeaderMap>,
    admin_email: Option<String>,
) -> Response<ResponseBody> {
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let body = generate_default_error_page(status, admin_email.as_deref());
    let mut builder = Response::builder().status(status);
    if let Some(headers) = headers {
        for (name, value) in headers {
            builder = builder.header(name, value);
        }
    }

    builder
        .header(
            http::header::CONTENT_TYPE,
            HeaderValue::from_static("text/html"),
        )
        .body(
            Full::new(Bytes::copy_from_slice(body.as_bytes()))
                .map_err(|e| match e {})
                .boxed_unsync(),
        )
        .unwrap_or_else(|_| builtin_error_response(500, None, admin_email))
}

#[inline]
fn emit_error(events: &CompositeEventSink, message: impl Into<String>) {
    events.emit(Event::Log(LogEvent {
        level: LogLevel::Error,
        message: message.into(),
        target: LOG_TARGET,
    }));
}

async fn execute_error_pipeline(
    error_pipeline: &Pipeline<HttpErrorContext>,
    error_code: u16,
    headers: Option<HeaderMap>,
    configuration: LayeredConfiguration,
    events: &CompositeEventSink,
) -> Option<Response<ResponseBody>> {
    let has_traces = events.has_trace_sinks();

    // Start error pipeline execution span
    if has_traces {
        events.emit(Event::Trace(TraceEvent::StartSpan {
            name: Cow::Borrowed("ferron.pipeline.execute_error"),
            parent_span_id: None,
            attributes: vec![(
                "http.response.status_code",
                TraceAttributeValue::I64(error_code as i64),
            )],
        }));
    }

    let mut error_ctx = HttpErrorContext {
        error_code,
        headers,
        configuration,
        res: None,
    };

    if let Err(error) = error_pipeline.execute_without_inverse(&mut error_ctx).await {
        emit_error(events, format!("Error pipeline execution error: {error}"));
    }

    // End error pipeline execution span
    if has_traces {
        events.emit(Event::Trace(TraceEvent::EndSpan {
            name: Cow::Borrowed("ferron.pipeline.execute_error"),
            error: None,
            attributes: vec![],
        }));
    }

    error_ctx.res
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(name: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time before UNIX epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "ferron-http-server-{name}-{}-{unique}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).expect("failed to create test directory");
            Self { path }
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn resolves_path_info_when_request_descends_below_a_file() {
        let root = TestDir::new("path-info");
        std::fs::write(root.path.join("index.html"), b"hello").expect("failed to write file");

        let resolved = resolve_http_file_target(&root.path, "/index.html/test", None)
            .await
            .expect("resolution should succeed")
            .expect("file should resolve");

        assert!(resolved.metadata.is_file());
        assert_eq!(
            resolved.file_path,
            root.path
                .join("index.html")
                .canonicalize()
                .expect("failed to canonicalize file"),
        );
        assert_eq!(resolved.path_info.as_deref(), Some("/test"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn returns_none_for_missing_files() {
        let root = TestDir::new("missing-file");

        let resolved = resolve_http_file_target(&root.path, "/missing.txt", None)
            .await
            .expect("resolution should succeed");

        assert!(resolved.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn rejects_parent_directory_traversal() {
        let root = TestDir::new("parent-traversal");

        let error = resolve_http_file_target(&root.path, "/../secret.txt", None)
            .await
            .expect_err("traversal should be rejected");

        assert!(matches!(error, FilePipelineExecutionError::Forbidden));
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn rejects_symlink_targets_outside_the_webroot() {
        let base = TestDir::new("symlink-traversal");
        let root = base.path.join("root");
        std::fs::create_dir_all(&root).expect("failed to create webroot");

        let outside = base.path.join("outside.txt");
        std::fs::write(&outside, b"secret").expect("failed to write outside file");
        std::os::unix::fs::symlink(&outside, root.join("escape.txt"))
            .expect("failed to create symlink");

        let error = resolve_http_file_target(&root, "/escape.txt", None)
            .await
            .expect_err("symlink escape should be rejected");

        assert!(matches!(error, FilePipelineExecutionError::Forbidden));
    }

    #[test]
    fn is_options_star_request_detects_asterisk_options() {
        let request = http::Request::builder()
            .method(http::Method::OPTIONS)
            .uri("*")
            .body(Empty::<Bytes>::new().map_err(|e| match e {}).boxed_unsync())
            .expect("valid request");

        assert!(is_options_star_request(&request));
    }

    #[test]
    fn is_options_star_request_rejects_other_methods() {
        let request = http::Request::builder()
            .method(http::Method::GET)
            .uri("*")
            .body(Empty::<Bytes>::new().map_err(|e| match e {}).boxed_unsync())
            .expect("valid request");

        assert!(!is_options_star_request(&request));
    }

    #[test]
    fn is_options_star_request_rejects_other_paths() {
        let request = http::Request::builder()
            .method(http::Method::OPTIONS)
            .uri("/path")
            .body(Empty::<Bytes>::new().map_err(|e| match e {}).boxed_unsync())
            .expect("valid request");

        assert!(!is_options_star_request(&request));
    }

    #[test]
    fn is_options_star_request_rejects_empty_path() {
        let request = http::Request::builder()
            .method(http::Method::OPTIONS)
            .uri("/")
            .body(Empty::<Bytes>::new().map_err(|e| match e {}).boxed_unsync())
            .expect("valid request");

        assert!(!is_options_star_request(&request));
    }

    #[test]
    fn strip_matched_path_prefix_preserves_root_when_location_matches_entire_path() {
        let path_and_query = "/api/users?expand=true"
            .parse()
            .expect("valid path and query");

        let stripped = strip_matched_path_prefix(&path_and_query, 2)
            .expect("matched prefix should strip cleanly");

        assert_eq!(stripped.as_str(), "/?expand=true");
    }

    #[test]
    fn strip_matched_path_prefix_preserves_remaining_suffix() {
        let path_and_query = "/api/users/profile/avatar"
            .parse()
            .expect("valid path and query");

        let stripped = strip_matched_path_prefix(&path_and_query, 2)
            .expect("matched prefix should strip cleanly");

        assert_eq!(stripped.as_str(), "/profile/avatar");
    }
}
