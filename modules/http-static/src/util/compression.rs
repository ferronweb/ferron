//! Compression-related utilities for static file serving.

use bytes::Bytes;
use futures_util::TryStreamExt;
use http_body::Frame;
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, StreamBody};

use super::file_stream::FileStream;

const COMPRESSED_STREAM_READER_BUFFER_SIZE: usize = 16384;

/// Compression algorithm
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    Gzip,
    Brotli,
    Deflate,
    Zstd,
    Identity,
}

#[allow(dead_code)]
impl Compression {
    /// Returns the HTTP `Content-Encoding` header value for this compression.
    pub fn header_value(self) -> Option<&'static str> {
        match self {
            Compression::Gzip => Some("gzip"),
            Compression::Brotli => Some("br"),
            Compression::Deflate => Some("deflate"),
            Compression::Zstd => Some("zstd"),
            Compression::Identity => None,
        }
    }

    /// Returns the file extension suffix for precompressed variants.
    pub fn precompressed_ext(self) -> Option<&'static str> {
        match self {
            Compression::Gzip => Some("gz"),
            Compression::Brotli => Some("br"),
            Compression::Deflate => Some("deflate"),
            Compression::Zstd => Some("zst"),
            Compression::Identity => None,
        }
    }

    /// Returns the ETag suffix for this compression.
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

/// Compress a file stream using Gzip.
pub fn compress_streaming_gzip(file: vibeio::fs::File) -> UnsyncBoxBody<Bytes, std::io::Error> {
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

/// Compress a file stream using Brotli.
pub fn compress_streaming_brotli(file: vibeio::fs::File) -> UnsyncBoxBody<Bytes, std::io::Error> {
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

/// Compress a file stream using Zstd.
pub fn compress_streaming_zstd(file: vibeio::fs::File) -> UnsyncBoxBody<Bytes, std::io::Error> {
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

/// Compress a file stream using Deflate.
pub fn compress_streaming_deflate(file: vibeio::fs::File) -> UnsyncBoxBody<Bytes, std::io::Error> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_compressible_extensions_contains_common_formats() {
        assert!(NON_COMPRESSIBLE_FILE_EXTENSIONS.contains("jpg"));
        assert!(NON_COMPRESSIBLE_FILE_EXTENSIONS.contains("png"));
        assert!(NON_COMPRESSIBLE_FILE_EXTENSIONS.contains("pdf"));
        assert!(NON_COMPRESSIBLE_FILE_EXTENSIONS.contains("zip"));
        assert!(NON_COMPRESSIBLE_FILE_EXTENSIONS.contains("mp4"));
    }

    #[test]
    fn non_compressible_extensions_does_not_contain_text_formats() {
        assert!(!NON_COMPRESSIBLE_FILE_EXTENSIONS.contains("html"));
        assert!(!NON_COMPRESSIBLE_FILE_EXTENSIONS.contains("css"));
        assert!(!NON_COMPRESSIBLE_FILE_EXTENSIONS.contains("js"));
        assert!(!NON_COMPRESSIBLE_FILE_EXTENSIONS.contains("txt"));
        assert!(!NON_COMPRESSIBLE_FILE_EXTENSIONS.contains("json"));
    }

    #[test]
    fn compression_header_values() {
        assert_eq!(Compression::Gzip.header_value(), Some("gzip"));
        assert_eq!(Compression::Brotli.header_value(), Some("br"));
        assert_eq!(Compression::Deflate.header_value(), Some("deflate"));
        assert_eq!(Compression::Zstd.header_value(), Some("zstd"));
        assert_eq!(Compression::Identity.header_value(), None);
    }

    #[test]
    fn compression_precompressed_ext() {
        assert_eq!(Compression::Gzip.precompressed_ext(), Some("gz"));
        assert_eq!(Compression::Brotli.precompressed_ext(), Some("br"));
        assert_eq!(Compression::Deflate.precompressed_ext(), Some("deflate"));
        assert_eq!(Compression::Zstd.precompressed_ext(), Some("zst"));
        assert_eq!(Compression::Identity.precompressed_ext(), None);
    }

    #[test]
    fn compression_etag_suffix() {
        assert_eq!(Compression::Gzip.etag_suffix(), Some("-gzip"));
        assert_eq!(Compression::Brotli.etag_suffix(), Some("-br"));
        assert_eq!(Compression::Deflate.etag_suffix(), Some("-deflate"));
        assert_eq!(Compression::Zstd.etag_suffix(), Some("-zstd"));
        assert_eq!(Compression::Identity.etag_suffix(), None);
    }

    #[test]
    fn compression_equality() {
        assert_eq!(Compression::Gzip, Compression::Gzip);
        assert_ne!(Compression::Gzip, Compression::Brotli);
    }
}
