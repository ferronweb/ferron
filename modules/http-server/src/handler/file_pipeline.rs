use std::borrow::Cow;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use ferron_core::config::ServerConfigurationValue;
use ferron_core::pipeline::{Pipeline, PipelineError};
use ferron_http::access_log::{custom_access_log_fields, CustomAccessLogField};
use ferron_http::file_descriptor::{ReusedFile, SymlinkMode};
use ferron_http::{HttpContext, HttpFileContext, HttpResponse};
use ferron_observability::{
    Event, MetricAttributeValue, MetricEvent, MetricType, MetricValue, Parent, TraceAttributeValue,
    TraceEvent,
};
use http_body_util::BodyExt;
use rustc_hash::FxHashMap;
use typemap_rev::TypeMap;

use super::observability::PerStageSpanHooks;

struct ResolvedHttpFile {
    metadata: zincio::fs::Metadata,
    file_path: PathBuf,
    path_info: Option<String>,
    etag: String,
    file: ReusedFile,
    is_index_file: bool,
}

impl std::fmt::Debug for ResolvedHttpFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedHttpFile")
            .field("metadata", &self.metadata)
            .field("file_path", &self.file_path)
            .field("path_info", &self.path_info)
            .field("etag", &self.etag)
            .finish_non_exhaustive()
    }
}

impl ResolvedHttpFile {
    #[inline]
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
    Forbidden {
        request_path: String,
        last_candidate_path: Option<String>,
    },
    BadRequest {
        request_path: String,
    },
    Timeout,
    Io(io::Error),
    Pipeline(PipelineError),
}

impl std::fmt::Display for FilePipelineExecutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Forbidden {
                request_path,
                last_candidate_path,
                ..
            } => {
                write!(f, "forbidden: {request_path}")?;
                if let Some(candidate) = last_candidate_path {
                    write!(f, " (last candidate: {candidate})")?;
                }
                Ok(())
            }
            Self::BadRequest { request_path } => {
                write!(f, "bad request: {request_path}")
            }
            Self::Timeout => write!(f, "file resolution timeout"),
            Self::Io(e) => write!(f, "file resolution I/O error: {e}"),
            Self::Pipeline(e) => write!(f, "file pipeline error: {e}"),
        }
    }
}

pub(super) async fn execute_http_file_pipeline(
    ctx: &mut HttpContext,
    file_pipeline: &Pipeline<HttpFileContext>,
    timeout: Option<std::time::Duration>,
    parent_span_key: Option<&str>,
    control_plane_metadata: Option<Arc<std::collections::BTreeMap<String, String>>>,
) -> Result<(), FilePipelineExecutionError> {
    let Some(request_path_encoded) = ctx
        .req
        .as_ref()
        .map(|request| request.uri().path().to_string())
    else {
        return Ok(());
    };
    let request_path = urlencoding::decode(&request_path_encoded)
        .map_err(|_| FilePipelineExecutionError::BadRequest {
            request_path: request_path_encoded.clone(),
        })?
        .to_string();
    let Some(root_path) = resolve_webroot(ctx)? else {
        return Ok(());
    };

    let index_files = resolve_index_files(ctx);
    let disable_symlinks = resolve_disable_symlinks(ctx)?;

    let has_traces = parent_span_key.is_some() && ctx.events.has_trace_sinks();
    let file_resolve_span_key = if has_traces {
        let key = super::observability::next_span_key("file_resolve");
        ctx.events.emit(Event::Trace(TraceEvent::StartSpan {
            key: Cow::Owned(key.clone()),
            name: Cow::Borrowed("ferron.pipeline.file_resolve"),
            parent: parent_span_key.map(|k| Parent::ByKey(k.to_string())),
            trace_context: None,
            builder_attributes: vec![],
            attributes: vec![
                (
                    "ferron.file_resolve.request_path",
                    TraceAttributeValue::String(request_path.clone()),
                ),
                (
                    "ferron.file_resolve.root_path",
                    TraceAttributeValue::String(root_path.to_string_lossy().into_owned()),
                ),
            ],
            links: vec![],
            control_plane_metadata: control_plane_metadata.clone(),
        }));
        Some(key)
    } else {
        None
    };

    let resolution_result = resolve_http_file_target(
        &root_path,
        &request_path,
        Some(&index_files),
        disable_symlinks,
    )
    .await;

    match resolution_result {
        Ok(Some(resolved_file)) => {
            // End file resolution span with success attributes
            if let Some(span_key) = file_resolve_span_key {
                ctx.events.emit(Event::Trace(TraceEvent::EndSpan {
                    key: Cow::Owned(span_key),
                    name: Cow::Borrowed("ferron.pipeline.file_resolve"),
                    error: None,
                    attributes: vec![
                        (
                            "ferron.file_resolve.outcome",
                            TraceAttributeValue::StaticStr("resolved"),
                        ),
                        (
                            "ferron.file_resolve.resolved_path",
                            TraceAttributeValue::String(
                                resolved_file.file_path.to_string_lossy().into_owned(),
                            ),
                        ),
                    ],
                    control_plane_metadata: control_plane_metadata.clone(),
                }));
            }

            // Check trailing slash redirection if resolved file is a directory,
            // or if it's an index file (which would be in a directory, just not in the URL)
            if resolved_file.metadata.is_dir() || resolved_file.is_index_file {
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
                            trace_context: ferron_http::trace_context::current_event_trace_context(
                                ctx,
                            ),
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
                control_plane_metadata,
            )
            .await
        }
        Ok(None) => {
            // File not found...
            if let Some(span_key) = file_resolve_span_key {
                ctx.events.emit(Event::Trace(TraceEvent::EndSpan {
                    key: Cow::Owned(span_key),
                    name: Cow::Borrowed("ferron.pipeline.file_resolve"),
                    error: None,
                    attributes: vec![(
                        "ferron.file_resolve.outcome",
                        TraceAttributeValue::StaticStr("not_found"),
                    )],
                    control_plane_metadata: control_plane_metadata.clone(),
                }));
            }
            Ok(())
        }
        Err(error) => {
            let (outcome, last_candidate) = match &error {
                FilePipelineExecutionError::Forbidden {
                    last_candidate_path,
                    ..
                } => ("forbidden", last_candidate_path.clone()),
                FilePipelineExecutionError::BadRequest { .. } => ("bad_request", None),
                FilePipelineExecutionError::Io(_) => ("error", None),
                _ => ("error", None),
            };

            if let Some(span_key) = file_resolve_span_key {
                let mut attrs = vec![(
                    "ferron.file_resolve.outcome",
                    TraceAttributeValue::StaticStr(outcome),
                )];
                if let Some(ref candidate) = last_candidate {
                    attrs.push((
                        "ferron.file_resolve.last_candidate_path",
                        TraceAttributeValue::String(candidate.clone()),
                    ));
                }
                ctx.events.emit(Event::Trace(TraceEvent::EndSpan {
                    key: Cow::Owned(span_key),
                    name: Cow::Borrowed("ferron.pipeline.file_resolve"),
                    error: Some(error.to_string()),
                    attributes: attrs,
                    control_plane_metadata: control_plane_metadata.clone(),
                }));
            }

            // Inject access log fields for the resolution error
            inject_resolution_error_fields(ctx, &error, &request_path, &root_path);

            Err(error)
        }
    }
}

fn inject_resolution_error_fields(
    ctx: &mut HttpContext,
    error: &FilePipelineExecutionError,
    request_path: &str,
    root_path: &Path,
) {
    match error {
        FilePipelineExecutionError::Forbidden {
            request_path: _,
            last_candidate_path,
        } => {
            custom_access_log_fields(ctx).insert(
                "ferron.file_resolve.request_path".into(),
                CustomAccessLogField::String(request_path.to_string()),
            );
            custom_access_log_fields(ctx).insert(
                "ferron.file_resolve.root_path".into(),
                CustomAccessLogField::String(root_path.to_string_lossy().into_owned()),
            );
            custom_access_log_fields(ctx).insert(
                "ferron.file_resolve.outcome".into(),
                CustomAccessLogField::String("forbidden".into()),
            );
            if let Some(candidate) = last_candidate_path {
                custom_access_log_fields(ctx).insert(
                    "ferron.file_resolve.last_candidate_path".into(),
                    CustomAccessLogField::String(candidate.clone()),
                );
            }
        }
        FilePipelineExecutionError::BadRequest { .. } => {
            custom_access_log_fields(ctx).insert(
                "ferron.file_resolve.request_path".into(),
                CustomAccessLogField::String(request_path.to_string()),
            );
            custom_access_log_fields(ctx).insert(
                "ferron.file_resolve.root_path".into(),
                CustomAccessLogField::String(root_path.to_string_lossy().into_owned()),
            );
            custom_access_log_fields(ctx).insert(
                "ferron.file_resolve.outcome".into(),
                CustomAccessLogField::String("bad_request".into()),
            );
        }
        FilePipelineExecutionError::Io(_) => {
            custom_access_log_fields(ctx).insert(
                "ferron.file_resolve.request_path".into(),
                CustomAccessLogField::String(request_path.to_string()),
            );
            custom_access_log_fields(ctx).insert(
                "ferron.file_resolve.root_path".into(),
                CustomAccessLogField::String(root_path.to_string_lossy().into_owned()),
            );
            custom_access_log_fields(ctx).insert(
                "ferron.file_resolve.outcome".into(),
                CustomAccessLogField::String("error".into()),
            );
        }
        _ => {}
    }
}

async fn apply_resolved_file_to_context(
    ctx: &mut HttpContext,
    resolved_file: ResolvedHttpFile,
    file_pipeline: &Pipeline<HttpFileContext>,
    timeout: Option<std::time::Duration>,
    root_path: PathBuf,
    parent_span_key: Option<&str>,
    control_plane_metadata: Option<Arc<std::collections::BTreeMap<String, String>>>,
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
        file_path: resolved_file.file_path.clone(),
        path_info: resolved_file.path_info.clone(),
        file_root: root_path,
        etag: resolved_file.etag.clone(),
        file: Some(resolved_file.file),
    };

    let has_traces = parent_span_key.is_some() && ctx.events.has_trace_sinks();
    let mut stage_hooks = PerStageSpanHooks::new(
        &ctx.events,
        has_traces,
        parent_span_key.unwrap_or(""),
        "file",
        control_plane_metadata,
    );
    let pipeline_result = if let Some(timeout) = timeout {
        if has_traces {
            zincio::time::timeout(timeout, async {
                let executed_stages = file_pipeline
                    .execute_without_inverse_with_hooks(&mut file_ctx, &mut stage_hooks)
                    .await?;
                file_pipeline
                    .execute_inverse_with_hooks(&mut file_ctx, executed_stages, &mut stage_hooks)
                    .await
            })
            .await
        } else {
            zincio::time::timeout(timeout, file_pipeline.execute(&mut file_ctx)).await
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

    drop(stage_hooks);

    *ctx = file_ctx.http;

    match pipeline_result {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(FilePipelineExecutionError::Pipeline(e)),
        Err(_) => Err(FilePipelineExecutionError::Timeout),
    }
}

#[inline]
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

#[inline]
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

#[inline]
fn resolve_disable_symlinks(ctx: &HttpContext) -> Result<SymlinkMode, FilePipelineExecutionError> {
    let value = ctx.configuration.get_entry("disable_symlinks", true);

    if let Some(s) = value.map(|v| {
        v.args
            .first()
            .unwrap_or(&ServerConfigurationValue::Boolean(false, None))
    }) {
        SymlinkMode::from_config_value(s).map_err(|e| {
            FilePipelineExecutionError::Io(io::Error::new(io::ErrorKind::InvalidInput, e))
        })
    } else {
        Ok(SymlinkMode::Off)
    }
}

async fn resolve_http_file_target(
    root_path: &Path,
    request_path: &str,
    index_files: Option<&[String]>,
    disable_symlinks: SymlinkMode,
) -> Result<Option<ResolvedHttpFile>, FilePipelineExecutionError> {
    if !request_path.starts_with('/') {
        return Ok(None);
    }

    let request_segments = request_path_segments(request_path)?;
    let mut candidate_depth = request_segments.len();
    let trailing_slash = request_path.ends_with('/') && request_path != "/";
    let req_str = request_path.to_string();

    loop {
        let candidate_path = build_candidate_path(root_path, &request_segments[..candidate_depth])
            .ok_or_else(|| FilePipelineExecutionError::Forbidden {
                request_path: req_str.clone(),
                last_candidate_path: None,
            })?;

        // Check for symlinks if enabled
        if disable_symlinks != SymlinkMode::Off {
            if let Err(e) =
                check_symlinks_in_path(&candidate_path, root_path, disable_symlinks).await
            {
                if e.kind() == io::ErrorKind::PermissionDenied {
                    return Err(FilePipelineExecutionError::Forbidden {
                        request_path: req_str.clone(),
                        last_candidate_path: Some(candidate_path.to_string_lossy().into_owned()),
                    });
                }
                // Continue with other checks
            }
        }

        // Optimization inspired by NGINX (kernel caching data)
        if let Some(idx) = index_files.and_then(|idf| idf.first()) {
            if let Some(index_file) = try_resolve_index_files(
                &candidate_path,
                std::slice::from_ref(idx),
                root_path,
                disable_symlinks,
                &req_str,
            )
            .await?
            {
                let mut resolved_file = index_file;
                resolved_file.etag = resolved_file.compute_etag();
                return Ok(Some(resolved_file));
            }
        }

        let reused_file = match ReusedFile::open(&candidate_path).await {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                return Err(FilePipelineExecutionError::Forbidden {
                    request_path: req_str,
                    last_candidate_path: Some(candidate_path.to_string_lossy().into_owned()),
                })
            }
            Err(error) if error.kind() == io::ErrorKind::InvalidFilename => {
                return Err(FilePipelineExecutionError::BadRequest {
                    request_path: req_str,
                })
            }
            Err(error) if is_not_directory_like(&error) && candidate_depth > 0 => {
                candidate_depth -= 1;
                continue;
            }
            Err(error) => return Err(FilePipelineExecutionError::Io(error)),
        };
        match reused_file.metadata() {
            Ok(metadata) => {
                if metadata.is_dir() {
                    if let Some(index_files) = index_files {
                        if let Some(index_file) = try_resolve_index_files(
                            &candidate_path,
                            &index_files[1..],
                            root_path,
                            disable_symlinks,
                            &req_str,
                        )
                        .await?
                        {
                            let mut resolved_file = index_file;
                            resolved_file.etag = resolved_file.compute_etag();
                            return Ok(Some(resolved_file));
                        }
                    }
                }

                let mut resolved = ResolvedHttpFile {
                    metadata,
                    file_path: candidate_path,
                    path_info: build_path_info(
                        &request_segments[candidate_depth..],
                        trailing_slash,
                    ),
                    etag: String::new(),
                    file: reused_file,
                    is_index_file: false,
                };
                resolved.etag = resolved.compute_etag();
                return Ok(Some(resolved));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                return Err(FilePipelineExecutionError::Forbidden {
                    request_path: req_str,
                    last_candidate_path: Some(candidate_path.to_string_lossy().into_owned()),
                })
            }
            Err(error) if error.kind() == io::ErrorKind::InvalidFilename => {
                return Err(FilePipelineExecutionError::BadRequest {
                    request_path: req_str,
                })
            }
            Err(error) if is_not_directory_like(&error) && candidate_depth > 0 => {
                candidate_depth -= 1;
            }
            Err(error) => return Err(FilePipelineExecutionError::Io(error)),
        }
    }
}

/// Check for symlinks in the path traversal chain (if enabled).
/// Returns an error if a forbidden symlink is found.
async fn check_symlinks_in_path(path: &Path, root: &Path, mode: SymlinkMode) -> io::Result<()> {
    if mode == SymlinkMode::Off {
        return Ok(());
    }

    // Walk the path components and check each for symlinks
    let mut current = root.to_path_buf();

    for component in path
        .strip_prefix(root)
        .unwrap_or_else(|_| Path::new(""))
        .components()
    {
        current.push(component);

        // Check if this component is a symlink
        match zincio::fs::symlink_metadata(&current).await {
            Ok(metadata) if metadata.is_symlink() => {
                match mode {
                    SymlinkMode::On => {
                        return Err(io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            "symlinks not allowed",
                        ));
                    }
                    SymlinkMode::IfNotOwner => {
                        // The UID check is Unix-specific, skip it on other platforms
                        // Based on NGINX disable_symlinks if_not_owner (UID comparison basically)
                        let mut same_owner = false;
                        #[cfg(unix)]
                        {
                            if let Ok(canonical_metadata) = zincio::fs::metadata(&current).await {
                                same_owner = metadata.uid() == canonical_metadata.uid();
                            }
                        }
                        if !same_owner {
                            return Err(io::Error::new(
                                io::ErrorKind::PermissionDenied,
                                "different-owner symlinks not allowed",
                            ));
                        }
                    }
                    SymlinkMode::Off => {} // Already checked above
                }
            }
            _ => {} // Not a symlink or doesn't exist, continue
        }
    }

    Ok(())
}

async fn try_resolve_index_files(
    directory: &Path,
    index_files: &[String],
    root: &Path,
    disable_symlinks: SymlinkMode,
    request_path: &str,
) -> Result<Option<ResolvedHttpFile>, FilePipelineExecutionError> {
    for index in index_files {
        let index_path = directory.join(index);

        // Check for symlinks if enabled
        if disable_symlinks != SymlinkMode::Off
            && check_symlinks_in_path(&index_path, root, disable_symlinks)
                .await
                .is_err()
        {
            continue;
        }

        let reused_file = match ReusedFile::open(&index_path).await {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) if error.kind() == io::ErrorKind::NotADirectory => continue,
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                return Err(FilePipelineExecutionError::Forbidden {
                    request_path: request_path.to_string(),
                    last_candidate_path: Some(index_path.to_string_lossy().into_owned()),
                });
            }
            Err(error) => {
                return Err(FilePipelineExecutionError::Io(error));
            }
        };
        match reused_file.metadata() {
            Ok(metadata) if metadata.is_file() => {
                return Ok(Some(ResolvedHttpFile {
                    metadata,
                    file_path: index_path,
                    path_info: None,
                    etag: String::new(),
                    file: reused_file,
                    is_index_file: true,
                }));
            }
            Ok(_) => continue,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) if error.kind() == io::ErrorKind::NotADirectory => continue,
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                return Err(FilePipelineExecutionError::Forbidden {
                    request_path: request_path.to_string(),
                    last_candidate_path: Some(index_path.to_string_lossy().into_owned()),
                });
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
    let req_str = request_path.to_string();

    for component in Path::new(request_path).components() {
        match component {
            Component::RootDir => {}
            Component::Normal(segment) => {
                let segment_str = segment.to_string_lossy().into_owned();
                // Reject segments containing ':' to prevent Windows Alternate Data Stream
                // (ADS) access (e.g., /file.txt::$DATA) and scheme injection.
                if segment_str.contains(':') {
                    return Err(FilePipelineExecutionError::Forbidden {
                        request_path: req_str,
                        last_candidate_path: None,
                    });
                }
                segments.push(segment_str);
            }
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => {
                return Err(FilePipelineExecutionError::Forbidden {
                    request_path: req_str,
                    last_candidate_path: None,
                });
            }
        }
    }

    Ok(segments)
}

#[inline]
fn build_candidate_path(root_path: &Path, request_segments: &[String]) -> Option<PathBuf> {
    let mut candidate_path = root_path.to_path_buf();
    for segment in request_segments {
        let segment_path: &Path = segment.as_ref();
        if segment_path.is_absolute() {
            // Prevent absolute path component path traversal, just in case
            return None;
        }
        candidate_path.push(segment_path);
    }
    Some(candidate_path)
}

#[inline]
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

#[inline]
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

        let resolved =
            resolve_http_file_target(&root.path, "/index.html/test", None, SymlinkMode::Off)
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

        let resolved = resolve_http_file_target(&root.path, "/missing.txt", None, SymlinkMode::Off)
            .await
            .expect("resolution should succeed");

        assert!(resolved.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn rejects_parent_directory_traversal() {
        let root = TestDir::new("parent-traversal");

        let error = resolve_http_file_target(&root.path, "/../secret.txt", None, SymlinkMode::Off)
            .await
            .expect_err("traversal should be rejected");

        assert!(matches!(
            error,
            FilePipelineExecutionError::Forbidden { .. }
        ));
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

        // With symlink protection enabled, it should reject
        let error = resolve_http_file_target(&root, "/escape.txt", None, SymlinkMode::On)
            .await
            .expect_err("symlink escape should be rejected");

        assert!(matches!(
            error,
            FilePipelineExecutionError::Forbidden { .. }
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn rejects_alternate_data_stream_access() {
        let root = TestDir::new("ads-rejection");
        std::fs::write(root.path.join("file.txt"), b"content").expect("failed to write file");

        // Windows ADS-style path: /file.txt::$DATA
        let error =
            resolve_http_file_target(&root.path, "/file.txt::$DATA", None, SymlinkMode::Off)
                .await
                .expect_err("ADS access should be rejected");

        assert!(matches!(
            error,
            FilePipelineExecutionError::Forbidden { .. }
        ));
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
