use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use ferron_core::config::layer::LayeredConfiguration;
use ferron_core::pipeline::{Pipeline, PipelineError};
use ferron_core::util::parse_duration;
use ferron_http::{HttpContext, HttpErrorContext, HttpFileContext, HttpRequest, HttpResponse};
use ferron_observability::{CompositeEventSink, Event, LogEvent, LogLevel};
use http::{HeaderMap, HeaderValue, Response, StatusCode};
use http_body_util::Empty;
use http_body_util::{combinators::UnsyncBoxBody, BodyExt, Full};
use typemap_rev::TypeMap;

use crate::config::ThreeStageResolver;
use crate::util::error_pages::generate_default_error_page;
use crate::util::url_sanitizer::sanitize_url;

const LOG_TARGET: &str = "ferron-http-server";
type ResponseBody = UnsyncBoxBody<Bytes, io::Error>;

#[derive(Debug)]
struct ResolvedHttpFile {
    metadata: vibeio::fs::Metadata,
    file_path: PathBuf,
    path_info: Option<String>,
}

#[derive(Debug)]
enum FilePipelineExecutionError {
    Forbidden,
    BadRequest,
    Timeout,
    Io(io::Error),
    Pipeline(PipelineError),
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
    events: CompositeEventSink,
) -> Result<Response<ResponseBody>, io::Error> {
    // TODO: add support for HTTP access logs

    let mut response_result = request_handler_inner(
        request,
        pipeline,
        file_pipeline,
        error_pipeline,
        config_resolver,
        local_address,
        remote_address,
        hostname,
        encrypted,
        events,
    )
    .await;
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
    events: CompositeEventSink,
) -> Result<Response<ResponseBody>, io::Error> {
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
            return Ok(response);
        }
        return Ok(builtin_error_response(
            400,
            None,
            config_resolver.global().and_then(|g| {
                g.get_value("admin_email")
                    .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            }),
        ));
    }

    // Sanitize URL
    if let Err(e) = sanitize_request_url(&mut request, &events) {
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
            return Ok(response);
        }
        return Ok(builtin_error_response(
            400,
            None,
            config_resolver.global().and_then(|g| {
                g.get_value("admin_email")
                    .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            }),
        ));
    }

    let mut variables = HashMap::new();
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

    let resolver_request = build_resolver_request(&request)?;
    let resolution = config_resolver.resolve(
        local_address.ip(),
        hostname.as_deref().unwrap_or(""),
        request.uri().path(),
        &(resolver_request, variables.clone()),
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
            return Ok(response);
        }
        return Ok(builtin_error_response(
            404,
            None,
            config_resolver.global().and_then(|g| {
                g.get_value("admin_email")
                    .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            }),
        ));
    };

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
    let mut ctx = HttpContext {
        req: Some(request),
        res: None,
        events: events.clone(),
        configuration: resolution.configuration,
        hostname,
        variables,
        previous_error: None,
        original_uri: Option::from(request_uri),
        encrypted,
        local_address,
        remote_address,
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
            if let Some(ref req) = ctx.req {
                let error_resolver_request = build_resolver_request(req)?;
                let error_resolution = config_resolver.resolve_error_scoped(
                    local_address.ip(),
                    ctx.hostname.as_deref().unwrap_or(""),
                    status,
                    &(error_resolver_request, ctx.variables.clone()),
                );
                if let Some(error_resolution) = error_resolution {
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

    Ok(
        match ctx.res.unwrap_or(HttpResponse::BuiltinError(404, None)) {
            HttpResponse::Custom(response) => response,
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
                    return Ok(response);
                }
                builtin_error_response(status, headers.as_ref(), admin_email)
            }
            HttpResponse::Abort => return Err(io::Error::other("Aborted")),
        },
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
    // Remove the base URL if path segments were matched
    if !path_segments.is_empty() {
        if let Some(req) = ctx.req.take() {
            let (mut parts, body) = req.into_parts();
            let mut uri_parts = parts.uri.into_parts();
            if let Some(path_and_query) = uri_parts.path_and_query {
                let mut path_split = path_and_query.path().split('/').collect::<Vec<_>>();
                let mut new_path_split = Vec::with_capacity(path_split.len() - path_segments.len());
                new_path_split.push("");
                new_path_split.extend(path_split.split_off(path_segments.len() + 1));
                let new_path = new_path_split.join("/");
                uri_parts.path_and_query = format!(
                    "{new_path}{}",
                    if let Some(q) = path_and_query.query() {
                        format!("?{q}")
                    } else {
                        "".to_string()
                    }
                )
                .try_into()
                .ok();
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

    let executed_stages = match if let Some(timeout_duration) =
        timeout_duration.map(|d| d.saturating_sub(instant.elapsed()))
    {
        vibeio::time::timeout(timeout_duration, pipeline.execute_without_inverse(ctx)).await
    } else {
        Ok(pipeline.execute_without_inverse(ctx).await)
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

        if let Err(error) = pipeline.execute_inverse(ctx, executed_stages).await {
            emit_error(
                events,
                format!("{log_prefix}Pipeline inverse execution error: {error}"),
            );
            ctx.res = Some(HttpResponse::BuiltinError(500, None));
        }
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
    let Some(resolved_file) = resolve_http_file_target(&root_path, &request_path).await? else {
        return Ok(());
    };

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
        variables: HashMap::new(),
        previous_error: None,
        original_uri: None,
        encrypted: ctx.encrypted,
        local_address: ctx.local_address,
        remote_address: ctx.remote_address,
        extensions: TypeMap::new(),
    };
    let http_ctx = std::mem::replace(ctx, placeholder);
    let mut file_ctx = HttpFileContext {
        http: http_ctx,
        metadata: resolved_file.metadata,
        file_path: resolved_file.file_path,
        path_info: resolved_file.path_info,
        file_root: root_path,
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

async fn resolve_http_file_target(
    root_path: &Path,
    request_path: &str,
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

                return Ok(Some(ResolvedHttpFile {
                    metadata,
                    file_path: candidate_path,
                    path_info: build_path_info(
                        &request_segments[candidate_depth..],
                        trailing_slash,
                    ),
                }));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) if is_not_directory_like(&error) && candidate_depth > 0 => {
                candidate_depth -= 1;
            }
            Err(error) => return Err(FilePipelineExecutionError::Io(error)),
        }
    }
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
    let host_header_option = request.headers().get(http::header::HOST);
    if let Some(header_data) = host_header_option {
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

/// Sanitize the request URL path
///
/// Removes dangerous sequences like path traversal attempts (../, .\\, etc.)
/// and normalizes slashes and percent-encoding.
fn sanitize_request_url(
    request: &mut HttpRequest,
    _events: &CompositeEventSink,
) -> Result<(), Box<dyn std::error::Error>> {
    let url_pathname = request.uri().path();
    let sanitized_url_pathname = sanitize_url(url_pathname, false)?;

    if sanitized_url_pathname != url_pathname {
        // We need to reconstruct the URI with the sanitized path
        let orig_uri = request.uri().clone();
        let mut uri_parts = orig_uri.into_parts();

        // Reconstruct the path_and_query with sanitized path and original query
        let new_path_and_query = format!(
            "{}{}",
            sanitized_url_pathname,
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

fn build_resolver_request(request: &HttpRequest) -> Result<HttpRequest, io::Error> {
    let mut builder = http::Request::builder()
        .method(request.method().clone())
        .uri(request.uri().clone())
        .version(request.version());
    for (name, value) in request.headers() {
        builder = builder.header(name, value);
    }

    builder
        .body(
            http_body_util::Empty::<Bytes>::new()
                .map_err(|e| match e {})
                .boxed_unsync(),
        )
        .map_err(|error| io::Error::other(error.to_string()))
}

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
    let mut error_ctx = HttpErrorContext {
        error_code,
        headers,
        configuration,
        res: None,
    };

    if let Err(error) = error_pipeline.execute_without_inverse(&mut error_ctx).await {
        emit_error(events, format!("Error pipeline execution error: {error}"));
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

        let resolved = resolve_http_file_target(&root.path, "/index.html/test")
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

        let resolved = resolve_http_file_target(&root.path, "/missing.txt")
            .await
            .expect("resolution should succeed");

        assert!(resolved.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn rejects_parent_directory_traversal() {
        let root = TestDir::new("parent-traversal");

        let error = resolve_http_file_target(&root.path, "/../secret.txt")
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

        let error = resolve_http_file_target(&root, "/escape.txt")
            .await
            .expect_err("symlink escape should be rejected");

        assert!(matches!(error, FilePipelineExecutionError::Forbidden));
    }
}
