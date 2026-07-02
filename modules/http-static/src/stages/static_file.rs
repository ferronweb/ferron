//! Static file serving stage with streaming I/O and optional zerocopy.

use std::borrow::Cow;
use std::io;

use async_trait::async_trait;
use bytes::Bytes;
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::StageConstraint;
use ferron_http::file_descriptor::ReusedFile;
use ferron_http::span::HttpContextSpanExt;
use ferron_http::trace_context::current_event_trace_context;
use ferron_http::util::parse_q_value_header_grouped::parse_q_value_header_grouped;
use ferron_http::{HttpFileContext, HttpResponse};
use ferron_observability::{
    Event, MetricAttributeValue, MetricEvent, MetricType, MetricValue, TraceAttributeValue,
};
use futures_util::TryStreamExt;
use http::header::{self, HeaderValue};
use http::{HeaderMap, Method, Response, StatusCode};
use http_body::Frame;
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, Empty, StreamBody};

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
    build_etag_header_map, build_last_modified_header_map, construct_etag, extract_etag_inner,
    split_etag_request,
};
use crate::util::file_stream::FileStream;
use crate::util::mime::get_content_type;
use crate::util::multipart_byterange::MultipartByterangeBody;
use crate::util::range::{parse_range_header, RangeParseError};

pub struct StaticFileStage;

impl Default for StaticFileStage {
    #[inline]
    fn default() -> Self {
        Self
    }
}

fn emit_static_response_metric(ctx: &HttpFileContext, status_code: u16, outcome: &'static str) {
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
}

#[async_trait(?Send)]
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
        let file_path_str = ctx.file_path.to_string_lossy().to_string();
        ctx.get_span_attributes().insert(
            "ferron.static.file_path",
            TraceAttributeValue::String(file_path_str),
        );

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
            emit_static_response_metric(ctx, 204, "options");
            ctx.get_span_attributes()
                .insert("http.response.status_code", TraceAttributeValue::I64(204));
            return Ok(false);
        }

        // Only handle GET and HEAD
        if method != Method::GET && method != Method::HEAD && method != Method::POST {
            let mut allow_headers = http::HeaderMap::new();
            allow_headers.insert(
                header::ALLOW,
                HeaderValue::from_static("GET, HEAD, POST, OPTIONS"),
            );
            ctx.http.req = Some(request);
            ctx.http.res = Some(HttpResponse::BuiltinError(405, Some(allow_headers)));
            emit_static_response_metric(ctx, 405, "method_not_allowed");
            ctx.get_span_attributes()
                .insert("http.response.status_code", TraceAttributeValue::I64(405));
            return Ok(false);
        }

        let config = &ctx.http.configuration;

        // Read configuration
        let compressed = config
            .get_value("compressed", true)
            .and_then(|v| v.as_boolean())
            .unwrap_or(true);
        let precompressed = config.get_flag("precompressed", true);
        let etag_enabled = config
            .get_value("etag", true)
            .and_then(|v| v.as_boolean())
            .unwrap_or(true);
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

        let mut etag_value: Option<String> = None;
        #[allow(unused_assignments)]
        let mut vary_header: Option<HeaderValue> = None;

        let mdate = metadata.modified().ok();

        if etag_enabled {
            etag_value = Some(ctx.etag.clone());
            vary_header = Some(HeaderValue::from_static(if compression_possible {
                "Accept-Encoding, If-Match, If-Modified-Since, If-None-Match, If-Range, If-Unmodified-Since, Range"
            } else {
                "If-Match, If-Modified-Since, If-None-Match, If-Range, If-Unmodified-Since, Range"
            }));
        } else {
            vary_header = Some(HeaderValue::from_static(if compression_possible {
                "Accept-Encoding, If-Modified-Since, If-Range, If-Unmodified-Since, Range"
            } else {
                "If-Modified-Since, If-Range, If-Unmodified-Since, Range"
            }));
        }

        // If-Match -> If-Unmodified-Since -> If-None-Match -> If-Modified-Since
        // (RFC 7232 compliant order)

        // Handle If-Match (Ferron only emits weak ETags, so strong If-Match won't match)
        if let Some(etag) = &etag_value {
            if let Some(if_match_value) = request.headers().get(header::IF_MATCH) {
                match if_match_value.to_str() {
                    Ok(if_match) => {
                        // "*" means any version is acceptable (RFC 7232 §3.1)
                        // Check wildcard first, then method, then specific ETag
                        if !split_etag_request(if_match)
                            .into_iter()
                            .any(|tag| tag == "*")
                        {
                            if !matches!(request.method(), &Method::GET | &Method::HEAD) {
                                // Precondition failed when method is not GET or HEAD
                                let header_map = build_etag_header_map(
                                    etag,
                                    vary_header,
                                    None,
                                    cache_control.as_deref(),
                                );
                                ctx.http.req = Some(request);
                                ctx.http.res =
                                    Some(HttpResponse::BuiltinError(412, Some(header_map)));
                                emit_static_response_metric(ctx, 412, "precondition_failed");
                                ctx.get_span_attributes().insert(
                                    "http.response.status_code",
                                    TraceAttributeValue::I64(412),
                                );
                                return Ok(false);
                            }

                            // Ferron only emits weak ETags, and strong comparison won't match
                            // for specific ETags
                            let header_map = build_etag_header_map(
                                etag,
                                vary_header,
                                None,
                                cache_control.as_deref(),
                            );
                            ctx.http.req = Some(request);
                            ctx.http.res = Some(HttpResponse::BuiltinError(412, Some(header_map)));
                            emit_static_response_metric(ctx, 412, "precondition_failed");
                            ctx.get_span_attributes()
                                .insert("http.response.status_code", TraceAttributeValue::I64(412));
                            return Ok(false);
                        }
                    }
                    Err(_) => {
                        let header_map = build_etag_header_map(
                            etag,
                            vary_header,
                            None,
                            cache_control.as_deref(),
                        );
                        ctx.http.req = Some(request);
                        ctx.http.res = Some(HttpResponse::BuiltinError(400, Some(header_map)));
                        emit_static_response_metric(ctx, 400, "bad_request");
                        ctx.get_span_attributes()
                            .insert("http.response.status_code", TraceAttributeValue::I64(400));
                        return Ok(false);
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
                    if mdate
                        .as_ref()
                        .is_some_and(|mdate| mdate > &if_unmodified_since)
                    {
                        let header_map = build_last_modified_header_map(
                            mdate.as_ref(),
                            vary_header,
                            None,
                            cache_control.as_deref(),
                        );
                        ctx.http.req = Some(request);
                        ctx.http.res = Some(HttpResponse::BuiltinError(412, Some(header_map)));
                        emit_static_response_metric(ctx, 412, "precondition_failed");
                        ctx.get_span_attributes()
                            .insert("http.response.status_code", TraceAttributeValue::I64(412));
                        return Ok(false);
                    }
                }
                None => {
                    let header_map = build_last_modified_header_map(
                        mdate.as_ref(),
                        vary_header,
                        None,
                        cache_control.as_deref(),
                    );
                    ctx.http.req = Some(request);
                    ctx.http.res = Some(HttpResponse::BuiltinError(400, Some(header_map)));
                    emit_static_response_metric(ctx, 400, "bad_request");
                    ctx.get_span_attributes()
                        .insert("http.response.status_code", TraceAttributeValue::I64(400));
                    return Ok(false);
                }
            }
        }

        // Handle If-None-Match
        if let Some(etag) = &etag_value {
            if let Some(if_none_match) = request.headers().get(header::IF_NONE_MATCH) {
                if let Ok(val) = if_none_match.to_str() {
                    for tag in split_etag_request(val) {
                        if let Some((extracted, suffix_opt, _)) = extract_etag_inner(&tag, true) {
                            if &extracted == etag {
                                if !matches!(request.method(), &Method::GET | &Method::HEAD) {
                                    let header_map = build_etag_header_map(
                                        etag,
                                        vary_header,
                                        None,
                                        cache_control.as_deref(),
                                    );
                                    ctx.http.req = Some(request);
                                    ctx.http.res =
                                        Some(HttpResponse::BuiltinError(412, Some(header_map)));
                                    emit_static_response_metric(ctx, 412, "precondition_failed");
                                    ctx.get_span_attributes().insert(
                                        "http.response.status_code",
                                        TraceAttributeValue::I64(412),
                                    );
                                    return Ok(false);
                                }
                                let suffix = suffix_opt.and_then(|s| match s.as_str() {
                                    "gzip" | "deflate" | "br" | "zstd" => Some(s),
                                    _ => None,
                                });
                                let full_etag = construct_etag(etag, suffix.as_deref(), true);
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
                                emit_static_response_metric(ctx, 304, "not_modified");
                                ctx.get_span_attributes().insert(
                                    "http.response.status_code",
                                    TraceAttributeValue::I64(304),
                                );
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
                    if metadata
                        .modified()
                        .is_ok_and(|mdate| mdate <= if_modified_since)
                    {
                        let mut builder =
                            Response::builder().status(StatusCode::NOT_MODIFIED).header(
                                header::VARY,
                                vary_header.unwrap_or_else(|| HeaderValue::from_static("")),
                            );
                        if let Some(mdate) = &mdate {
                            builder = builder
                                .header(header::LAST_MODIFIED, httpdate::fmt_http_date(*mdate));
                        }
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
                        emit_static_response_metric(ctx, 304, "not_modified");
                        ctx.get_span_attributes()
                            .insert("http.response.status_code", TraceAttributeValue::I64(304));
                        return Ok(false);
                    }
                }
                None => {
                    let header_map = build_last_modified_header_map(
                        mdate.as_ref(),
                        vary_header,
                        None,
                        cache_control.as_deref(),
                    );
                    ctx.http.req = Some(request);
                    ctx.http.res = Some(HttpResponse::BuiltinError(400, Some(header_map)));
                    emit_static_response_metric(ctx, 400, "bad_request");
                    return Ok(false);
                }
            }
        }

        // Determine compression method
        let mut used_compression = Compression::Identity;
        let mut precompressed_exts: Vec<&str> = Vec::new();

        if compression_possible {
            // Check for browsers with known compression bugs
            let user_agent = request
                .headers()
                .get(header::USER_AGENT)
                .and_then(|h| h.to_str().ok())
                .unwrap_or("");
            let (broken_html, broken_compression) =
                if let Some(rest) = user_agent.strip_prefix("Mozilla/4.") {
                    if user_agent.contains(" MSIE ") {
                        (false, false)
                    } else {
                        (
                            true,
                            matches!(rest.chars().nth(0), Some('0'))
                                && matches!(rest.chars().nth(1), Some('6') | Some('7') | Some('8')),
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
                if let Some(accept_enc) = request
                    .headers()
                    .get(header::ACCEPT_ENCODING)
                    .and_then(|h| h.to_str().ok())
                {
                    for enc in parse_q_value_header_grouped(accept_enc) {
                        let mut compression_found = false;
                        for penc in PREFERRED_CONTENT_ENCODING {
                            if enc.contains(*penc) {
                                let compression = Compression::from_header_value(penc);
                                if let Some(compression) = compression {
                                    if !compression_found {
                                        used_compression = compression;
                                        compression_found = true;
                                    }
                                    if precompressed {
                                        precompressed_exts
                                            .push(compression.precompressed_ext().unwrap_or(""));
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

        // Handle precompressed files
        let mut file_path = ctx.file_path.clone();
        let mut file_length = metadata.len();
        let mut is_precompressed_file = false;

        if precompressed {
            for ext in precompressed_exts {
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
        // If-Range is ignored when If-Match or If-Unmodified-Since is present
        let if_range_matches = if request.headers().contains_key(header::RANGE) {
            if let Some(if_range) = request.headers().get(header::IF_RANGE) {
                if request.headers().contains_key(header::IF_MATCH)
                    || request.headers().contains_key(header::IF_UNMODIFIED_SINCE)
                {
                    true
                } else if let Ok(if_range_str) = if_range.to_str() {
                    let is_valid =
                        if let Ok(if_range_date) = httpdate::parse_http_date(if_range_str) {
                            mdate.as_ref().is_none_or(|mdate| *mdate == if_range_date)
                        } else if if_range_str == "*" {
                            true
                        } else if let Some(etag) = &etag_value {
                            split_etag_request(if_range_str)
                                .iter()
                                .filter_map(|tag| extract_etag_inner(tag, true))
                                .any(|(extracted, _, _)| &extracted == etag)
                        } else {
                            false
                        };
                    is_valid
                } else {
                    true
                }
            } else {
                true
            }
        } else {
            true
        };

        // Handle Range requests
        if let Some(range_val) = request.headers().get(header::RANGE) {
            if let Ok(range_str) = range_val.to_str() {
                if if_range_matches {
                    match parse_range_header(range_str, file_length.saturating_sub(1)) {
                        Ok(ranges) => {
                            if file_length == 0
                                || ranges
                                    .iter()
                                    .any(|(start, end)| *start >= file_length || *start > *end)
                            {
                                let vary = vary_header
                                    .unwrap_or_else(|| HeaderValue::from_static("Range"));
                                let mut header_map = HeaderMap::new();
                                header_map.insert(
                                    header::CONTENT_RANGE,
                                    HeaderValue::from_str(&format!("bytes */{file_length}"))
                                        .expect("invalid content range header"),
                                );
                                header_map.insert(header::VARY, vary);
                                ctx.http.req = Some(request);
                                ctx.http.res =
                                    Some(HttpResponse::BuiltinError(416, Some(header_map)));
                                emit_static_response_metric(ctx, 416, "range_not_satisfiable");
                                ctx.get_span_attributes().insert(
                                    "http.response.status_code",
                                    TraceAttributeValue::I64(416),
                                );
                                return Ok(false);
                            }

                            if ranges.len() > 1 {
                                let multipart_boundary = hex::encode(rand::random::<[u8; 12]>());
                                let vary = vary_header
                                    .unwrap_or_else(|| HeaderValue::from_static("Range"));

                                let mut builder = Response::builder()
                                    .status(StatusCode::PARTIAL_CONTENT)
                                    .header(
                                        header::CONTENT_TYPE,
                                        format!(
                                            "multipart/byteranges; boundary={multipart_boundary}"
                                        ),
                                    );

                                if let Some(ref mdate) = mdate {
                                    builder = builder.header(
                                        header::LAST_MODIFIED,
                                        httpdate::fmt_http_date(*mdate),
                                    );
                                }
                                if let Some(ref etag) = etag_value {
                                    builder = builder.header(header::ETAG, format!("W/\"{etag}\""));
                                }
                                if let Some(cc) = cache_control.as_deref() {
                                    builder = builder.header(
                                        header::CACHE_CONTROL,
                                        HeaderValue::from_str(cc)
                                            .unwrap_or_else(|_| HeaderValue::from_static("")),
                                    );
                                }
                                builder = builder.header(header::VARY, vary);

                                if method == Method::HEAD {
                                    let response = builder
                                        .body(
                                            Empty::new().map_err(|_| unreachable!()).boxed_unsync(),
                                        )
                                        .expect("failed to build 206 HEAD response");
                                    ctx.http.req = Some(request);
                                    ctx.http.res = Some(HttpResponse::Custom(response));
                                    emit_static_response_metric(ctx, 206, "partial_content");
                                    ctx.get_span_attributes().insert(
                                        "http.response.status_code",
                                        TraceAttributeValue::I64(206),
                                    );
                                } else {
                                    let response = builder
                                        .body(
                                            MultipartByterangeBody::new(
                                                multipart_boundary,
                                                file_length,
                                                content_type,
                                                ranges,
                                                FileStream::new(file, 0, Some(file_length)),
                                            )
                                            .boxed_unsync(),
                                        )
                                        .expect("failed to build 206 response");
                                    ctx.http.req = Some(request);
                                    ctx.http.res = Some(HttpResponse::Custom(response));
                                    emit_static_response_metric(ctx, 206, "partial_content");
                                    ctx.get_span_attributes().insert(
                                        "http.response.status_code",
                                        TraceAttributeValue::I64(206),
                                    );
                                }
                                return Ok(false);
                            } else if let Some((start, end)) = ranges.first().map(|(s, e)| (*s, *e))
                            {
                                let end = end.min(file_length - 1);
                                let content_len = end - start + 1;
                                let vary = vary_header
                                    .unwrap_or_else(|| HeaderValue::from_static("Range"));

                                let mut builder = Response::builder()
                                    .status(StatusCode::PARTIAL_CONTENT)
                                    .header(header::CONTENT_LENGTH, content_len)
                                    .header(
                                        header::CONTENT_RANGE,
                                        format!("bytes {start}-{end}/{file_length}"),
                                    );

                                if let Some(ref mdate) = mdate {
                                    builder = builder.header(
                                        header::LAST_MODIFIED,
                                        httpdate::fmt_http_date(*mdate),
                                    );
                                }
                                if let Some(ref etag) = etag_value {
                                    builder = builder.header(header::ETAG, format!("W/\"{etag}\""));
                                }
                                if let Some(ref ct) = content_type {
                                    builder = builder.header(header::CONTENT_TYPE, ct);
                                }
                                if let Some(cc) = cache_control.as_deref() {
                                    builder = builder.header(
                                        header::CACHE_CONTROL,
                                        HeaderValue::from_str(cc)
                                            .unwrap_or_else(|_| HeaderValue::from_static("")),
                                    );
                                }
                                builder = builder.header(header::VARY, vary);

                                if method == Method::HEAD {
                                    let response = builder
                                        .body(
                                            Empty::new().map_err(|_| unreachable!()).boxed_unsync(),
                                        )
                                        .expect("failed to build 206 HEAD response");
                                    ctx.http.req = Some(request);
                                    ctx.http.res = Some(HttpResponse::Custom(response));
                                    emit_static_response_metric(ctx, 206, "partial_content");
                                    ctx.get_span_attributes().insert(
                                        "http.response.status_code",
                                        TraceAttributeValue::I64(206),
                                    );
                                } else {
                                    let response = builder
                                        .body(
                                            StreamBody::new(
                                                FileStream::new(file, start, Some(end + 1))
                                                    .map_ok(Frame::data),
                                            )
                                            .boxed_unsync(),
                                        )
                                        .expect("failed to build 206 response");
                                    ctx.http.req = Some(request);
                                    ctx.http.res = Some(HttpResponse::Custom(response));
                                    emit_static_response_metric(ctx, 206, "partial_content");
                                    ctx.get_span_attributes().insert(
                                        "http.response.status_code",
                                        TraceAttributeValue::I64(206),
                                    );
                                }
                                return Ok(false);
                            } else {
                                let vary = vary_header
                                    .unwrap_or_else(|| HeaderValue::from_static("Range"));
                                let mut header_map = HeaderMap::new();
                                header_map.insert(
                                    header::CONTENT_RANGE,
                                    HeaderValue::from_str(&format!("bytes */{file_length}"))
                                        .expect("invalid content range header"),
                                );
                                header_map.insert(header::VARY, vary);
                                ctx.http.req = Some(request);
                                ctx.http.res =
                                    Some(HttpResponse::BuiltinError(416, Some(header_map)));
                                emit_static_response_metric(ctx, 416, "range_not_satisfiable");
                                ctx.get_span_attributes().insert(
                                    "http.response.status_code",
                                    TraceAttributeValue::I64(416),
                                );
                                return Ok(false);
                            }
                        }
                        Err(RangeParseError::Unsatisfiable) => {
                            let vary =
                                vary_header.unwrap_or_else(|| HeaderValue::from_static("Range"));
                            let mut header_map = HeaderMap::new();
                            header_map.insert(
                                header::CONTENT_RANGE,
                                HeaderValue::from_str(&format!("bytes */{file_length}"))
                                    .expect("invalid content range header"),
                            );
                            header_map.insert(header::VARY, vary);
                            ctx.http.req = Some(request);
                            ctx.http.res = Some(HttpResponse::BuiltinError(416, Some(header_map)));
                            emit_static_response_metric(ctx, 416, "range_not_satisfiable");
                            ctx.get_span_attributes()
                                .insert("http.response.status_code", TraceAttributeValue::I64(416));
                            return Ok(false);
                        }
                        Err(RangeParseError::InvalidSyntax) => {
                            // Syntactically invalid — treat as absent, fall through to 200
                        }
                    }
                }
            }
        }

        let mut builder = Response::builder()
            .status(StatusCode::OK)
            .header(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));

        // Last-Modified
        if let Some(ref mdate) = mdate {
            builder = builder.header(header::LAST_MODIFIED, httpdate::fmt_http_date(*mdate));
        }

        // ETag
        if let Some(ref etag) = etag_value {
            let etag_suffix = used_compression.etag_suffix().unwrap_or("");
            let precompressed_suffix = if is_precompressed_file {
                "-precompress"
            } else {
                ""
            };
            let full_etag = format!("W/\"{etag}{precompressed_suffix}{etag_suffix}\"");
            builder = builder.header(header::ETAG, full_etag);
        }

        // Vary
        if let Some(vary) = vary_header {
            builder = builder.header(header::VARY, vary);
        }

        // Content-Type
        if let Some(ref ct) = content_type {
            builder = builder.header(header::CONTENT_TYPE, ct);
        }

        // Cache-Control
        if let Some(cc) = cache_control.as_deref() {
            builder = builder.header(
                header::CACHE_CONTROL,
                HeaderValue::from_str(cc).unwrap_or_else(|_| HeaderValue::from_static("")),
            );
        }

        // Content-Encoding / Content-Length
        match used_compression {
            Compression::Identity => {
                builder = builder.header(header::CONTENT_LENGTH, file_length);
            }
            c => {
                if is_precompressed_file {
                    builder = builder.header(header::CONTENT_LENGTH, file_length);
                }
                if let Some(hv) = c.header_value() {
                    builder =
                        builder.header(header::CONTENT_ENCODING, HeaderValue::from_static(hv));
                }
            }
        }

        if method == Method::HEAD {
            let response = builder
                .body(Empty::new().map_err(|_| unreachable!()).boxed_unsync())
                .expect("failed to build HEAD response");
            ctx.http.req = Some(request);
            ctx.http.res = Some(HttpResponse::Custom(response));
            emit_static_response_metric(ctx, 200, "head");
            ctx.get_span_attributes()
                .insert("http.response.status_code", TraceAttributeValue::I64(200));
            return Ok(false);
        }

        // Full file response — streaming I/O
        // Use the file handle from context (already opened during path resolution)
        // For precompressed files, the file_path may have changed, so we re-open
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

        // GET body — stream file content
        let body: UnsyncBoxBody<Bytes, io::Error> = if is_precompressed_file {
            // Precompressed file — stream as-is
            StreamBody::new(FileStream::new(file, 0, Some(file_length)).map_ok(Frame::data))
                .boxed_unsync()
        } else {
            match used_compression {
                Compression::Brotli => compress_streaming_brotli(file, Some(file_length)),
                Compression::Zstd => compress_streaming_zstd(file, Some(file_length)),
                Compression::Deflate => compress_streaming_deflate(file, Some(file_length)),
                Compression::Gzip => compress_streaming_gzip(file, Some(file_length)),
                Compression::Identity => {
                    // For identity (no compression), use zerocopy if available
                    StreamBody::new(FileStream::new(file, 0, Some(file_length)).map_ok(Frame::data))
                        .boxed_unsync()
                }
            }
        };

        let mut response = builder.body(body).expect("failed to build file response");

        // Enable zerocopy for uncompressed responses on Linux
        // vibeio-http's zerocopy bypasses the body entirely, using sendfile_exact
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
        emit_static_response_metric(ctx, 200, "full");
        ctx.get_span_attributes()
            .insert("http.response.status_code", TraceAttributeValue::I64(200));

        // Emit static file metrics
        let compression_label = match used_compression {
            Compression::Brotli => "br",
            Compression::Zstd => "zstd",
            Compression::Deflate => "deflate",
            Compression::Gzip => "gzip",
            Compression::Identity => "identity",
        };
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
            ty: MetricType::Histogram(Some(Cow::Borrowed(STATIC_FILE_BYTES_BUCKETS))),
            value: MetricValue::F64(file_size as f64),
            unit: Some("By"),
            description: Some("Bytes sent for static file responses."),
            trace_context: current_event_trace_context(&ctx.http),
        }));

        Ok(false)
    }
}
