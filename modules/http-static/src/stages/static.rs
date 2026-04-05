//! Static file serving stage with streaming I/O and optional zerocopy.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::io;
use std::sync::LazyLock;

use async_trait::async_trait;
use bytes::Bytes;
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::StageConstraint;
use ferron_http::util::parse_q_value_header::parse_q_value_header;
use ferron_http::{HttpFileContext, HttpResponse};
use futures_util::TryStreamExt;
use http::header::{self, HeaderValue};
use http::{Method, Response, StatusCode};
use http_body::Frame;
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, Empty, StreamBody};

const COMPRESSED_STREAM_READER_BUFFER_SIZE: usize = 16384;
const MAX_BUFFER_SIZE: usize = 16384;

use send_wrapper::SendWrapper;

/// Compression algorithm
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Compression {
    Gzip,
    Brotli,
    Deflate,
    Zstd,
    Identity,
}

/// Hard-coded list of non-compressible file extensions
static NON_COMPRESSIBLE_FILE_EXTENSIONS: LazyLock<BTreeSet<&'static str>> = LazyLock::new(|| {
    BTreeSet::from_iter([
        "7z",
        "air",
        "amlx",
        "apk",
        "apng",
        "appinstaller",
        "appx",
        "appxbundle",
        "arj",
        "au",
        "avif",
        "bdoc",
        "boz",
        "br",
        "bz",
        "bz2",
        "caf",
        "class",
        "doc",
        "docx",
        "dot",
        "dvi",
        "ear",
        "epub",
        "flv",
        "gdoc",
        "gif",
        "gsheet",
        "gslides",
        "gz",
        "iges",
        "igs",
        "jar",
        "jnlp",
        "jp2",
        "jpe",
        "jpeg",
        "jpf",
        "jpg",
        "jpg2",
        "jpgm",
        "jpm",
        "jpx",
        "kmz",
        "latex",
        "m1v",
        "m2a",
        "m2v",
        "m3a",
        "m4a",
        "mesh",
        "mk3d",
        "mks",
        "mkv",
        "mov",
        "mp2",
        "mp2a",
        "mp3",
        "mp4",
        "mp4a",
        "mp4v",
        "mpe",
        "mpeg",
        "mpg",
        "mpg4",
        "mpga",
        "msg",
        "msh",
        "msix",
        "msixbundle",
        "odg",
        "odp",
        "ods",
        "odt",
        "oga",
        "ogg",
        "ogv",
        "ogx",
        "opus",
        "p12",
        "pdf",
        "pfx",
        "pgp",
        "pkpass",
        "png",
        "pot",
        "pps",
        "ppt",
        "pptx",
        "qt",
        "ser",
        "silo",
        "sit",
        "snd",
        "spx",
        "stpxz",
        "stpz",
        "swf",
        "tif",
        "tiff",
        "ubj",
        "usdz",
        "vbox-extpack",
        "vrml",
        "war",
        "wav",
        "weba",
        "webm",
        "wmv",
        "wrl",
        "x3dbz",
        "x3dvz",
        "xla",
        "xlc",
        "xlm",
        "xls",
        "xlsx",
        "xlt",
        "xlw",
        "xpi",
        "xps",
        "zip",
        "zst",
    ])
});

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
        vec![
            StageConstraint::After("directory_index".to_string()),
            StageConstraint::After("directory_listing".to_string()),
        ]
    }

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
                .header(header::ALLOW, "GET, HEAD, OPTIONS")
                .body(Empty::new().map_err(|_| unreachable!()).boxed_unsync())
                .expect("failed to build OPTIONS response");
            ctx.http.req = Some(request);
            ctx.http.res = Some(HttpResponse::Custom(res));
            return Ok(false);
        }

        // Only handle GET and HEAD
        if method != Method::GET && method != Method::HEAD {
            let mut allow_headers = http::HeaderMap::new();
            allow_headers.insert(
                header::ALLOW,
                HeaderValue::from_static("GET, HEAD, OPTIONS"),
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
            let etag_cache_key = format!(
                "{}-{}-{}",
                ctx.file_path.to_string_lossy(),
                ctx.metadata.len(),
                ctx.metadata
                    .modified()
                    .ok()
                    .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
            );
            let etag = format!(
                "{:016x}",
                xxhash_rust::xxh3::xxh3_64(etag_cache_key.as_bytes())
            );
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
                        let res = build_etag_response(
                            StatusCode::PRECONDITION_FAILED,
                            &etag,
                            &vary_header,
                            None,
                            cache_control,
                        );
                        ctx.http.req = Some(request);
                        ctx.http.res = Some(HttpResponse::Custom(res));
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
            // We let the request through since we can't satisfy strong ETag matching.
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
        Ok(false)
    }
}

// --- FileStream: wraps vibeio::fs::File as a futures::Stream without blocking ---

/// A wrapper over `vibeio::fs::File` that implements `futures_core::Stream`.
/// Uses `read_at` for position-based async reads without spawning blocking threads.
/// `SendWrapper` ensures the non-`Send` `vibeio::fs::File` can safely cross thread boundaries
/// as long as it's only polled on the same thread (guaranteed by the single-threaded runtime).
struct FileStream {
    file: std::sync::Arc<SendWrapper<vibeio::fs::File>>,
    current_pos: u64,
    end: Option<u64>,
    finished: bool,
    #[allow(clippy::type_complexity)]
    read_future: Option<
        std::pin::Pin<
            Box<dyn std::future::Future<Output = Option<Result<Bytes, io::Error>>> + Send + Sync>,
        >,
    >,
}

impl FileStream {
    fn new(file: vibeio::fs::File, start: u64, end: Option<u64>) -> Self {
        Self {
            file: std::sync::Arc::new(SendWrapper::new(file)),
            current_pos: start,
            end,
            finished: false,
            read_future: None,
        }
    }
}

impl futures_core::Stream for FileStream {
    type Item = Result<Bytes, io::Error>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        if self.finished {
            return std::task::Poll::Ready(None);
        }

        if self.read_future.is_none() {
            self.read_future = Some(Box::pin(SendWrapper::new(read_chunk(
                self.file.clone(),
                self.current_pos,
                self.end,
            ))));
        }

        match self
            .read_future
            .as_mut()
            .expect("file stream read future is not initialized")
            .as_mut()
            .poll(cx)
        {
            std::task::Poll::Ready(Some(Ok(chunk))) => {
                let _ = self.read_future.take();
                self.current_pos += chunk.len() as u64;
                if let Some(end) = &self.end {
                    if self.current_pos >= *end {
                        self.finished = true;
                    }
                }
                std::task::Poll::Ready(Some(Ok(chunk)))
            }
            std::task::Poll::Ready(option) => {
                let _ = self.read_future.take();
                if option.is_none() {
                    self.finished = true;
                }
                std::task::Poll::Ready(option)
            }
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}

/// Reads a single chunk from a vibeio file at the given position.
async fn read_chunk(
    file: std::sync::Arc<SendWrapper<vibeio::fs::File>>,
    pos: u64,
    end: Option<u64>,
) -> Option<Result<Bytes, io::Error>> {
    let buffer_sz = end.map_or(MAX_BUFFER_SIZE, |n| {
        ((n - pos) as usize).min(MAX_BUFFER_SIZE)
    });
    if buffer_sz == 0 {
        return None;
    }
    let buffer_uninit = Box::new_uninit_slice(buffer_sz);
    // Safety: The buffer is a boxed slice of uninitialized `u8` values. `u8` is a primitive type.
    let buffer: Box<[u8]> = unsafe { buffer_uninit.assume_init() };
    let result = file.read_at(buffer, pos).await;
    match result {
        (Ok(n), buffer) => {
            if n == 0 {
                None
            } else {
                let mut bytes = Bytes::from_owner(buffer);
                bytes.truncate(n);
                Some(Ok(bytes))
            }
        }
        (Err(e), _) => Some(Err(e)),
    }
}

// --- Streaming compression helpers ---

fn compress_streaming_gzip(file: vibeio::fs::File) -> UnsyncBoxBody<Bytes, io::Error> {
    use async_compression::tokio::bufread::GzipEncoder;
    use tokio_util::io::{ReaderStream, StreamReader};
    let reader = StreamReader::new(FileStream::new(file, 0, None));
    let encoder = GzipEncoder::new(reader);
    StreamBody::new(
        ReaderStream::with_capacity(encoder, COMPRESSED_STREAM_READER_BUFFER_SIZE)
            .map_ok(Frame::data)
            .map_err(|e| e),
    )
    .boxed_unsync()
}

fn compress_streaming_brotli(file: vibeio::fs::File) -> UnsyncBoxBody<Bytes, io::Error> {
    use async_compression::brotli::EncoderParams;
    use async_compression::tokio::bufread::BrotliEncoder;
    use async_compression::Level;
    use tokio_util::io::{ReaderStream, StreamReader};
    let reader = StreamReader::new(FileStream::new(file, 0, None));
    let encoder = BrotliEncoder::with_params(
        reader,
        EncoderParams::default()
            .quality(Level::Precise(4))
            .window_size(17)
            .block_size(18),
    );
    StreamBody::new(
        ReaderStream::with_capacity(encoder, COMPRESSED_STREAM_READER_BUFFER_SIZE)
            .map_ok(Frame::data)
            .map_err(|e| e),
    )
    .boxed_unsync()
}

fn compress_streaming_zstd(file: vibeio::fs::File) -> UnsyncBoxBody<Bytes, io::Error> {
    use async_compression::tokio::bufread::ZstdEncoder;
    use async_compression::zstd::CParameter;
    use async_compression::Level;
    use tokio_util::io::{ReaderStream, StreamReader};
    let reader = StreamReader::new(FileStream::new(file, 0, None));
    let encoder = ZstdEncoder::with_quality_and_params(
        reader,
        Level::Default,
        &[CParameter::window_log(17), CParameter::hash_log(10)],
    );
    StreamBody::new(
        ReaderStream::with_capacity(encoder, COMPRESSED_STREAM_READER_BUFFER_SIZE)
            .map_ok(Frame::data)
            .map_err(|e| e),
    )
    .boxed_unsync()
}

fn compress_streaming_deflate(file: vibeio::fs::File) -> UnsyncBoxBody<Bytes, io::Error> {
    use async_compression::tokio::bufread::DeflateEncoder;
    use tokio_util::io::{ReaderStream, StreamReader};
    let reader = StreamReader::new(FileStream::new(file, 0, None));
    let encoder = DeflateEncoder::new(reader);
    StreamBody::new(
        ReaderStream::with_capacity(encoder, COMPRESSED_STREAM_READER_BUFFER_SIZE)
            .map_ok(Frame::data)
            .map_err(|e| e),
    )
    .boxed_unsync()
}

/// Get content type for a file path, respecting custom MIME type overrides.
fn get_content_type(
    path: &std::path::Path,
    config: &ferron_core::config::layer::LayeredConfiguration,
) -> Option<String> {
    // Check custom MIME types from config
    for entry in config.get_entries("mime_type", true) {
        if entry.args.len() >= 2 {
            if let (Some(key), Some(val)) = (entry.args[0].as_str(), entry.args[1].as_str()) {
                let ext_match = path
                    .extension()
                    .map(|e| e.to_string_lossy())
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                if key == ext_match || key == format!(".{ext_match}") {
                    return Some(val.to_string());
                }
            }
        }
    }

    // Fall back to new_mime_guess
    new_mime_guess::from_path(path)
        .first()
        .map(|mime| mime.to_string())
}

/// Build a response with ETag and Vary headers.
fn build_etag_response(
    status: StatusCode,
    etag: &str,
    vary: &Option<String>,
    content_type: Option<&str>,
    cache_control: Option<&str>,
) -> Response<UnsyncBoxBody<Bytes, io::Error>> {
    let mut builder = Response::builder()
        .status(status)
        .header(header::ETAG, construct_etag(etag, None, true));
    if let Some(v) = vary {
        builder = builder.header(
            header::VARY,
            HeaderValue::from_str(v).expect("invalid vary header"),
        );
    }
    if let Some(ct) = content_type {
        builder = builder.header(header::CONTENT_TYPE, ct);
    }
    if let Some(cc) = cache_control {
        builder = builder.header(
            header::CACHE_CONTROL,
            HeaderValue::from_str(cc).expect("invalid cache-control header"),
        );
    }
    builder
        .body(Empty::new().map_err(|_| unreachable!()).boxed_unsync())
        .expect("failed to build response")
}

/// Split an ETag request header into individual ETags.
fn split_etag_request(etag: &str) -> Vec<String> {
    let mut is_quote = false;
    let mut result = Vec::new();
    let mut current = String::new();
    let mut chars = etag.chars();

    while let Some(c) = chars.next() {
        if c == '"' {
            is_quote = !is_quote;
        } else if c == ',' && !is_quote {
            let trimmed = current.trim().to_owned();
            if !trimmed.is_empty() {
                result.push(trimmed);
            }
            current.clear();
        } else if c == '\\' && is_quote {
            if let Some(next) = chars.next() {
                current.push(next);
            }
        } else {
            current.push(c);
        }
    }
    let trimmed = current.trim().to_owned();
    if !trimmed.is_empty() {
        result.push(trimmed);
    }
    result
}

/// Extract ETag inner value, optionally handling weak ETags.
fn extract_etag_inner(input: &str, weak: bool) -> Option<(String, Option<String>, bool)> {
    let (is_weak, trimmed) = if weak {
        match input.strip_prefix("W/") {
            Some(s) => (true, s),
            None => (false, input),
        }
    } else {
        (false, input)
    };
    let trimmed = trimmed.trim_matches('"');
    let mut parts = trimmed.splitn(2, '-');
    parts
        .next()
        .map(|etag| (etag.to_string(), parts.next().map(String::from), is_weak))
}

/// Construct an ETag string.
fn construct_etag(etag: &str, suffix: Option<&str>, weak: bool) -> String {
    let inner = match suffix {
        Some(s) => format!("{etag}-{s}"),
        None => etag.to_string(),
    };
    if weak {
        format!("W/\"{inner}\"")
    } else {
        format!("\"{inner}\"")
    }
}

/// Parse the HTTP Range header value.
fn parse_range_header(range_str: &str, default_end: u64) -> Option<(u64, u64)> {
    let range_part = range_str.strip_prefix("bytes=")?;
    let parts: Vec<&str> = range_part.split('-').take(2).collect();
    if parts.len() != 2 {
        return None;
    }
    if parts[0].is_empty() {
        // Suffix range: -N (last N bytes)
        let n = parts[1].parse::<u64>().ok()?;
        if n == 0 {
            return None;
        }
        let file_len = default_end + 1;
        if n >= file_len {
            return Some((0, default_end));
        }
        Some((file_len - n, default_end))
    } else if parts[1].is_empty() {
        // Open-ended: N-
        let start = parts[0].parse::<u64>().ok()?;
        Some((start, default_end))
    } else {
        // Explicit range: N-M
        let start = parts[0].parse::<u64>().ok()?;
        let end = parts[1].parse::<u64>().ok()?;
        Some((start, end))
    }
}
