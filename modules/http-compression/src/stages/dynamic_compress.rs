//! Dynamic content compression stage

use std::collections::BTreeSet;
use std::sync::LazyLock;

use async_compression::brotli::EncoderParams;
use async_compression::tokio::bufread::{BrotliEncoder, DeflateEncoder, GzipEncoder, ZstdEncoder};
use async_compression::zstd::CParameter;
use async_compression::Level;
use bytes::Bytes;
use ferron_core::pipeline::{PipelineError, Stage};
use ferron_core::StageConstraint as CoreStageConstraint;
use ferron_http::util::parse_q_value_header::parse_q_value_header;
use ferron_http::{HttpContext, HttpResponse};
use futures_util::{StreamExt, TryStreamExt};
use http::header;
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, BodyStream, StreamBody};
use tokio_util::io::{ReaderStream, StreamReader};

const COMPRESSED_STREAM_READER_BUFFER_SIZE: usize = 16384;

/// MIME types that should not be compressed (already compressed formats).
static NON_COMPRESSIBLE_MIME_TYPES: LazyLock<BTreeSet<&'static str>> = LazyLock::new(|| {
    BTreeSet::from_iter(vec![
        "application/bdoc",
        "application/brotli",
        "application/epub+zip",
        "application/gzip",
        "application/java-archive",
        "application/java-serialized-object",
        "application/msword",
        "application/ogg",
        "application/pdf",
        "application/pgp-encrypted",
        "application/ubjson",
        "application/vnd.adobe.air-application-installer-package+zip",
        "application/vnd.adobe.flash.movie",
        "application/vnd.android.package-archive",
        "application/vnd.apple.pkpass",
        "application/vnd.google-apps.document",
        "application/vnd.google-apps.presentation",
        "application/vnd.google-apps.spreadsheet",
        "application/vnd.google-earth.kmz",
        "application/vnd.ms-excel",
        "application/vnd.ms-outlook",
        "application/vnd.ms-powerpoint",
        "application/vnd.ms-xpsdocument",
        "application/vnd.oasis.opendocument.graphics",
        "application/vnd.oasis.opendocument.presentation",
        "application/vnd.oasis.opendocument.spreadsheet",
        "application/vnd.oasis.opendocument.text",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "application/x-7z-compressed",
        "application/x-arj",
        "application/x-bzip",
        "application/x-bzip2",
        "application/x-dvi",
        "application/x-java",
        "application/x-java-jnlp-file",
        "application/x-latex",
        "application/x-pkcs12",
        "application/x-stuffit",
        "application/x-virtualbox-vbox-extpack",
        "application/x-xpinstall",
        "application/zip",
        "application/zstd",
        "audio/basic",
        "audio/mp4",
        "audio/mpeg",
        "audio/ogg",
        "audio/wav",
        "audio/webm",
        "audio/x-caf",
        "image/apng",
        "image/avif",
        "image/gif",
        "image/jp2",
        "image/jpeg",
        "image/jpm",
        "image/jpx",
        "image/png",
        "image/tiff",
        "model/iges",
        "model/mesh",
        "model/vnd.usdz+zip",
        "model/vrml",
        "model/x3d+binary",
        "model/x3d+vrml",
        "video/jpm",
        "video/mp4",
        "video/mpeg",
        "video/ogg",
        "video/quicktime",
        "video/webm",
        "video/x-flv",
        "video/x-matroska",
        "video/x-ms-wmv",
    ])
});

/// Compression algorithm selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Compression {
    Gzip,
    Brotli,
    Deflate,
    Zstd,
    Identity,
}

impl Compression {
    fn header_value(self) -> Option<&'static str> {
        match self {
            Compression::Gzip => Some("gzip"),
            Compression::Brotli => Some("br"),
            Compression::Deflate => Some("deflate"),
            Compression::Zstd => Some("zstd"),
            Compression::Identity => None,
        }
    }

    fn etag_suffix(self) -> Option<&'static str> {
        match self {
            Compression::Gzip => Some("-dynamic-gzip"),
            Compression::Brotli => Some("-dynamic-br"),
            Compression::Deflate => Some("-dynamic-deflate"),
            Compression::Zstd => Some("-dynamic-zstd"),
            Compression::Identity => None,
        }
    }
}

/// State captured during `run()` and used in `run_inverse()`.
pub(crate) struct CapturedState {
    pub(crate) accept_encoding: Option<String>,
    pub(crate) user_agent: Option<String>,
}

struct CapturedStateKey;
impl typemap_rev::TypeMapKey for CapturedStateKey {
    type Value = CapturedState;
}

/// Pipeline stage for dynamic response body compression.
///
/// This stage compresses response bodies on-the-fly based on the client's
/// `Accept-Encoding` header. It runs after response-generating stages
/// (static_file, reverse_proxy) and before header application stages.
#[derive(Default)]
pub struct DynamicCompressionStage {
    _private: (),
}

impl DynamicCompressionStage {
    pub fn new() -> Self {
        Self { _private: () }
    }
}

#[async_trait::async_trait(?Send)]
impl Stage<HttpContext> for DynamicCompressionStage {
    fn name(&self) -> &str {
        "dynamic_compression"
    }

    fn constraints(&self) -> Vec<CoreStageConstraint> {
        vec![
            CoreStageConstraint::Before("static_file".to_string()),
            CoreStageConstraint::Before("reverse_proxy".to_string()),
            CoreStageConstraint::After("headers".to_string()),
            CoreStageConstraint::After("acme_http01".to_string()),
            CoreStageConstraint::After("rewrite".to_string()),
        ]
    }

    fn is_applicable(
        &self,
        config: Option<&ferron_core::config::ServerConfigurationBlock>,
    ) -> bool {
        config.is_some_and(|c| c.has_directive("dynamic_compressed"))
    }

    #[inline]
    async fn run(&self, ctx: &mut HttpContext) -> Result<bool, PipelineError> {
        let accept_encoding = ctx
            .req
            .as_ref()
            .and_then(|r| r.headers().get(header::ACCEPT_ENCODING))
            .and_then(|v| v.to_str().ok())
            .map(String::from);

        let user_agent = ctx
            .req
            .as_ref()
            .and_then(|r| r.headers().get(header::USER_AGENT))
            .and_then(|v| v.to_str().ok())
            .map(String::from);

        let compression_enabled = ctx.configuration.get_flag("dynamic_compressed", true);

        if compression_enabled {
            ctx.extensions.insert::<CapturedStateKey>(CapturedState {
                accept_encoding,
                user_agent,
            });
        }

        Ok(true)
    }

    #[inline]
    async fn run_inverse(&self, ctx: &mut HttpContext) -> Result<(), PipelineError> {
        let state = match ctx.extensions.remove::<CapturedStateKey>() {
            Some(s) => s,
            None => return Ok(()),
        };

        // Extract the response, compress, and put it back
        let response = match ctx.res {
            Some(HttpResponse::Custom(_)) => {
                // Take the response out temporarily
                let response = ctx.res.take().unwrap();
                match response {
                    HttpResponse::Custom(resp) => resp,
                    _ => unreachable!(),
                }
            }
            _ => return Ok(()),
        };

        // Check if response is already encoded
        if response.headers().contains_key(header::CONTENT_ENCODING) {
            ctx.res = Some(HttpResponse::Custom(response));
            return Ok(());
        }

        // Get content type for compressibility check
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.split_once(';').map_or(v, |s| s.0).trim());

        // Check if the MIME type is compressible
        let is_compressible = content_type.is_none_or(|t| !NON_COMPRESSIBLE_MIME_TYPES.contains(t));

        if !is_compressible {
            ctx.res = Some(HttpResponse::Custom(response));
            return Ok(());
        }

        // Check for browsers with known compression bugs
        let user_agent = state.user_agent.as_deref().unwrap_or("");
        if has_broken_compression(user_agent, content_type) {
            ctx.res = Some(HttpResponse::Custom(response));
            return Ok(());
        }

        // Determine the best compression algorithm from Accept-Encoding
        let compression = determine_compression(state.accept_encoding.as_deref());

        if compression == Compression::Identity {
            ctx.res = Some(HttpResponse::Custom(response));
            return Ok(());
        }

        let (mut parts, body) = response.into_parts();

        // Prevent zerocopy from interfering with the module
        parts.extensions.clear();

        // Update Vary header
        update_vary_header(&mut parts.headers);

        // Remove Content-Length (compressed size differs from original)
        parts.headers.remove(header::CONTENT_LENGTH);

        // Update ETag with compression suffix
        if let Some(etag_suffix) = compression.etag_suffix() {
            if let Some(etag) = parts.headers.get_mut(header::ETAG) {
                if let Ok(etag_str) = etag.to_str() {
                    let new_etag = format!("{etag_str}{etag_suffix}");
                    if let Ok(val) = http::HeaderValue::from_str(&new_etag) {
                        *etag = val;
                    }
                }
            }
        }

        // Add Content-Encoding header
        if let Some(algo) = compression.header_value() {
            if let Ok(val) = http::HeaderValue::from_str(algo) {
                parts.headers.insert(header::CONTENT_ENCODING, val);
            }
        }

        // Compress the body
        let compressed_body = compress_body(body, compression);
        let new_response = http::Response::from_parts(parts, compressed_body);

        ctx.res = Some(HttpResponse::Custom(new_response));

        Ok(())
    }
}

/// Check if the client has known broken HTTP compression handling.
fn has_broken_compression(user_agent: &str, content_type: Option<&str>) -> bool {
    // Netscape 4.x (non-IE) has broken compression for certain content types
    if let Some(stripped) = user_agent.strip_prefix("Mozilla/4.") {
        // Internet Explorer masquerading as Netscape 4.x is fine
        if user_agent.contains(" MSIE ") {
            return false;
        }
        // Check for specific broken versions
        let first_char = stripped.chars().next();
        if matches!(first_char, Some('0')) {
            let second_char = stripped.chars().nth(1);
            let is_broken = matches!(second_char, Some('6') | Some('7') | Some('8'));
            if is_broken {
                return true;
            }
        }
        // HTML compression is broken for all Netscape 4.x
        if content_type == Some("text/html") {
            return true;
        }
    }

    // w3m browser has broken HTML compression
    if user_agent.starts_with("w3m/") && content_type == Some("text/html") {
        return true;
    }

    false
}

/// Determine the best compression algorithm based on Accept-Encoding header.
fn determine_compression(accept_encoding: Option<&str>) -> Compression {
    let Some(accept_encoding) = accept_encoding else {
        return Compression::Identity;
    };

    for enc in parse_q_value_header(accept_encoding) {
        match enc.as_str() {
            "br" => return Compression::Brotli,
            "zstd" => return Compression::Zstd,
            "gzip" => return Compression::Gzip,
            "deflate" => return Compression::Deflate,
            "identity" => return Compression::Identity,
            _ => continue,
        }
    }

    Compression::Identity
}

/// Add Accept-Encoding to the Vary header if not already present.
fn update_vary_header(headers: &mut http::HeaderMap) {
    if let Some(vary) = headers.get_mut(header::VARY) {
        if let Ok(vary_str) = vary.to_str() {
            let has_accept_encoding = vary_str
                .split(',')
                .any(|v| v.trim().eq_ignore_ascii_case("accept-encoding"));
            if !has_accept_encoding {
                let new_vary = format!("{vary_str}, Accept-Encoding");
                if let Ok(val) = http::HeaderValue::from_str(&new_vary) {
                    *vary = val;
                }
            }
        }
    } else {
        headers.insert(
            header::VARY,
            http::HeaderValue::from_static("Accept-Encoding"),
        );
    }
}

/// Compress a response body using the specified algorithm.
fn compress_body(
    body: UnsyncBoxBody<Bytes, std::io::Error>,
    compression: Compression,
) -> UnsyncBoxBody<Bytes, std::io::Error> {
    match compression {
        Compression::Brotli => compress_brotli(body),
        Compression::Zstd => compress_zstd(body),
        Compression::Deflate => compress_deflate(body),
        Compression::Gzip => compress_gzip(body),
        Compression::Identity => body,
    }
}

fn compress_brotli(
    body: UnsyncBoxBody<Bytes, std::io::Error>,
) -> UnsyncBoxBody<Bytes, std::io::Error> {
    let stream = BodyStream::new(body);
    let data_stream = TryStreamExt::map_ok(stream, |frame: http_body::Frame<Bytes>| {
        frame.into_data().unwrap_or_default()
    });
    let reader = StreamReader::new(data_stream);
    let encoder = BrotliEncoder::with_params(
        reader,
        EncoderParams::default()
            .quality(Level::Precise(4))
            .window_size(17)
            .block_size(18),
    );
    let reader_stream = ReaderStream::with_capacity(encoder, COMPRESSED_STREAM_READER_BUFFER_SIZE);
    StreamBody::new(reader_stream.map(|result| result.map(http_body::Frame::data))).boxed_unsync()
}

fn compress_zstd(
    body: UnsyncBoxBody<Bytes, std::io::Error>,
) -> UnsyncBoxBody<Bytes, std::io::Error> {
    let stream = BodyStream::new(body);
    let data_stream = TryStreamExt::map_ok(stream, |frame: http_body::Frame<Bytes>| {
        frame.into_data().unwrap_or_default()
    });
    let reader = StreamReader::new(data_stream);
    let encoder = ZstdEncoder::with_quality_and_params(
        reader,
        Level::Default,
        &[CParameter::window_log(17), CParameter::hash_log(10)],
    );
    let reader_stream = ReaderStream::with_capacity(encoder, COMPRESSED_STREAM_READER_BUFFER_SIZE);
    StreamBody::new(reader_stream.map(|result| result.map(http_body::Frame::data))).boxed_unsync()
}

fn compress_deflate(
    body: UnsyncBoxBody<Bytes, std::io::Error>,
) -> UnsyncBoxBody<Bytes, std::io::Error> {
    let stream = BodyStream::new(body);
    let data_stream = TryStreamExt::map_ok(stream, |frame: http_body::Frame<Bytes>| {
        frame.into_data().unwrap_or_default()
    });
    let reader = StreamReader::new(data_stream);
    let encoder = DeflateEncoder::with_quality(reader, Level::Precise(4));
    let reader_stream = ReaderStream::with_capacity(encoder, COMPRESSED_STREAM_READER_BUFFER_SIZE);
    StreamBody::new(reader_stream.map(|result| result.map(http_body::Frame::data))).boxed_unsync()
}

fn compress_gzip(
    body: UnsyncBoxBody<Bytes, std::io::Error>,
) -> UnsyncBoxBody<Bytes, std::io::Error> {
    let stream = BodyStream::new(body);
    let data_stream = TryStreamExt::map_ok(stream, |frame: http_body::Frame<Bytes>| {
        frame.into_data().unwrap_or_default()
    });
    let reader = StreamReader::new(data_stream);
    let encoder = GzipEncoder::with_quality(reader, Level::Precise(4));
    let reader_stream = ReaderStream::with_capacity(encoder, COMPRESSED_STREAM_READER_BUFFER_SIZE);
    StreamBody::new(reader_stream.map(|result| result.map(http_body::Frame::data))).boxed_unsync()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_determine_compression_brotli_preferred() {
        assert_eq!(
            determine_compression(Some("br; q=1.0, gzip; q=0.8, deflate; q=0.6")),
            Compression::Brotli
        );
    }

    #[test]
    fn test_determine_compression_zstd() {
        assert_eq!(
            determine_compression(Some("zstd; q=1.0, gzip; q=0.8")),
            Compression::Zstd
        );
    }

    #[test]
    fn test_determine_compression_gzip_fallback() {
        assert_eq!(
            determine_compression(Some("gzip; q=0.8, identity; q=0.5")),
            Compression::Gzip
        );
    }

    #[test]
    fn test_determine_compression_no_accept_header() {
        assert_eq!(determine_compression(None), Compression::Identity);
    }

    #[test]
    fn test_has_broken_compression_netscape4_html() {
        assert!(has_broken_compression("Mozilla/4.08", Some("text/html")));
    }

    #[test]
    fn test_has_broken_compression_ie_masquerading() {
        assert!(!has_broken_compression(
            "Mozilla/4.0 (compatible; MSIE 6.0)",
            Some("text/html")
        ));
    }

    #[test]
    fn test_has_broken_compression_w3m_html() {
        assert!(has_broken_compression("w3m/0.5.3", Some("text/html")));
    }

    #[test]
    fn test_has_broken_compression_modern_browser() {
        assert!(!has_broken_compression(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64)",
            Some("text/html")
        ));
    }

    #[test]
    fn test_non_compressible_mime_types_contains_common_formats() {
        assert!(NON_COMPRESSIBLE_MIME_TYPES.contains("image/jpeg"));
        assert!(NON_COMPRESSIBLE_MIME_TYPES.contains("image/png"));
        assert!(NON_COMPRESSIBLE_MIME_TYPES.contains("application/pdf"));
        assert!(NON_COMPRESSIBLE_MIME_TYPES.contains("application/zip"));
        assert!(NON_COMPRESSIBLE_MIME_TYPES.contains("video/mp4"));
    }

    #[test]
    fn test_compression_header_values() {
        assert_eq!(Compression::Gzip.header_value(), Some("gzip"));
        assert_eq!(Compression::Brotli.header_value(), Some("br"));
        assert_eq!(Compression::Deflate.header_value(), Some("deflate"));
        assert_eq!(Compression::Zstd.header_value(), Some("zstd"));
        assert_eq!(Compression::Identity.header_value(), None);
    }

    #[test]
    fn test_compression_etag_suffix() {
        assert_eq!(Compression::Gzip.etag_suffix(), Some("-dynamic-gzip"));
        assert_eq!(Compression::Brotli.etag_suffix(), Some("-dynamic-br"));
        assert_eq!(Compression::Deflate.etag_suffix(), Some("-dynamic-deflate"));
        assert_eq!(Compression::Zstd.etag_suffix(), Some("-dynamic-zstd"));
        assert_eq!(Compression::Identity.etag_suffix(), None);
    }
}
