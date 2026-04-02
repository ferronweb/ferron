use std::collections::HashMap;
use std::io;
use std::net::IpAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use bytes::Bytes;
use ferron_core::pipeline::{Pipeline, PipelineError};
use ferron_http::{HttpContext, HttpFileContext, HttpRequest, HttpResponse};
use ferron_observability::{CompositeEventSink, Event, LogEvent, LogLevel};
use http::{HeaderMap, Response, StatusCode};
use http_body_util::{combinators::UnsyncBoxBody, BodyExt, Full};

use crate::config::ThreeStageResolver;

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
    Io(io::Error),
    Pipeline(PipelineError),
}

pub async fn request_handler(
    request: HttpRequest,
    pipeline: Arc<Pipeline<HttpContext>>,
    file_pipeline: Arc<Pipeline<HttpFileContext>>,
    config_resolver: Arc<ThreeStageResolver>,
    local_ip: IpAddr,
    hostname: Option<String>,
    is_tls: bool,
    events: CompositeEventSink,
) -> Result<Response<ResponseBody>, io::Error> {
    // TODO: remodel the pipeline handler
    let mut variables = HashMap::new();
    if let Some(hostname) = hostname.as_ref() {
        variables.insert("request.host".to_string(), hostname.clone());
    }
    variables.insert(
        "request.scheme".to_string(),
        if is_tls { "https" } else { "http" }.to_string(),
    );
    variables.insert("server.ip".to_string(), local_ip.to_string());

    let resolver_request = build_resolver_request(&request)?;
    let resolution = config_resolver.resolve(
        local_ip,
        hostname.as_deref().unwrap_or(""),
        request.uri().path(),
        &(resolver_request, variables.clone()),
    );

    let Some(resolution) = resolution else {
        return Ok(text_response(StatusCode::NOT_FOUND, b"Not Found"));
    };

    let mut ctx = HttpContext {
        req: Some(request),
        res: None,
        events: events.clone(),
        configuration: resolution.configuration,
        hostname,
        variables,
    };

    let executed_stages = match pipeline.execute_without_inverse(&mut ctx).await {
        Ok(executed_stages) => Some(executed_stages),
        Err(error) => {
            emit_error(&events, format!("Pipeline execution error: {error}"));
            ctx.res = Some(HttpResponse::BuiltinError(500, None));
            None
        }
    };

    if let Some(executed_stages) = executed_stages {
        if ctx.res.is_none() {
            match execute_http_file_pipeline(&mut ctx, file_pipeline.as_ref()).await {
                Ok(()) => {}
                Err(FilePipelineExecutionError::Forbidden) => {
                    ctx.res = Some(HttpResponse::BuiltinError(403, None));
                }
                Err(FilePipelineExecutionError::Io(error)) => {
                    emit_error(&events, format!("HTTP file resolution error: {error}"));
                    ctx.res = Some(HttpResponse::BuiltinError(500, None));
                }
                Err(FilePipelineExecutionError::Pipeline(error)) => {
                    emit_error(&events, format!("Pipeline execution error: {error}"));
                    ctx.res = Some(HttpResponse::BuiltinError(500, None));
                }
            }
        }

        if let Err(error) = pipeline.execute_inverse(&mut ctx, executed_stages).await {
            emit_error(&events, format!("Pipeline execution error: {error}"));
            ctx.res = Some(HttpResponse::BuiltinError(500, None));
        }
    }

    Ok(
        match ctx.res.unwrap_or(HttpResponse::BuiltinError(404, None)) {
            HttpResponse::Custom(response) => response,
            HttpResponse::BuiltinError(status, headers) => {
                builtin_error_response(status, headers.as_ref())
            }
            HttpResponse::Abort => return Err(io::Error::other("Aborted")),
        },
    )
}

async fn execute_http_file_pipeline(
    ctx: &mut HttpContext,
    file_pipeline: &Pipeline<HttpFileContext>,
) -> Result<(), FilePipelineExecutionError> {
    let Some(request_path) = ctx
        .req
        .as_ref()
        .map(|request| request.uri().path().to_string())
    else {
        return Ok(());
    };
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
    };
    let http_ctx = std::mem::replace(ctx, placeholder);
    let mut file_ctx = HttpFileContext {
        http: http_ctx,
        metadata: resolved_file.metadata,
        file_path: resolved_file.file_path,
        path_info: resolved_file.path_info,
        file_root: root_path,
    };

    let pipeline_result = file_pipeline.execute(&mut file_ctx).await;
    *ctx = file_ctx.http;

    pipeline_result.map_err(FilePipelineExecutionError::Pipeline)
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

// TODO: improved built-in error responses
fn builtin_error_response(status: u16, headers: Option<&HeaderMap>) -> Response<ResponseBody> {
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let body = status.canonical_reason().unwrap_or("Error");
    let mut builder = Response::builder().status(status);
    if let Some(headers) = headers {
        for (name, value) in headers {
            builder = builder.header(name, value);
        }
    }

    builder
        .body(
            Full::new(Bytes::copy_from_slice(body.as_bytes()))
                .map_err(|e| match e {})
                .boxed_unsync(),
        )
        .unwrap_or_else(|_| {
            text_response(StatusCode::INTERNAL_SERVER_ERROR, b"Internal Server Error")
        })
}

fn text_response(status: StatusCode, body: &'static [u8]) -> Response<ResponseBody> {
    Response::builder()
        .status(status)
        .body(
            Full::new(Bytes::from_static(body))
                .map_err(|e| match e {})
                .boxed_unsync(),
        )
        .expect("failed to build text response")
}

fn emit_error(events: &CompositeEventSink, message: impl Into<String>) {
    events.emit(Event::Log(LogEvent {
        level: LogLevel::Error,
        message: message.into(),
        target: LOG_TARGET,
    }));
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
