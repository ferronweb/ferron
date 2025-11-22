use std::collections::BTreeSet;
use std::error::Error;
use std::sync::{Arc, LazyLock};

use async_compression::brotli::EncoderParams;
use async_compression::tokio::bufread::{BrotliEncoder, DeflateEncoder, GzipEncoder, ZstdEncoder};
use async_compression::zstd::CParameter;
use async_compression::Level;
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::TryStreamExt;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, StreamBody};
use hyper::body::Frame;
use hyper::{header, Request, Response};

use ferron_common::config::ServerConfiguration;
use ferron_common::logging::ErrorLogger;
use ferron_common::modules::{Module, ModuleHandlers, ModuleLoader, ResponseData, SocketData};
use ferron_common::util::ModuleCache;
use ferron_common::{get_entries_for_validation, get_value};
use tokio_util::io::{ReaderStream, StreamReader};

const COMPRESSED_STREAM_READER_BUFFER_SIZE: usize = 16384;

/// A hard-coded list of non-compressible MIME types
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

/// A dynamic content compression module loader
pub struct DynamicCompressionModuleLoader {
  cache: ModuleCache<DynamicCompressionModule>,
}

impl Default for DynamicCompressionModuleLoader {
  fn default() -> Self {
    Self::new()
  }
}

impl DynamicCompressionModuleLoader {
  /// Creates a new module loader
  pub fn new() -> Self {
    Self {
      cache: ModuleCache::new(vec![]),
    }
  }
}

impl ModuleLoader for DynamicCompressionModuleLoader {
  fn load_module(
    &mut self,
    config: &ServerConfiguration,
    _global_config: Option<&ServerConfiguration>,
    _secondary_runtime: &tokio::runtime::Runtime,
  ) -> Result<Arc<dyn Module + Send + Sync>, Box<dyn Error + Send + Sync>> {
    Ok(
      self
        .cache
        .get_or_init::<_, Box<dyn std::error::Error + Send + Sync>>(config, move |_| {
          Ok(Arc::new(DynamicCompressionModule))
        })?,
    )
  }

  fn get_requirements(&self) -> Vec<&'static str> {
    vec!["dynamic_compressed"]
  }

  fn validate_configuration(
    &self,
    config: &ServerConfiguration,
    used_properties: &mut std::collections::HashSet<String>,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    if let Some(entries) = get_entries_for_validation!("dynamic_compressed", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `dynamic_compressed` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!("Invalid dynamic content compression enabling option"))?
        }
      }
    }

    Ok(())
  }
}

/// A dynamic content compression module
struct DynamicCompressionModule;

impl Module for DynamicCompressionModule {
  fn get_module_handlers(&self) -> Box<dyn ModuleHandlers> {
    Box::new(DynamicCompressionModuleHandlers {
      user_agent: None,
      accept_encoding: None,
      compression_enabled: false,
    })
  }
}

/// Handlers for the dynamic content compression module
struct DynamicCompressionModuleHandlers {
  user_agent: Option<String>,
  accept_encoding: Option<String>,
  compression_enabled: bool,
}

#[async_trait(?Send)]
impl ModuleHandlers for DynamicCompressionModuleHandlers {
  async fn request_handler(
    &mut self,
    request: Request<BoxBody<Bytes, std::io::Error>>,
    config: &ServerConfiguration,
    _socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    self.user_agent = match request.headers().get(hyper::header::USER_AGENT) {
      Some(user_agent_value) => user_agent_value.to_str().ok().map(|v| v.to_owned()),
      None => None,
    };

    self.accept_encoding = match request.headers().get(hyper::header::ACCEPT_ENCODING) {
      Some(header_value) => header_value.to_str().ok().map(|v| v.to_owned()),
      None => None,
    };

    self.compression_enabled = get_value!("dynamic_compressed", config)
      .and_then(|v| v.as_bool())
      .unwrap_or(false);

    Ok(ResponseData {
      request: Some(request),
      response: None,
      response_status: None,
      response_headers: None,
      new_remote_address: None,
    })
  }

  async fn response_modifying_handler(
    &mut self,
    mut response: Response<BoxBody<Bytes, std::io::Error>>,
  ) -> Result<Response<BoxBody<Bytes, std::io::Error>>, Box<dyn Error>> {
    // Initialize compression flags
    let mut use_gzip = false;
    let mut use_deflate = false;
    let mut use_brotli = false;
    let mut use_zstd = false;

    // Get response content type
    let content_type_option = response
      .headers()
      .get(hyper::header::CONTENT_TYPE)
      .and_then(|v| v.to_str().ok())
      .map(|v| v.split_once(';').map_or(v, |s| s.0).trim());

    let compressible = self.compression_enabled
      && !response.headers().contains_key(header::CONTENT_ENCODING)
      && content_type_option.is_none_or(|t| !NON_COMPRESSIBLE_MIME_TYPES.contains(&t));

    // Determine the appropriate compression algorithm based on Accept-Encoding
    if compressible {
      // Get User-Agent for browser compatibility checks
      let user_agent = self.user_agent.as_deref().unwrap_or("");

      // Check for browsers with known compression bugs
      // Some web browsers have broken HTTP compression handling
      let (is_netscape_4_broken_html_compression, is_netscape_4_broken_compression) =
        match user_agent.strip_prefix("Mozilla/4.") {
          Some(stripped_user_agent) => {
            if user_agent.contains(" MSIE ") {
              // Internet Explorer "masquerading" as Netscape 4.x
              (false, false)
            } else {
              (
                true,
                matches!(stripped_user_agent.chars().nth(0), Some('0'))
                  && matches!(stripped_user_agent.chars().nth(1), Some('6') | Some('7') | Some('8')),
              )
            }
          }
          None => (false, false),
        };
      let is_w3m_broken_html_compression = user_agent.starts_with("w3m/");
      if !(content_type_option == Some("text/html")
        && (is_netscape_4_broken_html_compression || is_w3m_broken_html_compression))
        && !is_netscape_4_broken_compression
      {
        // Get Accept-Encoding header to determine supported compression algorithms
        let accept_encoding = self.accept_encoding.as_deref().unwrap_or("");

        // Parse Accept-Encoding header to select the best compression method
        // Check for supported compression algorithms in order of preference
        if accept_encoding.contains("br") {
          use_brotli = true;
        }
        if (!use_brotli) && accept_encoding.contains("zstd") {
          use_zstd = true;
        }
        if (!(use_brotli || use_zstd)) && accept_encoding.contains("deflate") {
          use_deflate = true;
        }
        if (!(use_brotli || use_zstd || use_deflate)) && accept_encoding.contains("gzip") {
          use_gzip = true;
        }
      }
    }

    // Add Accept-Encoding header to Vary header
    if let Some(vary) = response.headers_mut().get_mut(header::VARY) {
      let mut old_vary = vary
        .to_str()
        .map(|v| v.split(',').map(|v| v.trim()).collect::<Vec<&str>>())
        .unwrap_or(vec![]);
      if !old_vary.iter().any(|v| v.to_lowercase() == "accept-encoding") {
        old_vary.push("Accept-Encoding");
        *vary = old_vary.join(", ").try_into()?;
      }
    } else {
      response
        .headers_mut()
        .insert(header::VARY, "Accept-Encoding".parse().unwrap());
    }

    // Remove Content-Length header if compression is used
    if use_brotli || use_zstd || use_deflate || use_gzip {
      while response.headers_mut().remove(header::CONTENT_LENGTH).is_some() {}
    }

    // Content-Encoding header value
    let algorithm_str = if use_brotli {
      Some("br")
    } else if use_zstd {
      Some("zstd")
    } else if use_deflate {
      Some("deflate")
    } else if use_gzip {
      Some("gzip")
    } else {
      None
    };

    // Add ETag suffix based on compression method
    if let Some(etag) = response.headers_mut().get_mut(header::ETAG) {
      if let Ok(etag_str) = etag.to_str() {
        if let Some(algorithm_str) = &algorithm_str {
          if let Some(etag_str) = etag_str.strip_suffix('"') {
            *etag = format!("{etag_str}-dynamic-{algorithm_str}\"").try_into()?;
          } else {
            *etag = format!("{etag_str}-dynamic-{algorithm_str}").try_into()?;
          }
        }
      }
    }

    if let Some(algorithm_str) = &algorithm_str {
      // Add Content-Encoding header
      response
        .headers_mut()
        .insert(header::CONTENT_ENCODING, algorithm_str.parse()?);

      let (response_parts, response_body) = response.into_parts();

      // Create the appropriate response body based on compression method
      let boxed_body = if use_brotli {
        let body_reader = StreamReader::new(response_body.into_data_stream());

        // Use Brotli compression with moderate quality (4) for good compression/speed balance
        // Also, set the window size and block size to optimize compression, and reduce memory usage
        let reader_stream = ReaderStream::with_capacity(
          BrotliEncoder::with_params(
            body_reader,
            EncoderParams::default()
              .quality(Level::Precise(4))
              .window_size(17)
              .block_size(18),
          ),
          COMPRESSED_STREAM_READER_BUFFER_SIZE,
        );
        let stream_body = StreamBody::new(reader_stream.map_ok(Frame::data));
        stream_body.boxed()
      } else if use_zstd {
        let body_reader = StreamReader::new(response_body.into_data_stream());

        // Limit the Zstandard window size to 128K (2^17 bytes) to support many HTTP clients
        // Also, set the size of the initial probe table to reduce memory usage
        let reader_stream = ReaderStream::with_capacity(
          ZstdEncoder::with_quality_and_params(
            body_reader,
            Level::Default,
            &[CParameter::window_log(17), CParameter::hash_log(10)],
          ),
          COMPRESSED_STREAM_READER_BUFFER_SIZE,
        );
        let stream_body = StreamBody::new(reader_stream.map_ok(Frame::data));
        stream_body.boxed()
      } else if use_deflate {
        let body_reader = StreamReader::new(response_body.into_data_stream());

        let reader_stream =
          ReaderStream::with_capacity(DeflateEncoder::new(body_reader), COMPRESSED_STREAM_READER_BUFFER_SIZE);
        let stream_body = StreamBody::new(reader_stream.map_ok(Frame::data));
        stream_body.boxed()
      } else if use_gzip {
        let body_reader = StreamReader::new(response_body.into_data_stream());

        let reader_stream =
          ReaderStream::with_capacity(GzipEncoder::new(body_reader), COMPRESSED_STREAM_READER_BUFFER_SIZE);
        let stream_body = StreamBody::new(reader_stream.map_ok(Frame::data));
        stream_body.boxed()
      } else {
        // No compression algorithm is used, so we can just return the original response body
        response_body
      };

      response = Response::from_parts(response_parts, boxed_body);
    }

    Ok(response)
  }
}
