//! Static file serving stage with streaming I/O and optional zerocopy.

use std::ffi::OsStr;
use std::io;

use async_trait::async_trait;
use bytes::Bytes;
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::StageConstraint;
use ferron_http::util::parse_q_value_header::parse_q_value_header;
use ferron_http::{HttpFileContext, HttpResponse};
use ferron_observability::{Event, MetricAttributeValue, MetricEvent, MetricType, MetricValue};
use futures_util::TryStreamExt;
use http::header::{self, HeaderValue};
use http::{Method, Response, StatusCode};
use http_body::Frame;
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, Empty, StreamBody};

use crate::util::compression::{
    compress_streaming_brotli, compress_streaming_deflate, compress_streaming_gzip,
    compress_streaming_zstd, Compression, NON_COMPRESSIBLE_FILE_EXTENSIONS,
};
use crate::util::etag::{
    build_etag_header_map, construct_etag, extract_etag_inner, split_etag_request,
};
use crate::util::file_stream::FileStream;
use crate::util::mime::get_content_type;
use crate::util::range::parse_range_header;

pub struct StaticFileStage;

impl Default for StaticFileStage {
    #[inline]
    fn default() -> Self {
        Self
    }
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
        // Skip if root is not configured
        if ctx.http.configuration.get_value("root", true).is_none() {
            return Ok(true);
        }

        let Some(request) = ctx.http.req.take() else {
            return Ok(true);
        };

        // Only handle files
        if ctx.path_info.is_some() || !ctx.metadata.is_file() {
            ctx.http.req = Some(request);
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
            .and_then(|v| v.as_str());

        // Determine content type
        let content_type = get_content_type(&ctx.file_path, config);

        // Check if compression is possible
        let compression_possible = compressed && {
            let file_len = ctx.metadata.len();
            let ext = ctx
                .file_path
                .extension()
                .map(|e| e.to_string_lossy().to_string())
                .unwrap_or_default();
            file_len > 256 && !NON_COMPRESSIBLE_FILE_EXTENSIONS.contains(ext.as_str())
        };

        // Handle ETags
        let mut etag_value: Option<String> = None;
        #[allow(unused_assignments)]
        let mut vary_header: Option<String> = None;

        if etag_enabled {
            let etag = ctx.etag.clone();
            etag_value = Some(etag.clone());

            vary_header = Some(if compression_possible {
                "Accept-Encoding, If-Match, If-None-Match, Range".to_string()
            } else {
                "If-Match, If-None-Match, Range".to_string()
            });

            // Handle If-None-Match
            if let Some(if_none_match) = request.headers().get(header::IF_NONE_MATCH) {
                if let Ok(val) = if_none_match.to_str() {
                    if method != Method::GET && method != Method::HEAD {
                        let header_map =
                            build_etag_header_map(&etag, &vary_header, None, cache_control);
                        ctx.http.req = Some(request);
                        ctx.http.res = Some(HttpResponse::BuiltinError(412, Some(header_map)));
                        return Ok(false);
                    }
                    for tag in split_etag_request(val) {
                        if let Some((extracted, suffix_opt, _)) = extract_etag_inner(&tag, true) {
                            if extracted == etag {
                                let suffix = suffix_opt.and_then(|s| match s.as_str() {
                                    "gzip" | "deflate" | "br" | "zstd" => Some(s),
                                    _ => None,
                                });
                                let full_etag = construct_etag(&etag, suffix.as_deref(), true);
                                let mut builder = Response::builder()
                                    .status(StatusCode::NOT_MODIFIED)
                                    .header(header::ETAG, &full_etag)
                                    .header(
                                        header::VARY,
                                        HeaderValue::from_str(vary_header.as_deref().unwrap_or(""))
                                            .unwrap_or_else(|_| HeaderValue::from_static("")),
                                    );
                                if let Some(cc) = cache_control {
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
                                return Ok(false);
                            }
                        }
                    }
                }
            }

            // Handle If-Match (Ferron only emits weak ETags, so strong If-Match won't match)
            if let Some(if_match_value) = request.headers().get(header::IF_MATCH) {
                match if_match_value.to_str() {
                    Ok(if_match) => {
                        if !matches!(request.method(), &Method::GET | &Method::HEAD) {
                            // Precondition failed when method is not GET or HEAD
                            let header_map =
                                build_etag_header_map(&etag, &vary_header, None, cache_control);
                            ctx.http.req = Some(request);
                            ctx.http.res = Some(HttpResponse::BuiltinError(412, Some(header_map)));
                            return Ok(false);
                        }

                        // "*" means any version is acceptable
                        // Ferron only emits weak ETags, and comparing a strong ETag with it would not match
                        // for strong comparsions, for more details see RFC 7232
                        if !split_etag_request(if_match)
                            .into_iter()
                            .any(|if_match| if_match == "*")
                        {
                            let header_map =
                                build_etag_header_map(&etag, &vary_header, None, cache_control);
                            ctx.http.req = Some(request);
                            ctx.http.res = Some(HttpResponse::BuiltinError(412, Some(header_map)));
                            return Ok(false);
                        }
                    }
                    Err(_) => {
                        let header_map =
                            build_etag_header_map(&etag, &vary_header, None, cache_control);
                        ctx.http.req = Some(request);
                        ctx.http.res = Some(HttpResponse::BuiltinError(400, Some(header_map)));
                        return Ok(false);
                    }
                }
            }
        } else {
            vary_header = Some(if compression_possible {
                "Accept-Encoding, Range".to_string()
            } else {
                "Range".to_string()
            });
        }

        // Determine compression method
        let mut used_compression = Compression::Identity;
        let mut precompressed_ext: Option<&str> = None;

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
                    for enc in parse_q_value_header(accept_enc) {
                        match enc.as_str() {
                            "br" => {
                                if precompressed {
                                    precompressed_ext = Some("br");
                                } else {
                                    used_compression = Compression::Brotli;
                                }
                                break;
                            }
                            "zstd" => {
                                if precompressed {
                                    precompressed_ext = Some("zst");
                                } else {
                                    used_compression = Compression::Zstd;
                                }
                                break;
                            }
                            "gzip" => {
                                if precompressed {
                                    precompressed_ext = Some("gz");
                                } else {
                                    used_compression = Compression::Gzip;
                                }
                                break;
                            }
                            "deflate" => {
                                if precompressed {
                                    precompressed_ext = Some("deflate");
                                } else {
                                    used_compression = Compression::Deflate;
                                }
                                break;
                            }
                            "identity" => {
                                if precompressed {
                                    precompressed_ext = Some("");
                                }
                                used_compression = Compression::Identity;
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        // Handle precompressed files
        let mut file_path = ctx.file_path.clone();
        let mut file_length = ctx.metadata.len();
        let mut is_precompressed_file = false;

        if precompressed && precompressed_ext.is_some() {
            if let Some(ext) = precompressed_ext {
                if !ext.is_empty() {
                    let mut precomp_path = ctx.file_path.clone();
                    let new_ext = format!(
                        "{}.{}",
                        ctx.file_path
                            .extension()
                            .map(OsStr::to_string_lossy)
                            .unwrap_or_default(),
                        ext
                    );
                    precomp_path.set_extension(&new_ext);

                    if let Ok(meta) = vibeio::fs::metadata(&precomp_path).await {
                        if meta.is_file() {
                            file_path = precomp_path;
                            file_length = meta.len();
                            is_precompressed_file = true;
                            used_compression = match ext {
                                "br" => Compression::Brotli,
                                "zst" => Compression::Zstd,
                                "deflate" => Compression::Deflate,
                                "gz" => Compression::Gzip,
                                _ => Compression::Identity,
                            };
                        }
                    }
                } else {
                    used_compression = Compression::Identity;
                }
            }
        }

        // Handle Range requests
        if let Some(range_val) = request.headers().get(header::RANGE) {
            if let Ok(range_str) = range_val.to_str() {
                if let Some((start, end)) =
                    parse_range_header(range_str, file_length.saturating_sub(1))
                {
                    if file_length == 0 || end >= file_length || start >= file_length || start > end
                    {
                        let vary = vary_header.as_deref().unwrap_or("Range");
                        let res = Response::builder()
                            .status(StatusCode::RANGE_NOT_SATISFIABLE)
                            .header(
                                header::CONTENT_RANGE,
                                HeaderValue::from_str(&format!("bytes */{file_length}")).unwrap(),
                            )
                            .header(
                                header::VARY,
                                HeaderValue::from_str(vary).expect("invalid vary header"),
                            )
                            .body(Empty::new().map_err(|_| unreachable!()).boxed_unsync())
                            .expect("failed to build 416 response");
                        ctx.http.req = Some(request);
                        ctx.http.res = Some(HttpResponse::Custom(res));
                        return Ok(false);
                    }

                    let content_len = end - start + 1;
                    let vary = vary_header.as_deref().unwrap_or("Range");

                    let mut builder = Response::builder()
                        .status(StatusCode::PARTIAL_CONTENT)
                        .header(header::CONTENT_LENGTH, content_len)
                        .header(
                            header::CONTENT_RANGE,
                            format!("bytes {start}-{end}/{file_length}"),
                        );

                    if let Some(ref etag) = etag_value {
                        builder = builder.header(header::ETAG, format!("W/\"{etag}\""));
                    }
                    if let Some(ref ct) = content_type {
                        builder = builder.header(header::CONTENT_TYPE, ct);
                    }
                    if let Some(cc) = cache_control {
                        builder = builder.header(
                            header::CACHE_CONTROL,
                            HeaderValue::from_str(cc).expect("invalid cache-control header"),
                        );
                    }
                    builder = builder.header(
                        header::VARY,
                        HeaderValue::from_str(vary).expect("invalid vary header"),
                    );

                    if method == Method::HEAD {
                        let response = builder
                            .body(Empty::new().map_err(|_| unreachable!()).boxed_unsync())
                            .expect("failed to build 206 HEAD response");
                        ctx.http.req = Some(request);
                        ctx.http.res = Some(HttpResponse::Custom(response));
                    } else {
                        // Stream the range using FileStream with offset/limit
                        let file = vibeio::fs::File::open(&file_path).await.map_err(|e| {
                            PipelineError::custom(format!("failed to open file: {e}"))
                        })?;
                        let response = builder
                            .body(
                                StreamBody::new(
                                    FileStream::new(file, start, Some(end + 1)).map_ok(Frame::data),
                                )
                                .boxed_unsync(),
                            )
                            .expect("failed to build 206 response");
                        ctx.http.req = Some(request);
                        ctx.http.res = Some(HttpResponse::Custom(response));
                    }
                    return Ok(false);
                } else {
                    let vary = vary_header.as_deref().unwrap_or("Range");
                    let res = Response::builder()
                        .status(StatusCode::RANGE_NOT_SATISFIABLE)
                        .header(
                            header::CONTENT_RANGE,
                            HeaderValue::from_str(&format!("bytes */{file_length}")).unwrap(),
                        )
                        .header(
                            header::VARY,
                            HeaderValue::from_str(vary).expect("invalid vary header"),
                        )
                        .body(Empty::new().map_err(|_| unreachable!()).boxed_unsync())
                        .expect("failed to build 416 response");
                    ctx.http.req = Some(request);
                    ctx.http.res = Some(HttpResponse::Custom(res));
                    return Ok(false);
                }
            }
        }

        let mut builder = Response::builder()
            .status(StatusCode::OK)
            .header(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));

        // ETag
        if let Some(ref etag) = etag_value {
            let etag_suffix = match (is_precompressed_file, used_compression) {
                (true, Compression::Brotli) => "-br",
                (true, Compression::Zstd) => "-zstd",
                (true, Compression::Deflate) => "-deflate",
                (true, Compression::Gzip) => "-gzip",
                (_, Compression::Brotli) => "-br",
                (_, Compression::Zstd) => "-zstd",
                (_, Compression::Deflate) => "-deflate",
                (_, Compression::Gzip) => "-gzip",
                _ => "",
            };
            let full_etag = format!("W/\"{etag}{etag_suffix}\"");
            builder = builder.header(header::ETAG, full_etag);
        }

        // Vary
        if let Some(vary) = &vary_header {
            builder = builder.header(
                header::VARY,
                HeaderValue::from_str(vary).expect("invalid vary header value"),
            );
        }

        // Content-Type
        if let Some(ref ct) = content_type {
            builder = builder.header(header::CONTENT_TYPE, ct);
        }

        // Cache-Control
        if let Some(cc) = cache_control {
            builder = builder.header(
                header::CACHE_CONTROL,
                HeaderValue::from_str(cc).expect("invalid cache-control header"),
            );
        }

        // Content-Encoding / Content-Length
        match used_compression {
            Compression::Brotli => {
                builder = builder.header(header::CONTENT_ENCODING, HeaderValue::from_static("br"));
            }
            Compression::Zstd => {
                builder =
                    builder.header(header::CONTENT_ENCODING, HeaderValue::from_static("zstd"));
            }
            Compression::Deflate => {
                builder = builder.header(
                    header::CONTENT_ENCODING,
                    HeaderValue::from_static("deflate"),
                );
            }
            Compression::Gzip => {
                builder =
                    builder.header(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
            }
            Compression::Identity => {
                builder = builder.header(header::CONTENT_LENGTH, file_length);
            }
        }

        if method == Method::HEAD {
            let response = builder
                .body(Empty::new().map_err(|_| unreachable!()).boxed_unsync())
                .expect("failed to build HEAD response");
            ctx.http.req = Some(request);
            ctx.http.res = Some(HttpResponse::Custom(response));
            return Ok(false);
        }

        // Full file response — streaming I/O
        let file = vibeio::fs::File::open(&file_path)
            .await
            .map_err(|e| PipelineError::custom(format!("failed to open file: {e}")))?;

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
                Compression::Brotli => compress_streaming_brotli(file),
                Compression::Zstd => compress_streaming_zstd(file),
                Compression::Deflate => compress_streaming_deflate(file),
                Compression::Gzip => compress_streaming_gzip(file),
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

        // Emit static file metrics
        let compression_label = match used_compression {
            Compression::Brotli => "br",
            Compression::Zstd => "zstd",
            Compression::Deflate => "deflate",
            Compression::Gzip => "gzip",
            Compression::Identity => "identity",
        };
        let cache_hit = is_precompressed_file;
        let file_size = ctx.metadata.len();

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
            ty: MetricType::Histogram(Some(vec![
                1024.0,
                10240.0,
                102400.0,
                1048576.0,
                10485760.0,
                104857600.0,
            ])),
            value: MetricValue::F64(file_size as f64),
            unit: Some("By"),
            description: Some("Bytes sent for static file responses."),
        }));

        Ok(false)
    }
}
