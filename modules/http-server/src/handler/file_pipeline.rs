use std::io;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use ferron_core::pipeline::{Pipeline, PipelineError};
use ferron_http::{HttpContext, HttpFileContext, HttpResponse};
use ferron_observability::{Event, MetricAttributeValue, MetricEvent, MetricType, MetricValue};
use http_body_util::BodyExt;
use rustc_hash::FxHashMap;
use typemap_rev::TypeMap;

use super::PerStageSpanHooks;

/// Cache for path canonicalization results.
/// Keys: (canonical_root, request_path), Value: Timestamped<ResolvedHttpFile>
/// TTL default: 100 milliseconds to balance performance with filesystem change detection.
static PATH_RESOLVE_CACHE_TTL_MILLIS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(100);

fn path_resolve_cache_ttl() -> Duration {
    Duration::from_millis(PATH_RESOLVE_CACHE_TTL_MILLIS.load(std::sync::atomic::Ordering::Relaxed))
}

pub(crate) fn set_path_resolve_cache_ttl_millis(ms: u64) {
    PATH_RESOLVE_CACHE_TTL_MILLIS.store(ms, std::sync::atomic::Ordering::Relaxed);
}

static PATH_RESOLVE_CACHE: std::sync::LazyLock<
    quick_cache::sync::Cache<
        (PathBuf, String),
        Timestamped<ResolvedHttpFile>,
        PathResolveCacheWeighter,
    >,
> = std::sync::LazyLock::new(|| {
    quick_cache::sync::Cache::with_weighter(1024, 64 * 1024 * 1024, PathResolveCacheWeighter)
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
    etag: String,
}

impl ResolvedHttpFile {
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
pub(super) enum FilePipelineExecutionError {
    Forbidden,
    BadRequest,
    Timeout,
    Io(io::Error),
    Pipeline(PipelineError),
    WebrootNotFound,
}

pub(super) async fn execute_http_file_pipeline(
    ctx: &mut HttpContext,
    file_pipeline: &Pipeline<HttpFileContext>,
    timeout: Option<std::time::Duration>,
    parent_span_key: Option<&str>,
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

    let index_files = resolve_index_files(ctx);
    let cache_key = (root_path.clone(), request_path.clone());
    let resolved_file = match PATH_RESOLVE_CACHE.get(&cache_key) {
        Some(timestamped) if !timestamped.is_expired(path_resolve_cache_ttl()) => {
            let cache_path = &timestamped.value.file_path;
            match vibeio::fs::symlink_metadata(cache_path).await {
                Ok(current_metadata) if current_metadata.is_symlink() => {
                    return Err(FilePipelineExecutionError::Forbidden);
                }
                Ok(current_metadata)
                    if current_metadata.len() == timestamped.value.metadata.len()
                        && current_metadata.modified().ok()
                            == timestamped.value.metadata.modified().ok() =>
                {
                    timestamped.value.clone()
                }
                _ => {
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
            let Some(resolved) =
                resolve_and_cache(&root_path, &request_path, Some(&index_files)).await?
            else {
                return Ok(());
            };
            resolved
        }
    };

    if resolved_file.metadata.is_dir() {
        let trailing_slash_redirect_enabled = ctx
            .configuration
            .get_value("trailing_slash_redirect", true)
            .map(|v| v.as_boolean())
            .unwrap_or(Some(true))
            .unwrap_or(true);

        if trailing_slash_redirect_enabled && !request_path.ends_with('/') {
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
                            http_body_util::Empty::<bytes::Bytes>::new()
                                .map_err(|_| unreachable!())
                                .boxed_unsync(),
                        )
                        .expect("failed to build redirect response"),
                ));
                ctx.events.emit(Event::Metric(MetricEvent {
                    name: "ferron.http.server.redirects",
                    attributes: vec![
                        ("http.response.status_code", MetricAttributeValue::I64(301)),
                        (
                            "ferron.http.redirect.reason",
                            MetricAttributeValue::StaticStr("trailing_slash"),
                        ),
                    ],
                    ty: MetricType::Counter,
                    value: MetricValue::U64(1),
                    unit: Some("{redirect}"),
                    description: Some("Number of HTTP redirects emitted by the server."),
                }));
                return Ok(());
            }
        }
    }

    apply_resolved_file_to_context(
        ctx,
        resolved_file,
        file_pipeline,
        timeout,
        root_path,
        parent_span_key,
    )
    .await
}

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

    let mut resolved_file = resolved_file;
    resolved_file.etag = resolved_file.compute_etag();

    let cache_key = (root_path.to_path_buf(), request_path.to_string());
    PATH_RESOLVE_CACHE.insert(cache_key, Timestamped::new(resolved_file.clone()));
    Ok(Some(resolved_file))
}

async fn apply_resolved_file_to_context(
    ctx: &mut HttpContext,
    resolved_file: ResolvedHttpFile,
    file_pipeline: &Pipeline<HttpFileContext>,
    timeout: Option<std::time::Duration>,
    root_path: PathBuf,
    parent_span_key: Option<&str>,
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

    let has_traces = parent_span_key.is_some() && ctx.events.has_trace_sinks();
    let mut stage_hooks = PerStageSpanHooks::new(
        &ctx.events,
        has_traces,
        parent_span_key.unwrap_or(""),
        "file",
    );
    let pipeline_result = if let Some(timeout) = timeout {
        if has_traces {
            vibeio::time::timeout(timeout, async {
                let executed_stages = file_pipeline
                    .execute_without_inverse_with_hooks(&mut file_ctx, &mut stage_hooks)
                    .await?;
                file_pipeline
                    .execute_inverse_with_hooks(&mut file_ctx, executed_stages, &mut stage_hooks)
                    .await
            })
            .await
        } else {
            vibeio::time::timeout(timeout, file_pipeline.execute(&mut file_ctx)).await
        }
    } else if has_traces {
        Ok(async {
            let executed_stages = file_pipeline
                .execute_without_inverse_with_hooks(&mut file_ctx, &mut stage_hooks)
                .await?;
            file_pipeline
                .execute_inverse_with_hooks(&mut file_ctx, executed_stages, &mut stage_hooks)
                .await
        }
        .await)
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

async fn resolve_http_file_target(
    root_path: &Path,
    request_path: &str,
    index_files: Option<&[String]>,
) -> Result<Option<ResolvedHttpFile>, FilePipelineExecutionError> {
    if !request_path.starts_with('/') {
        return Ok(None);
    }

    let canonical_root = match vibeio::fs::canonicalize(root_path).await {
        Ok(path) => path,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(FilePipelineExecutionError::WebrootNotFound)
        }
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            return Err(FilePipelineExecutionError::Forbidden)
        }
        Err(e) if e.kind() == io::ErrorKind::InvalidFilename => {
            return Err(FilePipelineExecutionError::BadRequest)
        }
        Err(e) => return Err(FilePipelineExecutionError::Io(e)),
    };

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

                if metadata.is_dir() {
                    if let Some(index_files) = index_files {
                        if let Some(index_file) =
                            try_resolve_index_files(&candidate_path, index_files, &canonical_root)
                                .await?
                        {
                            return Ok(Some(ResolvedHttpFile {
                                metadata: index_file.metadata,
                                file_path: index_file.file_path,
                                path_info: build_path_info(
                                    &request_segments[candidate_depth..],
                                    trailing_slash,
                                ),
                                etag: String::new(),
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
                    etag: String::new(),
                };
                return Ok(Some(resolved));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                return Err(FilePipelineExecutionError::Forbidden)
            }
            Err(error) if error.kind() == io::ErrorKind::InvalidFilename => {
                return Err(FilePipelineExecutionError::BadRequest)
            }
            Err(error) if is_not_directory_like(&error) && candidate_depth > 0 => {
                candidate_depth -= 1;
            }
            Err(error) => return Err(FilePipelineExecutionError::Io(error)),
        }
    }
}

async fn try_resolve_index_files(
    directory: &Path,
    index_files: &[String],
    canonical_root: &Path,
) -> Result<Option<ResolvedHttpFile>, FilePipelineExecutionError> {
    for index in index_files {
        let index_path = directory.join(index);
        match vibeio::fs::metadata(&index_path).await {
            Ok(metadata) if metadata.is_file() => {
                let canonical = vibeio::fs::canonicalize(&index_path)
                    .await
                    .map_err(FilePipelineExecutionError::Io)?;
                if !canonical.starts_with(canonical_root) {
                    return Err(FilePipelineExecutionError::Forbidden);
                }

                return Ok(Some(ResolvedHttpFile {
                    metadata,
                    file_path: canonical,
                    path_info: None,
                    etag: String::new(),
                }));
            }
            Ok(_) => continue,
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

pub(super) fn strip_matched_path_prefix(
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
