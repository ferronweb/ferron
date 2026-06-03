//! Compression-related utilities for static file serving.

use async_compression::brotli::EncoderParams;
use async_compression::tokio::bufread::{BrotliEncoder, DeflateEncoder, GzipEncoder, ZstdEncoder};
use async_compression::zstd::CParameter;
use async_compression::Level;
use bytes::Bytes;
use futures_util::TryStreamExt;
use http_body::Frame;
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, StreamBody};

use super::file_stream::FileStream;

const COMPRESSED_STREAM_READER_BUFFER_SIZE: usize = 16384;

/// Known HTTP compression ETag suffixes (without leading dash)
pub const COMP_SUFFIXES: &[&str] = &["gzip", "br", "deflate", "zstd"];

/// Compression algorithm
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    Gzip,
    Brotli,
    Deflate,
    Zstd,
    Identity,
}

impl Compression {
    /// Returns the HTTP `Content-Encoding` header value for this compression.
    #[inline]
    pub fn header_value(self) -> Option<&'static str> {
        match self {
            Compression::Gzip => Some("gzip"),
            Compression::Brotli => Some("br"),
            Compression::Deflate => Some("deflate"),
            Compression::Zstd => Some("zstd"),
            Compression::Identity => None,
        }
    }

    /// Returns the compression from a `Content-Encoding` header value.
    #[inline]
    pub fn from_header_value(value: &str) -> Option<Self> {
        match value {
            "gzip" => Some(Compression::Gzip),
            "br" => Some(Compression::Brotli),
            "deflate" => Some(Compression::Deflate),
            "zstd" => Some(Compression::Zstd),
            // "identity" is for explicitly no compression
            "identity" => Some(Compression::Identity),
            _ => None,
        }
    }

    /// Returns the file extension suffix for precompressed variants.
    #[inline]
    pub fn precompressed_ext(self) -> Option<&'static str> {
        match self {
            Compression::Gzip => Some("gz"),
            Compression::Brotli => Some("br"),
            Compression::Deflate => Some("deflate"),
            Compression::Zstd => Some("zst"),
            Compression::Identity => None,
        }
    }

    /// Returns the compression from a precompressed file extension.
    #[inline]
    pub fn from_precompressed_ext(ext: &str) -> Self {
        match ext {
            "gz" => Compression::Gzip,
            "br" => Compression::Brotli,
            "deflate" => Compression::Deflate,
            "zst" => Compression::Zstd,
            _ => Compression::Identity,
        }
    }

    /// Returns the ETag suffix for this compression.
    #[inline]
    pub fn etag_suffix(self) -> Option<&'static str> {
        match self {
            Compression::Gzip => Some("-gzip"),
            Compression::Brotli => Some("-br"),
            Compression::Deflate => Some("-deflate"),
            Compression::Zstd => Some("-zstd"),
            Compression::Identity => None,
        }
    }
}

/// Hard-coded list of non-compressible file extensions
pub static NON_COMPRESSIBLE_FILE_EXTENSIONS: phf::Set<&'static str> = phf::phf_set! {
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
};

macro_rules! compress_streaming {
    ($fn_name:ident, $compression:expr) => {
        pub fn $fn_name(
            file: vibeio::fs::File,
            len: Option<u64>,
        ) -> UnsyncBoxBody<Bytes, std::io::Error> {
            use tokio_util::io::{ReaderStream, StreamReader};
            let reader = StreamReader::new(FileStream::new(file, 0, len));
            let encoder = ($compression)(reader);
            StreamBody::new(
                ReaderStream::with_capacity(encoder, COMPRESSED_STREAM_READER_BUFFER_SIZE)
                    .map_ok(Frame::data)
                    .map_err(|e| e),
            )
            .boxed_unsync()
        }
    };
}

compress_streaming!(compress_streaming_gzip, |reader| {
    GzipEncoder::with_quality(reader, Level::Precise(4))
});
compress_streaming!(compress_streaming_deflate, |reader| {
    DeflateEncoder::with_quality(reader, Level::Precise(4))
});
compress_streaming!(compress_streaming_brotli, |reader| {
    BrotliEncoder::with_params(
        reader,
        EncoderParams::default()
            .quality(Level::Precise(4))
            .window_size(17)
            .block_size(18),
    )
});
compress_streaming!(compress_streaming_zstd, |reader| {
    ZstdEncoder::with_quality_and_params(
        reader,
        Level::Precise(4),
        &[CParameter::window_log(17), CParameter::hash_log(10)],
    )
});
