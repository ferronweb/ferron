//! Static file serving stage with streaming I/O and optional zerocopy.

use std::io;

use bytes::Bytes;
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::StageConstraint;
use ferron_http::access_log::{custom_access_log_fields, CustomAccessLogField};
use ferron_http::file_descriptor::ReusedFile;
use ferron_http::span::HttpContextSpanExt;
use ferron_http::trace_context::current_event_trace_context;
use ferron_http::util::parse_q_value_header_grouped::parse_q_value_header_grouped;
use ferron_http::{HttpFileContext, HttpRequest, HttpResponse};
use ferron_observability::{
    Event, MetricAttributeValue, MetricEvent, MetricType, MetricValue, TraceAttributeValue,
};
use futures_util::TryStreamExt;
use http::header::{self, HeaderValue};
use http::{HeaderMap, Method, Response, StatusCode};
use http_body::Frame;
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, Empty, StreamBody};
pub struct StaticFileStage;

static STATIC_FILE_BYTES_BUCKETS: &[f64] = &[
    1024.0,
    10240.0,
    102400.0,
    1048576.0,
    10485760.0,
    104857600.0,
];

use crate::util::compression::{
    compress_streaming_brotli, compress_streaming_deflate, compress_streaming_gzip,
    compress_streaming_zstd, Compression, NON_COMPRESSIBLE_FILE_EXTENSIONS,
    PREFERRED_CONTENT_ENCODING,
};
use crate::util::etag::{
    build_response_header_map, construct_etag, extract_etag_inner, split_etag_request,
};
use crate::util::file_stream::FileStream;
use crate::util::mime::get_content_type;
use crate::util::multipart_byterange::MultipartByterangeBody;
use crate::util::range::parse_range_header;

/// Helper: emit the static response metric AND insert the span attribute.
fn emit_static_response_metric_and_span(
    ctx: &mut HttpFileContext,
    status_code: u16,
    outcome: &'static str,
) {
    ctx.http.events.emit(Event::Metric(MetricEvent {
        name: "ferron.static.responses",
        attributes: vec![
            (
                "http.response.status_code",
                MetricAttributeValue::I64(status_code as i64),
            ),
            (
                "ferron.static.outcome",
                MetricAttributeValue::StaticStr(outcome),
            ),
        ],
        ty: MetricType::Counter,
        value: MetricValue::U64(1),
        unit: Some("{response}"),
        description: Some("Number of static file responses by outcome."),
        trace_context: current_event_trace_context(&ctx.http),
    }));
    ctx.get_span_attributes().insert(
        "http.response.status_code",
        TraceAttributeValue::I64(status_code as i64),
    );
}

/// Helper: set a pre-constructed HttpResponse, emit the response metric + span attribute.
fn respond_with_httpresponse(
    ctx: &mut HttpFileContext,
    request: HttpRequest,
    res: HttpResponse,
    status_code: u16,
    outcome: &'static str,
) -> Result<bool, PipelineError> {
    ctx.http.req = Some(request);
    ctx.http.res = Some(res);
    emit_static_response_metric_and_span(ctx, status_code, outcome);
    Ok(false)
}

/// Helper: set a builtin error response, emit the response metric + span attribute.
#[inline]
fn respond_with_builtin(
    ctx: &mut HttpFileContext,
    request: HttpRequest,
    status_code: u16,
    headers: Option<HeaderMap>,
    outcome: &'static str,
) -> Result<bool, PipelineError> {
    respond_with_httpresponse(
        ctx,
        request,
        HttpResponse::BuiltinError(status_code, headers),
        status_code,
        outcome,
    )
}

/// Helper: build a partial content response for a single range.
/// Helper: get or create an ETag value from the file's metadata.
fn get_or_create_etag(ctx: &HttpFileContext) -> String {
    if !ctx.etag.is_empty() {
        return ctx.etag.clone();
    }
    if let Some(file) = &ctx.file {
        if let Ok(meta) = file.metadata() {
            if let Ok(mdate) = meta.modified() {
                let etag_value = format!("{mdate:?}");
                return construct_etag(&etag_value, None, true);
            }
        }
    }
    construct_etag("default", None, true)
}

/// Helper: build a vary header based on compression capabilities and ETag settings.
fn build_vary_header(compression_possible: bool, etag_enabled: bool) -> &'static str {
    if !compression_possible || !etag_enabled {
        "If-Modified-Since, If-Range, If-Unmodified-Since, Range"
    } else {
        "Accept-Encoding, If-Match, If-None-Match, If-Modified-Since, If-Range, If-Unmodified-Since, Range"
    }
}

#[async_trait::async_trait(?Send)]
impl Stage<HttpFileContext> for StaticFileStage {
    #[inline]
    fn name(&self) -> &str {
        "static_file"
    }

    #[inline]
    fn constraints(&self) -> Vec<StageConstraint> {
        vec![StageConstraint::After("directory_listing".to_string())]
    }

    #[inline]
    async fn run(&self, ctx: &mut HttpFileContext) -> Result<bool, PipelineError> {
        // Skip if root is not configured
        if ctx.http.configuration.get_value("root", true).is_none() {
            return Ok(true);
        }

        let Some(request) = ctx.http.req.take() else {
            return Ok(true);
        };

        // Take the file handle from context and get metadata from FD
        let Some(file) = ctx.file.take() else {
            ctx.http.req = Some(request);
            return Ok(true);
        };
        let metadata = file
            .metadata()
            .map_err(|e| PipelineError::custom(format!("failed to get file metadata: {e}")))?;

        // Only handle files
        if ctx.path_info.is_some() || !metadata.is_file() {
            ctx.http.req = Some(request);
            ctx.file = Some(file);
            return Ok(true);
        }

        let file_path_str = ctx.file_path.to_string_lossy().to_string();
        ctx.get_span_attributes().insert(
            "ferron.static.file_path",
            TraceAttributeValue::String(file_path_str.clone()),
        );
        custom_access_log_fields(&mut ctx.http).insert(
            "ferron.static.file_path".into(),
            CustomAccessLogField::String(file_path_str),
        );

        let method = request.method().clone();

        // Handle OPTIONS
        if method == Method::OPTIONS {
            let res = Response::builder()
                .status(StatusCode::NO_CONTENT)
                .header(header::ALLOW, "GET, HEAD, POST, OPTIONS")
                .body(Empty::new().map_err(|_| unreachable!()).boxed_unsync())
                .expect("failed to build OPTIONS response");
            ctx.http.req = Some(request);
            ctx.http.res = Some(HttpResponse::Custom(res));
            emit_static_response_metric_and_span(ctx, 204, "options");
            ctx.get_span_attributes()
                .insert("http.response.status_code", TraceAttributeValue::I64(204));
            return Ok(false);
        }

        // Only handle GET and HEAD (and POST for some use cases)
        if method != Method::GET && method != Method::HEAD && method != Method::POST {
            let mut allow_headers = http::HeaderMap::new();
            allow_headers.insert(
                header::ALLOW,
                HeaderValue::from_static("GET, HEAD, POST, OPTIONS"),
            );
            return respond_with_builtin(
                ctx,
                request,
                405,
                Some(allow_headers),
                "method_not_allowed",
            );
        }

        // Read configuration
        let config = &ctx.http.configuration;

        // Compressed (on-the-fly)
        let compressed = config
            .get_value("compressed", true)
            .and_then(|v| v.as_boolean())
            .unwrap_or(true);

        // Precompressed file serving
        let precompressed = config.get_flag("precompressed", true);

        // ETag generation
        let etag_enabled = config
            .get_value("etag", true)
            .and_then(|v| v.as_boolean())
            .unwrap_or(true);

        // Cache-Control header for static files
        let cache_control = config
            .get_value("file_cache_control", true)
            .and_then(|v| v.as_str().map(|s| s.to_string()));

        // Determine content type
        let content_type = get_content_type(&ctx.file_path, config);

        // Check if compression is possible
        let compression_possible = compressed && {
            let file_len = metadata.len();
            let ext = ctx
                .file_path
                .extension()
                .map(|e| e.to_string_lossy().to_string())
                .unwrap_or_default();
            file_len > 256 && !NON_COMPRESSIBLE_FILE_EXTENSIONS.contains(ext.as_str())
        };

        let etag_value: Option<String> = if etag_enabled {
            Some(get_or_create_etag(&ctx))
        } else {
            None
        };

        // Pre-declare used_compression so precondition handlers can reference it
        let mut used_compression = Compression::Identity;

        // Build vary header based on compression capabilities and ETag settings
        let vary_header: Option<HeaderValue> =
            HeaderValue::from_str(build_vary_header(compression_possible, etag_enabled)).ok();

        // If-Match -> If-Unmodified-Since -> If-None-Match -> If-Modified-Since
        if let Some(_etag) = &etag_value {
            if let Some(if_match_value) = request.headers().get(header::IF_MATCH) {
                match if_match_value.to_str() {
                    Ok(if_match) => {
                        // "*" means any version is acceptable (RFC 7232 §3.1)
                        if !split_etag_request(if_match)
                            .into_iter()
                            .any(|tag| tag == "*")
                        {
                            // Precondition failed when method is not GET or HEAD
                            let header_map = build_response_header_map(
                                (!request.headers().contains_key(header::RANGE))
                                    .then_some(())
                                    .and_then(|_| etag_value.as_deref()),
                                None,
                                vary_header,
                                None,
                                cache_control.as_deref(),
                            );
                            return respond_with_builtin(
                                ctx,
                                request,
                                412,
                                Some(header_map),
                                "precondition_failed",
                            );
                        }

                        // Ferron only emits weak ETags, and strong comparison won't match
                        for tag in split_etag_request(if_match) {
                            if let Some((extracted, _, _)) = extract_etag_inner(&tag, true) {
                                if extracted == etag_value.as_deref().unwrap_or("") {
                                    let header_map = build_response_header_map(
                                        (!request.headers().contains_key(header::RANGE))
                                            .then_some(())
                                            .and_then(|_| etag_value.as_deref()),
                                        None,
                                        vary_header,
                                        None,
                                        cache_control.as_deref(),
                                    );
                                    return respond_with_builtin(
                                        ctx,
                                        request,
                                        412,
                                        Some(header_map),
                                        "precondition_failed",
                                    );
                                }
                            }
                        }

                        // No match found - 304 Not Modified
                        let etag_str = etag_value.as_deref().unwrap_or("");
                        let full_etag = format!("W/\"{etag_str}\"");
                        let mut builder = Response::builder()
                            .status(StatusCode::NOT_MODIFIED)
                            .header(header::ETAG, &full_etag);

                        if let Some(cc) = cache_control.as_deref() {
                            builder = builder.header(
                                header::CACHE_CONTROL,
                                HeaderValue::from_str(cc)
                                    .unwrap_or_else(|_| HeaderValue::from_static("")),
                            );
                        }

                        if let Some(v) = vary_header {
                            builder = builder.header(header::VARY, v);
                        }
                        let response = builder
                            .body(Empty::new().map_err(|_| unreachable!()).boxed_unsync())
                            .expect("failed to build 304 response");
                        ctx.http.req = Some(request);
                        ctx.http.res = Some(HttpResponse::Custom(response));
                        emit_static_response_metric_and_span(ctx, 304, "not_modified");
                        return Ok(false);
                    }
                    Err(_) => {
                        let header_map = build_response_header_map(
                            (!request.headers().contains_key(header::RANGE))
                                .then_some(())
                                .and_then(|_| etag_value.as_deref()),
                            None,
                            vary_header,
                            None,
                            cache_control.as_deref(),
                        );
                        return respond_with_builtin(
                            ctx,
                            request,
                            400,
                            Some(header_map),
                            "bad_request",
                        );
                    }
                }
            }
        }

        // Handle If-Unmodified-Since
        if let Some(if_unmodified_since) = request.headers().get(header::IF_UNMODIFIED_SINCE) {
            match if_unmodified_since
                .to_str()
                .ok()
                .and_then(|ius| httpdate::parse_http_date(ius).ok())
            {
                Some(if_unmodified_since) => {
                    if let Ok(mdate) = metadata.modified() {
                        if mdate > if_unmodified_since {
                            let header_map = build_response_header_map(
                                None,
                                Some(&mdate),
                                vary_header,
                                None,
                                cache_control.as_deref(),
                            );
                            return respond_with_builtin(
                                ctx,
                                request,
                                412,
                                Some(header_map),
                                "precondition_failed",
                            );
                        }
                    }
                }
                None => {
                    let header_map = build_response_header_map(
                        None,
                        metadata.modified().ok().as_ref(),
                        vary_header,
                        None,
                        cache_control.as_deref(),
                    );
                    ctx.http.req = Some(request);
                    ctx.http.res = Some(HttpResponse::BuiltinError(400, Some(header_map)));
                    emit_static_response_metric_and_span(ctx, 400, "bad_request");
                    return Ok(false);
                }
            }
        }

        // Handle If-None-Match
        if let Some(_etag) = &etag_value {
            if let Some(if_none_match) = request.headers().get(header::IF_NONE_MATCH) {
                if let Ok(val) = if_none_match.to_str() {
                    for tag in split_etag_request(val) {
                        if let Some((extracted, _, _)) = extract_etag_inner(&tag, true) {
                            if extracted == etag_value.as_deref().unwrap_or("") {
                                // RFC 7232 mandates that clients MUST NOT use weak validators
                                // for range requests
                                if !matches!(request.method(), &Method::GET | &Method::HEAD)
                                    || request.headers().contains_key(header::RANGE)
                                {
                                    let header_map = build_response_header_map(
                                        (!request.headers().contains_key(header::RANGE))
                                            .then_some(())
                                            .and_then(|_| etag_value.as_deref()),
                                        None,
                                        vary_header,
                                        None,
                                        cache_control.as_deref(),
                                    );
                                    return respond_with_builtin(
                                        ctx,
                                        request,
                                        412,
                                        Some(header_map),
                                        "precondition_failed",
                                    );
                                }

                                let suffix = used_compression.etag_suffix().unwrap_or("");
                                let etag_str = etag_value.as_deref().unwrap_or("");
                                let full_etag = construct_etag(etag_str, Some(suffix), true);
                                let mut builder = Response::builder()
                                    .status(StatusCode::NOT_MODIFIED)
                                    .header(header::ETAG, &full_etag)
                                    .header(
                                        header::VARY,
                                        vary_header.unwrap_or_else(|| HeaderValue::from_static("")),
                                    );

                                if let Some(cc) = cache_control.as_deref() {
                                    builder = builder.header(
                                        header::CACHE_CONTROL,
                                        HeaderValue::from_str(cc)
                                            .unwrap_or_else(|_| HeaderValue::from_static("")),
                                    );
                                }

                                let response = builder
                                    .body(Empty::new().map_err(|_| unreachable!()).boxed_unsync())
                                    .expect("failed to build 304 response");
                                ctx.http.req = Some(request);
                                ctx.http.res = Some(HttpResponse::Custom(response));
                                emit_static_response_metric_and_span(ctx, 304, "not_modified");
                                return Ok(false);
                            }
                        }
                    }
                }
            }
        }

        // Handle If-Modified-Since
        if let Some(if_modified_since) = request.headers().get(header::IF_MODIFIED_SINCE) {
            match if_modified_since
                .to_str()
                .ok()
                .and_then(|ims| httpdate::parse_http_date(ims).ok())
            {
                Some(if_modified_since) => {
                    if let Ok(mdate) = metadata.modified() {
                        if mdate <= if_modified_since {
                            let mut builder = Response::builder().status(StatusCode::NOT_MODIFIED);

                            if let Some(cc) = cache_control.as_deref() {
                                builder = builder.header(
                                    header::CACHE_CONTROL,
                                    HeaderValue::from_str(cc)
                                        .unwrap_or_else(|_| HeaderValue::from_static("")),
                                );
                            }

                            let response = builder
                                .body(Empty::new().map_err(|_| unreachable!()).boxed_unsync())
                                .expect("failed to build 304 response");
                            ctx.http.req = Some(request);
                            ctx.http.res = Some(HttpResponse::Custom(response));
                            emit_static_response_metric_and_span(ctx, 304, "not_modified");
                            return Ok(false);
                        }
                    }
                }
                None => {
                    let header_map = build_response_header_map(
                        None,
                        metadata.modified().ok().as_ref(),
                        vary_header,
                        None,
                        cache_control.as_deref(),
                    );
                    ctx.http.req = Some(request);
                    ctx.http.res = Some(HttpResponse::BuiltinError(400, Some(header_map)));
                    emit_static_response_metric_and_span(ctx, 400, "bad_request");
                    return Ok(false);
                }
            }
        }

        // Determine compression method
        let mut precompressed_exts: Vec<&str> = Vec::new();

        if compression_possible {
            let user_agent = request
                .headers()
                .get(header::USER_AGENT)
                .and_then(|h| h.to_str().ok())
                .unwrap_or("");

            let (broken_html, broken_compression) = if user_agent.starts_with("Mozilla/4.") {
                let rest = &user_agent[9..];
                if user_agent.contains(" MSIE ") {
                    (false, false)
                } else {
                    let m0 = rest.chars().nth(0).unwrap_or(' ');
                    (
                        !m0.is_ascii_alphabetic(),
                        matches!(
                            rest.chars().nth(1),
                            Some('0') | Some('6') | Some('7') | Some('8')
                        ),
                    )
                }
            } else {
                (false, false)
            };

            let is_text_html = content_type.as_deref() == Some("text/html");
            let skip_compression = (is_text_html
                && (broken_html || user_agent.starts_with("w3m/")))
                || broken_compression;

            if !skip_compression {
                if let Some(accept_enc) = request.headers().get(header::ACCEPT_ENCODING) {
                    if let Ok(accept_enc_str) = accept_enc.to_str() {
                        for enc in parse_q_value_header_grouped(accept_enc_str) {
                            let mut compression_found = false;
                            for penc in PREFERRED_CONTENT_ENCODING.iter() {
                                if enc.contains(*penc) {
                                    if let Some(compression) = Compression::from_header_value(penc)
                                    {
                                        if !compression_found {
                                            used_compression = compression;
                                            compression_found = true;
                                        }
                                        if precompressed {
                                            precompressed_exts.push(
                                                used_compression.precompressed_ext().unwrap_or(""),
                                            );
                                        } else {
                                            break;
                                        }
                                    }
                                }
                            }
                            if !precompressed && compression_found {
                                break;
                            }
                        }
                    }
                }
            }
        }

        // Determine precompressed file
        let mut file_path = ctx.file_path.clone();
        let mut file_length = metadata.len();
        let mut is_precompressed_file = false;

        if precompressed {
            for ext in &precompressed_exts {
                if !ext.is_empty() {
                    let mut precomp_path = ctx.file_path.clone();
                    if let Some(orig_ext) = ctx.file_path.extension() {
                        let orig_ext_str = orig_ext.to_string_lossy();
                        let new_ext = format!("{}.{}", orig_ext_str, ext);
                        precomp_path.set_extension(new_ext);
                    } else {
                        precomp_path.set_extension(ext);
                    }

                    if let Ok(file) = ReusedFile::open(&precomp_path).await {
                        if let Ok(meta) = file.metadata() {
                            if meta.is_file() {
                                ctx.get_span_attributes().insert(
                                    "ferron.static.file_path_precompressed",
                                    TraceAttributeValue::String(
                                        precomp_path.to_string_lossy().to_string(),
                                    ),
                                );
                                custom_access_log_fields(&mut ctx.http).insert(
                                    "ferron.static.file_path_precompressed".into(),
                                    CustomAccessLogField::String(
                                        precomp_path.to_string_lossy().to_string(),
                                    ),
                                );
                                file_path = precomp_path;
                                file_length = meta.len();
                                is_precompressed_file = true;
                                used_compression = Compression::from_precompressed_ext(ext);
                                break;
                            }
                        }
                    }
                } else {
                    used_compression = Compression::Identity;
                    break;
                }
            }
        }

        ctx.get_span_attributes().insert(
            "ferron.static.precompressed",
            TraceAttributeValue::Bool(is_precompressed_file),
        );

        // Handle If-Range (RFC 7233 §3.2)
        if request.headers().contains_key(header::RANGE) {
            let if_range_matches = etag_enabled;
            if if_range_matches {
                if let Some(if_range_date) = get_if_range_date(ctx, &request) {
                    if let Ok(mdate) = metadata.modified() {
                        if mdate != if_range_date {
                            return respond_with_builtin(
                                ctx,
                                request,
                                412,
                                None,
                                "precondition_failed",
                            );
                        }
                    } else {
                        return respond_with_builtin(
                            ctx,
                            request,
                            416,
                            None,
                            "range_not_satisfiable",
                        );
                    }
                }
            }

            if let Some(range_val) = request.headers().get(header::RANGE) {
                if let Ok(range_str) = range_val.to_str() {
                    match parse_range_header(range_str, file_length.saturating_sub(1)) {
                        Ok(ranges) => {
                            // Check if ranges can be satisfied
                            if ranges.is_empty()
                                || ranges.iter().any(|(start, _)| *start >= file_length)
                            {
                                let vary_str = etag_enabled
                                    .then_some("Accept-Encoding, If-Match, If-None-Match, If-Modified-Since, If-Range, If-Unmodified-Since, Range")
                                    .or(Some("If-Modified-Since, If-Range, If-Unmodified-Since, Range"));

                                let mut header_map = http::HeaderMap::new();
                                header_map.insert(
                                    header::CONTENT_RANGE,
                                    HeaderValue::from_str(&format!("bytes */{file_length}"))
                                        .expect("invalid content range header"),
                                );
                                if let Some(v) = vary_str {
                                    header_map.insert(
                                        header::VARY,
                                        HeaderValue::from_str(v)
                                            .unwrap_or_else(|_| HeaderValue::from_static("")),
                                    );
                                }

                                return respond_with_builtin(
                                    ctx,
                                    request,
                                    416,
                                    Some(header_map),
                                    "range_not_satisfiable",
                                );
                            }

                            if ranges.len() > 1 {
                                // Multiple non-overlapping ranges → multipart byterange
                                let boundaries = hex::encode(rand::random::<[u8; 12]>());
                                let vary_str = etag_enabled
                                    .then_some("Accept-Encoding, If-Match, If-None-Match, If-Modified-Since, If-Range, If-Unmodified-Since, Range")
                                    .or(Some("If-Modified-Since, If-Range, If-Unmodified-Since, Range"));

                                let mut builder = Response::builder()
                                    .status(StatusCode::PARTIAL_CONTENT)
                                    .header(header::CONTENT_TYPE, "multipart/byteranges");

                                if let Ok(mdate) = metadata.modified() {
                                    builder = builder.header(
                                        header::LAST_MODIFIED,
                                        httpdate::fmt_http_date(mdate),
                                    );
                                }

                                if let Some(etag) = &etag_value {
                                    let suffix = used_compression.etag_suffix().unwrap_or("");
                                    let full_etag = format!("W/\"{etag}{suffix}\"");
                                    builder = builder.header(header::ETAG, &full_etag);
                                }

                                if let Some(cc) = cache_control.as_deref() {
                                    builder = builder.header(
                                        header::CACHE_CONTROL,
                                        HeaderValue::from_str(cc)
                                            .unwrap_or_else(|_| HeaderValue::from_static("")),
                                    );
                                }

                                if let Some(v) = vary_str {
                                    builder = builder.header(
                                        header::VARY,
                                        HeaderValue::from_str(v)
                                            .unwrap_or_else(|_| HeaderValue::from_static("")),
                                    );
                                }

                                if method == Method::HEAD {
                                    let response = builder
                                        .body(
                                            Empty::new().map_err(|_| unreachable!()).boxed_unsync(),
                                        )
                                        .expect("failed to build HEAD response");
                                    return respond_with_httpresponse(
                                        ctx,
                                        request,
                                        HttpResponse::Custom(response),
                                        206,
                                        "partial_content",
                                    );
                                }

                                // Use MultipartByterangeBody for multiple ranges
                                let boundary = format!("{boundaries}--");
                                let multipart_body = MultipartByterangeBody::new(
                                    boundary,
                                    file_length,
                                    None,
                                    ranges.into(),
                                    FileStream::new(file, 0, Some(file_length)),
                                );
                                let response = builder
                                    .body(multipart_body.boxed_unsync())
                                    .expect("failed to build partial content response");
                                return respond_with_httpresponse(
                                    ctx,
                                    request,
                                    HttpResponse::Custom(response),
                                    206,
                                    "partial_content",
                                );
                            }

                            // Single range → single FileStream
                            if let Some((start, end)) = ranges.first().map(|(s, e)| (*s, *e)) {
                                let end = end.min(file_length - 1);
                                let content_len = end - start + 1;

                                let vary_str = etag_enabled
                                    .then_some("Accept-Encoding, If-Match, If-None-Match, If-Modified-Since, If-Range, If-Unmodified-Since, Range")
                                    .or(Some("If-Modified-Since, If-Range, If-Unmodified-Since, Range"));

                                let mut builder = Response::builder()
                                    .status(StatusCode::PARTIAL_CONTENT)
                                    .header(header::CONTENT_LENGTH, content_len)
                                    .header(
                                        header::CONTENT_RANGE,
                                        format!("bytes {start}-{end}/{file_length}"),
                                    );

                                if let Ok(mdate) = metadata.modified() {
                                    builder = builder.header(
                                        header::LAST_MODIFIED,
                                        httpdate::fmt_http_date(mdate),
                                    );
                                }

                                if let Some(etag) = &etag_value {
                                    let suffix = used_compression.etag_suffix().unwrap_or("");
                                    let full_etag = format!("W/\"{etag}{suffix}\"");
                                    builder = builder.header(header::ETAG, &full_etag);
                                }

                                if let Some(cc) = cache_control.as_deref() {
                                    builder = builder.header(
                                        header::CACHE_CONTROL,
                                        HeaderValue::from_str(cc)
                                            .unwrap_or_else(|_| HeaderValue::from_static("")),
                                    );
                                }

                                if let Some(v) = vary_str {
                                    builder = builder.header(
                                        header::VARY,
                                        HeaderValue::from_str(v)
                                            .unwrap_or_else(|_| HeaderValue::from_static("")),
                                    );
                                }

                                if method == Method::HEAD {
                                    let response = builder
                                        .body(
                                            Empty::new().map_err(|_| unreachable!()).boxed_unsync(),
                                        )
                                        .expect("failed to build HEAD response");
                                    return respond_with_httpresponse(
                                        ctx,
                                        request,
                                        HttpResponse::Custom(response),
                                        206,
                                        "partial_content",
                                    );
                                }

                                // Build the streaming body based on compression type
                                let body = if is_precompressed_file {
                                    StreamBody::new(
                                        FileStream::new(file, start, Some(end + 1))
                                            .map_ok(Frame::data),
                                    )
                                    .boxed_unsync()
                                } else {
                                    match used_compression {
                                        Compression::Identity => {
                                            // For identity (no compression), use FileStream for streaming I/O
                                            let body_stream =
                                                FileStream::new(file, start, Some(end + 1))
                                                    .map_ok(Frame::data);
                                            StreamBody::new(body_stream).boxed_unsync()
                                        }
                                        Compression::Gzip => {
                                            compress_streaming_gzip(file, Some(end + 1))
                                        }
                                        Compression::Brotli => {
                                            compress_streaming_brotli(file, Some(end + 1))
                                        }
                                        Compression::Deflate => {
                                            compress_streaming_deflate(file, Some(end + 1))
                                        }
                                        Compression::Zstd => {
                                            compress_streaming_zstd(file, Some(end + 1))
                                        }
                                    }
                                };

                                let response = builder
                                    .body(body)
                                    .expect("failed to build partial content response");
                                return respond_with_httpresponse(
                                    ctx,
                                    request,
                                    HttpResponse::Custom(response),
                                    206,
                                    "partial_content",
                                );
                            } else {
                                // No valid ranges (empty after parsing) → 416 Not Satisfiable
                                let vary_str = etag_enabled
                                    .then_some("Accept-Encoding, If-Match, If-None-Match, If-Modified-Since, If-Range, If-Unmodified-Since, Range")
                                    .or(Some("If-Modified-Since, If-Range, If-Unmodified-Since, Range"));

                                let mut header_map = http::HeaderMap::new();
                                header_map.insert(
                                    header::CONTENT_RANGE,
                                    HeaderValue::from_str(&format!("bytes */{file_length}"))
                                        .expect("invalid content range header"),
                                );
                                if let Some(v) = vary_str {
                                    header_map.insert(
                                        header::VARY,
                                        HeaderValue::from_str(v)
                                            .unwrap_or_else(|_| HeaderValue::from_static("")),
                                    );
                                }

                                return respond_with_builtin(
                                    ctx,
                                    request,
                                    416,
                                    Some(header_map),
                                    "range_not_satisfiable",
                                );
                            }
                        }
                        Err(_) => {
                            return respond_with_builtin(
                                ctx,
                                request,
                                416,
                                None,
                                "range_not_satisfiable",
                            );
                        }
                    }
                }
            } else {
                // No Range header → proceed to 200 OK response
            }
        }

        // Build the main response (200 OK)
        let mut builder = Response::builder()
            .status(StatusCode::OK)
            .header(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));

        if let Ok(mdate) = metadata.modified() {
            builder = builder.header(header::LAST_MODIFIED, httpdate::fmt_http_date(mdate));
        }

        // ETag
        if let Some(etag) = &etag_value {
            let suffix = used_compression.etag_suffix().unwrap_or("");
            let precompressed_suffix = if is_precompressed_file {
                "-precompress"
            } else {
                ""
            };
            let full_etag = format!("W/\"{etag}{precompressed_suffix}{suffix}\"");
            builder = builder.header(header::ETAG, &full_etag);
        }

        // Vary
        if etag_enabled {
            builder = builder.header(
                    header::VARY,
                    "Accept-Encoding, If-Match, If-None-Match, If-Modified-Since, If-Range, If-Unmodified-Since, Range"
                );
        }

        // Content-Type
        if let Some(ct) = content_type {
            builder = builder.header(header::CONTENT_TYPE, ct);
        }

        // Cache-Control
        if let Some(cc) = cache_control.as_deref() {
            builder = builder.header(
                header::CACHE_CONTROL,
                HeaderValue::from_str(cc).unwrap_or_else(|_| HeaderValue::from_static("")),
            );
        }

        // Content-Length / Content-Encoding
        match used_compression {
            Compression::Identity => {
                builder = builder.header(header::CONTENT_LENGTH, file_length);
            }
            c => {
                if is_precompressed_file {
                    builder = builder.header(header::CONTENT_LENGTH, file_length);
                } else {
                    // Approximate compressed size (would need actual compression to know exact size)
                    // For now, use original file length as an estimate
                    let content_len = file_length / 2;
                    builder = builder.header(header::CONTENT_LENGTH, content_len);
                }

                if let Some(hv) = c.header_value() {
                    builder =
                        builder.header(header::CONTENT_ENCODING, HeaderValue::from_static(hv));
                }
            }
        }

        // HEAD request - return headers only
        if method == Method::HEAD {
            let response = builder
                .body(Empty::new().map_err(|_| unreachable!()).boxed_unsync())
                .expect("failed to build HEAD response");
            ctx.http.req = Some(request);
            ctx.http.res = Some(HttpResponse::Custom(response));
            emit_static_response_metric_and_span(ctx, 200, "head");
            ctx.get_span_attributes()
                .insert("http.response.status_code", TraceAttributeValue::I64(200));
            return Ok(false);
        }

        // Full file response — streaming I/O
        let file = if is_precompressed_file {
            ReusedFile::open(&file_path)
                .await
                .map_err(|e| PipelineError::custom(format!("failed to open file: {e}")))?
        } else {
            file
        };

        // Extract raw fd for zerocopy (from the vibeio file via its std::fs::File inner)
        #[cfg(unix)]
        let raw_fd = {
            use std::os::fd::AsRawFd;
            let std_file = file.as_raw_fd();
            Some(std_file as i64)
        };
        #[cfg(windows)]
        let raw_fd = {
            use std::os::windows::io::AsRawHandle;
            let std_file = file.as_raw_handle();
            Some(std_file as i64)
        };

        // Build the body based on compression type
        let body: UnsyncBoxBody<Bytes, io::Error> = if is_precompressed_file {
            StreamBody::new(FileStream::new(file, 0, Some(file_length)).map_ok(Frame::data))
                .boxed_unsync()
        } else {
            match used_compression {
                Compression::Identity => {
                    // For identity (no compression), use zerocopy if available
                    let body_stream =
                        FileStream::new(file, 0, Some(file_length)).map_ok(Frame::data);
                    StreamBody::new(body_stream).boxed_unsync()
                }
                Compression::Brotli => compress_streaming_brotli(file, Some(file_length)),
                Compression::Zstd => compress_streaming_zstd(file, Some(file_length)),
                Compression::Deflate => compress_streaming_deflate(file, Some(file_length)),
                Compression::Gzip => compress_streaming_gzip(file, Some(file_length)),
            }
        };

        let mut response = builder.body(body).expect("failed to build file response");

        // Enable zerocopy for uncompressed responses on Linux
        if !is_precompressed_file && used_compression == Compression::Identity {
            if let Some(handle) = raw_fd {
                #[cfg(unix)]
                {
                    use std::os::fd::RawFd;
                    unsafe { vibeio_http::install_zerocopy(&mut response, handle as RawFd) };
                }
                #[cfg(windows)]
                {
                    use std::os::windows::io::RawHandle;
                    unsafe { vibeio_http::install_zerocopy(&mut response, handle as RawHandle) };
                }
            }
        }

        ctx.http.req = Some(request);
        ctx.http.res = Some(HttpResponse::Custom(response));
        emit_static_response_metric_and_span(ctx, 200, "full");

        // Emit metrics for files served and bytes sent
        let compression_label = used_compression.header_value().unwrap_or("identity");
        let cache_hit = is_precompressed_file;
        let file_size = file_length;

        ctx.http.events.emit(Event::Metric(MetricEvent {
            name: "ferron.static.files_served",
            attributes: vec![
                (
                    "ferron.compression",
                    MetricAttributeValue::String(compression_label.to_string()),
                ),
                ("ferron.cache_hit", MetricAttributeValue::Bool(cache_hit)),
            ],
            ty: MetricType::Counter,
            value: MetricValue::U64(1),
            unit: Some("{file}"),
            description: Some("Number of static files served."),
            trace_context: current_event_trace_context(&ctx.http),
        }));

        ctx.http.events.emit(Event::Metric(MetricEvent {
            name: "ferron.static.bytes_sent",
            attributes: vec![
                (
                    "ferron.compression",
                    MetricAttributeValue::String(compression_label.to_string()),
                ),
                ("ferron.cache_hit", MetricAttributeValue::Bool(cache_hit)),
            ],
            ty: MetricType::Histogram(Some(std::borrow::Cow::Borrowed(STATIC_FILE_BYTES_BUCKETS))),
            value: MetricValue::F64(file_size as f64),
            unit: Some("By"),
            description: Some("Bytes sent for static file responses."),
            trace_context: current_event_trace_context(&ctx.http),
        }));

        Ok(false)
    }
}

/// Helper: get the If-Range date from the request headers.
fn get_if_range_date(
    _ctx: &HttpFileContext,
    request: &HttpRequest,
) -> Option<std::time::SystemTime> {
    if let Some(if_range) = request.headers().get(header::IF_RANGE) {
        // Extract just the date part (ignore range info like bytes=100-200)
        if let Ok(range_str) = if_range.to_str() {
            // The If-Range header can contain both a date and a range, e.g., "Wed, 01 Jan 2018 00:00:00 GMT; bytes=100-200"
            let date_part = range_str.split(';').next().unwrap_or("");
            return httpdate::parse_http_date(date_part).ok();
        }
    }

    None
}
