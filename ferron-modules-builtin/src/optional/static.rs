use std::collections::{BTreeSet, HashSet};
use std::error::Error;
use std::ffi::OsStr;
#[cfg(feature = "runtime-monoio")]
use std::fs::ReadDir;
#[cfg(feature = "runtime-tokio")]
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, SystemTime};

use async_compression::brotli::EncoderParams;
use async_compression::tokio::bufread::{BrotliEncoder, DeflateEncoder, GzipEncoder, ZstdEncoder};
use async_compression::zstd::CParameter;
use async_compression::Level;
use async_trait::async_trait;
use chrono::{DateTime, Local};
use futures_util::TryStreamExt;
use hashlink::LruCache;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty, Full, StreamBody};
use hyper::body::{Bytes, Frame};
use hyper::header::{self, HeaderValue};
use hyper::{HeaderMap, Method, Request, Response, StatusCode};
#[cfg(feature = "runtime-monoio")]
use monoio::fs;
#[cfg(feature = "runtime-tokio")]
use tokio::fs::{self, ReadDir};
#[cfg(feature = "runtime-tokio")]
use tokio::io::{AsyncReadExt, AsyncSeekExt, BufReader};
use tokio::sync::RwLock;
use tokio_util::io::{ReaderStream, StreamReader};

use ferron_common::config::ServerConfiguration;
use ferron_common::logging::ErrorLogger;
use ferron_common::modules::{Module, ModuleHandlers, ModuleLoader, RequestData, ResponseData, SocketData};
#[cfg(feature = "runtime-monoio")]
use ferron_common::util::MonoioFileStreamNoSpawn;
use ferron_common::util::{anti_xss, sizify, ModuleCache, TtlCache};
use ferron_common::{format_page, get_entries, get_entries_for_validation, get_entry, get_value};

const COMPRESSED_STREAM_READER_BUFFER_SIZE: usize = 16384;

/// A hard-coded list of non-compressible file extensions
static NON_COMPRESSIBLE_FILE_EXTENSIONS: LazyLock<BTreeSet<&'static str>> = LazyLock::new(|| {
  BTreeSet::from_iter(vec![
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

/// Generates a directory listing
#[inline]
pub async fn generate_directory_listing(
  directory: ReadDir,
  request_path: &str,
  description: Option<String>,
) -> Result<String, Box<dyn Error + Send + Sync>> {
  let mut request_path_without_trailing_slashes = request_path;
  while request_path_without_trailing_slashes.ends_with("/") {
    request_path_without_trailing_slashes =
      &request_path_without_trailing_slashes[..(request_path_without_trailing_slashes.len() - 1)];
  }

  // Return path
  let mut return_path_vec: Vec<&str> = request_path_without_trailing_slashes.split("/").collect();
  return_path_vec.pop();
  return_path_vec.push("");
  let return_path = &return_path_vec.join("/") as &str;

  let mut table_rows = Vec::new();
  if !request_path_without_trailing_slashes.is_empty() {
    table_rows.push(format!(
      "<tr><td>⬆️ <a href=\"{}\">Return</a></td><td></td><td></td></tr>",
      anti_xss(return_path)
    ));
  }
  let min_table_rows_length = table_rows.len();

  // Create a vector containing entries, then sort them by file name.
  #[cfg(feature = "runtime-monoio")]
  let mut entries = monoio::spawn_blocking(move || {
    let mut entries = Vec::new();
    for entry in directory {
      entries.push(entry?);
    }
    Ok(entries)
  })
  .await
  .unwrap_or(Err(std::io::Error::other(
    "Can't spawn a blocking task to obtain the files in a directory",
  )))?;
  #[cfg(feature = "runtime-tokio")]
  let mut entries = {
    let mut entries = Vec::new();
    let mut directory = directory;
    while let Some(entry) = directory.next_entry().await? {
      entries.push(entry);
    }
    entries
  };

  entries.sort_by_cached_key(|entry| entry.file_name().to_string_lossy().to_string());

  for entry in entries.iter() {
    let filename = entry.file_name().to_string_lossy().to_string();
    if filename.starts_with('.') {
      // Don't add files nor directories with "." at the beginning of their names
      continue;
    }

    // Monoio's `fs` doesn't expose `metadata()` on Windows, so we have to spawn a blocking task to obtain the metadata on this platform
    #[cfg(any(feature = "runtime-tokio", all(feature = "runtime-monoio", unix)))]
    let metadata_obt = fs::metadata(entry.path()).await;
    #[cfg(all(feature = "runtime-monoio", windows))]
    let metadata_obt = {
      let entry_pathbuf = entry.path().clone();
      monoio::spawn_blocking(move || std::fs::metadata(entry_pathbuf))
        .await
        .unwrap_or(Err(std::io::Error::other(
          "Can't spawn a blocking task to obtain the file metadata",
        )))
    };

    match metadata_obt {
      Ok(metadata) => {
        let filename_link = format!(
          "{} <a href=\"{}/{}{}\">{}</a>",
          if metadata.is_dir() {
            "📁"
          } else if metadata.is_file() {
            "📄"
          } else {
            "❓"
          },
          request_path_without_trailing_slashes,
          anti_xss(urlencoding::encode(&filename).as_ref()),
          match metadata.is_dir() {
            true => "/",
            false => "",
          },
          anti_xss(&filename)
        );

        let row = format!(
          "<tr><td class=\"directory-filename\">{}</td>\
          <td class=\"directory-size\">{}</td><td class=\"directory-date\">{}</td></tr>",
          filename_link,
          match metadata.is_file() {
            true => anti_xss(&sizify(metadata.len(), false)),
            false => "-".to_string(),
          },
          anti_xss(
            &(match metadata.modified() {
              Ok(mtime) => {
                let datetime: DateTime<Local> = mtime.into();
                datetime.format("%a %b %d %Y").to_string()
              }
              Err(_) => "-".to_string(),
            })
          )
        );
        table_rows.push(row);
      }
      Err(_) => {
        let filename_link = format!(
          "⚠️ <a href=\"{}/{}\">{}</a>",
          request_path_without_trailing_slashes,
          anti_xss(urlencoding::encode(&filename).as_ref()),
          anti_xss(&filename)
        );
        let row = format!(
          "<tr><td class=\"directory-filename\">{filename_link}</td>\
          <td class=\"directory-size\">-</td><td class=\"directory-date\">-</td></tr>"
        );
        table_rows.push(row);
      }
    };
  }

  if table_rows.len() <= min_table_rows_length {
    table_rows.push(
      "<tr><td class=\"directory-filename\">🤷 No files found</td>\
        <td class=\"directory-size\"></td><td class=\"directory-date\"></td></tr>"
        .to_string(),
    );
  }

  Ok(format_page!(
    format!(
      "<h1>Directory: {}</h1>
      <table>
      <tr><th class=\"directory-filename\">Filename</th><th class=\"directory-size\">Size</th>\
      <th class=\"directory-date\">Date</th></tr>
      {}
    </table>{}",
      anti_xss(request_path),
      table_rows.join(""),
      match description {
        Some(description) => format!(
          "<hr><pre class=\"directory-description\">{}</pre>",
          anti_xss(&description)
        ),
        None => "".to_string(),
      }
    ),
    &format!("Directory: {request_path}"),
    vec![
      include_str!("../../../assets/common.css"),
      include_str!("../../../assets/directory.css")
    ]
  ))
}

/// Parses the HTTP "Range" header value
#[inline]
fn parse_range_header(range_str: &str, default_end: u64) -> Option<(u64, u64)> {
  if let Some(range_part) = range_str.strip_prefix("bytes=") {
    let parts: Vec<&str> = range_part.split('-').take(2).collect();
    if parts.len() == 2 {
      if parts[0].is_empty() {
        if let Ok(end) = u64::from_str(parts[1]) {
          return Some((default_end - end + 1, default_end));
        }
      } else if parts[1].is_empty() {
        if let Ok(start) = u64::from_str(parts[0]) {
          return Some((start, default_end));
        }
      } else if !parts[0].is_empty() && !parts[1].is_empty() {
        if let (Ok(start), Ok(end)) = (u64::from_str(parts[0]), u64::from_str(parts[1])) {
          return Some((start, end));
        }
      }
    }
  }
  None
}

/// Extracts inner ETag
#[inline]
fn extract_etag_inner(input: &str, weak: bool) -> Option<String> {
  // Remove the surrounding double quotes and preceding "W/"
  let weak_might_removed = if weak {
    match input.strip_prefix("W/") {
      Some(stripped) => stripped,
      None => input,
    }
  } else {
    input
  };
  let trimmed = weak_might_removed.trim_matches('"');

  // Split the string at the hyphen and take the first part
  trimmed.split('-').next().map(ToOwned::to_owned)
}

/// Converts strong ETag to weak one, if it's not a weak one
#[inline]
fn etag_strong_to_weak(input: &str) -> String {
  if input.starts_with("W/") {
    input.to_string()
  } else {
    format!("W/{input}")
  }
}

/// A static file serving module loader
pub struct StaticFileServingModuleLoader {
  cache: ModuleCache<StaticFileServingModule>,
  pathbuf_cache: Arc<RwLock<TtlCache<String, PathBuf>>>,
  path_traversal_check_cache: Arc<RwLock<TtlCache<PathBuf, bool>>>,
  etag_cache: Arc<RwLock<LruCache<String, String>>>,
}

impl Default for StaticFileServingModuleLoader {
  fn default() -> Self {
    Self::new()
  }
}

impl StaticFileServingModuleLoader {
  /// Creates a new module loader
  pub fn new() -> Self {
    Self {
      cache: ModuleCache::new(vec![]),
      pathbuf_cache: Arc::new(RwLock::new(TtlCache::new(Duration::from_millis(100)))),
      path_traversal_check_cache: Arc::new(RwLock::new(TtlCache::new(Duration::from_millis(100)))),
      etag_cache: Arc::new(RwLock::new(LruCache::new(1000))),
    }
  }
}

impl ModuleLoader for StaticFileServingModuleLoader {
  fn load_module(
    &mut self,
    config: &ServerConfiguration,
    _global_config: Option<&ServerConfiguration>,
    _secondary_runtime: &tokio::runtime::Runtime,
  ) -> Result<Arc<dyn Module + Send + Sync>, Box<dyn Error + Send + Sync>> {
    Ok(
      self
        .cache
        .get_or_init::<_, Box<dyn std::error::Error + Send + Sync>>(config, |_| {
          Ok(Arc::new(StaticFileServingModule {
            pathbuf_cache: self.pathbuf_cache.clone(),
            path_traversal_check_cache: self.path_traversal_check_cache.clone(),
            etag_cache: self.etag_cache.clone(),
          }))
        })?,
    )
  }

  fn get_requirements(&self) -> Vec<&'static str> {
    vec!["root"]
  }

  fn validate_configuration(
    &self,
    config: &ServerConfiguration,
    used_properties: &mut HashSet<String>,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    if let Some(entries) = get_entries_for_validation!("compressed", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `compressed` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!("Invalid static file compression enabling option"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("directory_listing", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `directory_listing` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!("Invalid directory listing enabling option"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("etag", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `etag` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!("Invalid ETag enabling option"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("file_cache_control", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `file_cache_control` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_string() && !entry.values[0].is_null() {
          Err(anyhow::anyhow!("Invalid file cache control header value"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("precompressed", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `precompressed` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!("Invalid static file precompression enabling option"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("mime_type", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 2 {
          Err(anyhow::anyhow!(
            "The `mime_type` configuration property must have exactly two values"
          ))?
        } else if !entry.values[0].is_string() {
          Err(anyhow::anyhow!("The file extension must be a string"))?
        } else if !entry.values[1].is_string() {
          Err(anyhow::anyhow!("The MIME type must be a string"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("index", config, used_properties) {
      for entry in &entries.inner {
        if !entry.values.iter().all(|v| v.is_string()) {
          Err(anyhow::anyhow!("An index file name must be a string"))?
        }
      }
    }

    Ok(())
  }
}

/// A static file serving module
struct StaticFileServingModule {
  pathbuf_cache: Arc<RwLock<TtlCache<String, PathBuf>>>,
  path_traversal_check_cache: Arc<RwLock<TtlCache<PathBuf, bool>>>,
  etag_cache: Arc<RwLock<LruCache<String, String>>>,
}

impl Module for StaticFileServingModule {
  fn get_module_handlers(&self) -> Box<dyn ModuleHandlers> {
    Box::new(StaticFileServingModuleHandlers {
      pathbuf_cache: self.pathbuf_cache.clone(),
      path_traversal_check_cache: self.path_traversal_check_cache.clone(),
      etag_cache: self.etag_cache.clone(),
    })
  }
}

/// Handlers for the static file serving module
struct StaticFileServingModuleHandlers {
  pathbuf_cache: Arc<RwLock<TtlCache<String, PathBuf>>>,
  path_traversal_check_cache: Arc<RwLock<TtlCache<PathBuf, bool>>>,
  etag_cache: Arc<RwLock<LruCache<String, String>>>,
}

#[async_trait(?Send)]
impl ModuleHandlers for StaticFileServingModuleHandlers {
  /// Handles incoming HTTP requests for static file serving
  ///
  /// This is the main handler for the static file serving module which:
  /// - Processes various HTTP methods (GET, POST, HEAD, OPTIONS)
  /// - Handles conditional requests with ETags
  /// - Supports partial content with Range headers
  /// - Provides file compression when appropriate
  /// - Generates directory listings when configured
  async fn request_handler(
    &mut self,
    request: Request<BoxBody<Bytes, std::io::Error>>,
    config: &ServerConfiguration,
    _socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    // Handle different HTTP methods
    match request.method() {
      // OPTIONS method: Return allowed methods without body
      &Method::OPTIONS => {
        return Ok(ResponseData {
          request: Some(request),
          response: Some(
            Response::builder()
              .status(StatusCode::NO_CONTENT)
              .header(header::ALLOW, HeaderValue::from_static("GET, POST, HEAD, OPTIONS"))
              .body(Empty::new().map_err(|e| match e {}).boxed())
              .unwrap_or_default(),
          ),
          response_status: None,
          response_headers: None,
          new_remote_address: None,
        });
      }
      // GET, POST, HEAD methods are allowed and handled below
      &Method::GET | &Method::POST | &Method::HEAD => (),
      // All other methods are not allowed
      _ => {
        let mut header_map = HeaderMap::new();
        header_map.insert(header::ALLOW, HeaderValue::from_static("GET, POST, HEAD, OPTIONS"));
        return Ok(ResponseData {
          request: Some(request),
          response: None,
          response_status: Some(StatusCode::METHOD_NOT_ALLOWED),
          response_headers: Some(header_map),
          new_remote_address: None,
        });
      }
    }

    // Get the web root directory from configuration
    if let Some(wwwroot) = get_entry!("root", config)
      .and_then(|e| e.values.first())
      .and_then(|v| v.as_str())
    {
      // Extract and validate the request path
      let request_path = request.uri().path();
      let mut request_path_bytes = request_path.bytes();
      // Ensure path starts with a forward slash
      if request_path_bytes.len() < 1 || request_path_bytes.nth(0) != Some(b'/') {
        return Ok(ResponseData {
          request: Some(request),
          response: None,
          response_status: Some(StatusCode::BAD_REQUEST),
          response_headers: None,
          new_remote_address: None,
        });
      }

      // Get the original request path from request extensions (used for directory listings)
      let original_request_path = request
        .extensions()
        .get::<RequestData>()
        .and_then(|d| d.original_url.as_ref())
        .map_or(request_path, |u| u.path());

      // Get the configured index files
      let indexes = get_entry!("index", config)
        .map(|e| e.values.iter().filter_map(|v| v.as_str()).collect::<Vec<&str>>())
        .unwrap_or(vec!["index.html", "index.htm", "index.xhtml"]);

      // Create a cache key that includes IP and hostname filters if present
      let cache_key = format!(
        "{}{}{}",
        match &config.filters.ip {
          Some(ip) => format!("{ip}-"),
          None => String::from(""),
        },
        match &config.filters.hostname {
          Some(domain) => format!("{domain}-"),
          None => String::from(""),
        },
        request_path
      );

      // Try to get the file path from cache
      let rwlock_read = self.pathbuf_cache.read().await;
      let joined_pathbuf_option = rwlock_read.get(&cache_key);
      drop(rwlock_read);

      let joined_pathbuf_cached = joined_pathbuf_option.is_some();
      let mut joined_pathbuf = match joined_pathbuf_option {
        // Use cached path if available
        Some(joined_pathbuf) => joined_pathbuf,
        // Otherwise, construct the file path
        None => {
          let path = Path::new(wwwroot);
          // Strip leading slash and normalize path
          let mut relative_path = &request_path[1..];
          while relative_path.as_bytes().first().copied() == Some(b'/') {
            relative_path = &relative_path[1..];
          }

          // URL-decode the path
          let decoded_relative_path = match urlencoding::decode(relative_path) {
            Ok(path) => path.to_string(),
            Err(_) => {
              // Return BAD_REQUEST if URL decoding fails
              return Ok(ResponseData {
                request: Some(request),
                response: None,
                response_status: Some(StatusCode::BAD_REQUEST),
                response_headers: None,
                new_remote_address: None,
              });
            }
          };

          // Join the web root with the decoded relative path
          path.join(decoded_relative_path)
        }
      };

      // Check for possible path traversal attack, if the URL sanitizer is disabled.
      if get_value!("disable_url_sanitizer", config)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
      {
        let rwlock_read = self.path_traversal_check_cache.read().await;
        let allowed_option = rwlock_read.get(&joined_pathbuf);
        drop(rwlock_read);
        let allowed = match allowed_option {
          Some(allowed) => allowed,
          None => {
            // Canonicalize the file path
            #[cfg(feature = "runtime-monoio")]
            let canonicalize_result = {
              let joined_pathbuf = joined_pathbuf.clone();
              monoio::spawn_blocking(move || std::fs::canonicalize(joined_pathbuf))
                .await
                .unwrap_or(Err(std::io::Error::other(
                  "Can't spawn a blocking task to obtain the canonical file path",
                )))
            };
            #[cfg(feature = "runtime-tokio")]
            let canonicalize_result = fs::canonicalize(&joined_pathbuf).await;

            let canonical_joined_pathbuf = match canonicalize_result {
              Ok(pathbuf) => pathbuf,
              Err(_) => joined_pathbuf.clone(),
            };

            // Canonicalize the webroot
            #[cfg(feature = "runtime-monoio")]
            let canonicalize_result = {
              let wwwroot = wwwroot.to_owned();
              monoio::spawn_blocking(move || std::fs::canonicalize(wwwroot))
                .await
                .unwrap_or(Err(std::io::Error::other(
                  "Can't spawn a blocking task to obtain the canonical file path",
                )))
            };
            #[cfg(feature = "runtime-tokio")]
            let canonicalize_result = fs::canonicalize(wwwroot).await;

            let canonical_wwwroot = match canonicalize_result {
              Ok(pathbuf) => pathbuf,
              Err(_) => PathBuf::from_str(wwwroot)?,
            };

            let allowed = canonical_joined_pathbuf.starts_with(&canonical_wwwroot);

            let mut rwlock_write = self.path_traversal_check_cache.write().await;
            rwlock_write.insert(joined_pathbuf.clone(), allowed);
            drop(rwlock_write);

            allowed
          }
        };

        // Return 403 Forbidden if the path is outside the webroot
        if !allowed {
          return Ok(ResponseData {
            request: Some(request),
            response: None,
            response_status: Some(StatusCode::FORBIDDEN),
            response_headers: None,
            new_remote_address: None,
          });
        }
      }

      // Get file metadata (platform-specific implementation)
      // Monoio's `fs` doesn't expose `metadata()` on Windows, so we have to spawn a blocking task to obtain the metadata on this platform
      #[cfg(any(feature = "runtime-tokio", all(feature = "runtime-monoio", unix)))]
      let metadata_obt = fs::metadata(&joined_pathbuf).await;
      #[cfg(all(feature = "runtime-monoio", windows))]
      let metadata_obt = {
        let joined_pathbuf = joined_pathbuf.clone();
        monoio::spawn_blocking(move || std::fs::metadata(joined_pathbuf))
          .await
          .unwrap_or(Err(std::io::Error::other(
            "Can't spawn a blocking task to obtain the file metadata",
          )))
      };

      match metadata_obt {
        Ok(mut metadata) => {
          // If the path wasn't in cache and it's a directory, try to find an index file
          if !joined_pathbuf_cached {
            if metadata.is_dir() {
              // Try common index file names
              for index in indexes {
                let temp_joined_pathbuf = joined_pathbuf.join(index);

                // Monoio's `fs` doesn't expose `metadata()` on Windows, so we have to spawn a blocking task to obtain the metadata on this platform
                #[cfg(any(feature = "runtime-tokio", all(feature = "runtime-monoio", unix)))]
                let metadata_obt = fs::metadata(&temp_joined_pathbuf).await;
                #[cfg(all(feature = "runtime-monoio", windows))]
                let metadata_obt = {
                  let temp_joined_pathbuf = temp_joined_pathbuf.clone();
                  monoio::spawn_blocking(move || std::fs::metadata(temp_joined_pathbuf))
                    .await
                    .unwrap_or(Err(std::io::Error::other(
                      "Can't spawn a blocking task to obtain the file metadata",
                    )))
                };

                match metadata_obt {
                  Ok(temp_metadata) => {
                    // If an index file exists, use it instead of the directory
                    if temp_metadata.is_file() {
                      metadata = temp_metadata;
                      joined_pathbuf = temp_joined_pathbuf;
                      break;
                    }
                  }
                  Err(err) => match err.kind() {
                    // Skip if file doesn't exist and try next index
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory => {
                      continue;
                    }
                    std::io::ErrorKind::PermissionDenied => {
                      return Ok(ResponseData {
                        request: Some(request),
                        response: None,
                        response_status: Some(StatusCode::FORBIDDEN),
                        response_headers: None,
                        new_remote_address: None,
                      });
                    }
                    _ => Err(err)?,
                  },
                };
              }
            }
            // Cache the resolved path for future requests
            let mut rwlock_write = self.pathbuf_cache.write().await;
            rwlock_write.cleanup();
            rwlock_write.insert(cache_key, joined_pathbuf.clone());
            drop(rwlock_write);
          }

          if metadata.is_file() {
            // Handle file serving

            // Obtain the "Cache-Control" header value
            let cache_control = get_value!("file_cache_control", config).and_then(|v| v.as_str());

            // Determine if precompression is enabled
            let enable_precompression = get_value!("precompressed", config)
              .and_then(|v| v.as_bool())
              .unwrap_or(false);

            // Determine if compression should be used
            let mut compression_possible = false;

            // Check if compression is enabled in config (defaults to true)
            if enable_precompression
              || get_value!("compressed", config)
                .and_then(|v| v.as_bool())
                .unwrap_or(true)
            {
              let file_extension = joined_pathbuf
                .extension()
                .map_or_else(|| "".to_string(), |ext| ext.to_string_lossy().to_string());
              let file_extension_compressible = !NON_COMPRESSIBLE_FILE_EXTENSIONS.contains(&(&file_extension as &str));

              // Only compress files larger than 256 bytes and with compressible extensions
              if metadata.len() > 256 && file_extension_compressible {
                compression_possible = true;
              }
            }

            // Vary header will be used to indicate which request headers affect the response
            let vary;

            // Generate and handle ETags for caching
            let mut etag_option = None;
            // Check if ETags are enabled in config (defaults to true)
            if get_value!("etag", config).and_then(|v| v.as_bool()).unwrap_or(true) {
              // Create ETag cache key based on file path, size, and modification time
              let etag_cache_key = format!(
                "{}-{}-{}",
                joined_pathbuf.to_string_lossy(),
                metadata.len(),
                match metadata.modified() {
                  Ok(mtime) => {
                    (match mtime.duration_since(SystemTime::UNIX_EPOCH) {
                      Ok(duration) => duration.as_secs() as i128,
                      Err(error) => -(error.duration().as_secs() as i128),
                    })
                    .to_string()
                  }
                  Err(_) => String::from(""),
                }
              );
              let rwlock_read = self.etag_cache.read().await;
              // Had to use "peek", since "get" would mutate the LRU cache
              let etag_locked_option = rwlock_read.peek(&etag_cache_key).cloned();
              drop(rwlock_read);
              let etag = match etag_locked_option {
                Some(etag) => etag,
                None => {
                  let etag = format!("{:016x}", xxhash_rust::xxh3::xxh3_64(etag_cache_key.as_bytes()));

                  let mut rwlock_write = self.etag_cache.write().await;
                  rwlock_write.insert(etag_cache_key, etag.clone());
                  drop(rwlock_write);

                  etag
                }
              };

              // Set Vary header based on available features
              // Include Accept-Encoding if compression is possible
              vary = if compression_possible {
                "Accept-Encoding, If-Match, If-None-Match, Range"
              } else {
                "If-Match, If-None-Match, Range"
              };

              // Handle If-None-Match header for conditional requests
              // If the client's cached version matches our ETag, return 304 Not Modified
              if let Some(if_none_match_value) = request.headers().get(header::IF_NONE_MATCH) {
                match if_none_match_value.to_str() {
                  Ok(if_none_match) => {
                    if let Some(etag_extracted) = extract_etag_inner(if_none_match, true) {
                      // Client's cached version matches our current version
                      if etag_extracted == etag {
                        let etag_original = if_none_match.to_string();
                        let mut not_modified_response = Response::builder()
                          .status(StatusCode::NOT_MODIFIED)
                          .header(header::ETAG, etag_strong_to_weak(&etag_original))
                          .header(header::VARY, HeaderValue::from_static(vary))
                          .body(Empty::new().map_err(|e| match e {}).boxed())?;
                        if let Some(cache_control) = cache_control {
                          not_modified_response
                            .headers_mut()
                            .insert(header::CACHE_CONTROL, HeaderValue::from_str(cache_control)?);
                        }
                        return Ok(ResponseData {
                          request: Some(request),
                          response: Some(not_modified_response),
                          response_status: None,
                          response_headers: None,
                          new_remote_address: None,
                        });
                      }
                    }
                  }
                  Err(_) => {
                    let mut header_map = HeaderMap::new();
                    header_map.insert(header::VARY, HeaderValue::from_static(vary));
                    return Ok(ResponseData {
                      request: Some(request),
                      response: None,
                      response_status: Some(StatusCode::BAD_REQUEST),
                      response_headers: Some(header_map),
                      new_remote_address: None,
                    });
                  }
                }
              }

              // Handle If-Match header for conditional requests
              // Only proceed if the client's version matches our current version
              if let Some(if_match_value) = request.headers().get(header::IF_MATCH) {
                match if_match_value.to_str() {
                  Ok(if_match) => {
                    // "*" means any version is acceptable
                    if if_match != "*" {
                      if let Some(etag_extracted) = extract_etag_inner(if_match, true) {
                        // Client's version doesn't match our current version
                        if etag_extracted != etag {
                          let mut header_map = HeaderMap::new();
                          header_map.insert(header::ETAG, if_match_value.clone());
                          header_map.insert(header::VARY, HeaderValue::from_static(vary));
                          return Ok(ResponseData {
                            request: Some(request),
                            response: None,
                            response_status: Some(StatusCode::PRECONDITION_FAILED),
                            response_headers: Some(header_map),
                            new_remote_address: None,
                          });
                        }
                      }
                    }
                  }
                  Err(_) => {
                    let mut header_map = HeaderMap::new();
                    header_map.insert(header::VARY, HeaderValue::from_static(vary));
                    return Ok(ResponseData {
                      request: Some(request),
                      response: None,
                      response_status: Some(StatusCode::BAD_REQUEST),
                      response_headers: Some(header_map),
                      new_remote_address: None,
                    });
                  }
                }
              }
              etag_option = Some(etag);
            } else {
              vary = if compression_possible {
                "Accept-Encoding, Range"
              } else {
                "Range"
              };
            }

            let custom_content_type_option = {
              let mut custom_content_type = None;
              if let Some(mime_types_entries) = get_entries!("mime_type", config) {
                if let Some(extension) = joined_pathbuf.extension().map(|a| format!(".{}", a.to_string_lossy())) {
                  for entry in mime_types_entries.inner.iter() {
                    if let Some(key) = entry.values.first().and_then(|v| v.as_str()) {
                      if key == extension {
                        if let Some(value) = entry.values.get(1).and_then(|v| v.as_str()) {
                          custom_content_type = Some(value.to_string());
                          break;
                        }
                      }
                    }
                  }
                }
              }
              custom_content_type
            };

            // Determine the content type based on file extension
            let content_type_option = custom_content_type_option.or_else(|| {
              new_mime_guess::from_path(&joined_pathbuf)
                .first()
                .map(|mime_type| mime_type.to_string())
            });

            // Handle Range requests for partial content
            let range_header = match request.headers().get(header::RANGE) {
              Some(value) => match value.to_str() {
                Ok(value) => Some(value),
                Err(_) => {
                  let mut header_map = HeaderMap::new();
                  header_map.insert(header::VARY, HeaderValue::from_static(vary));
                  return Ok(ResponseData {
                    request: Some(request),
                    response: None,
                    response_status: Some(StatusCode::BAD_REQUEST),
                    response_headers: Some(header_map),
                    new_remote_address: None,
                  });
                }
              },
              None => None,
            };

            // Process range request if present
            if let Some(range_header) = range_header {
              // Get file size
              let file_length = metadata.len();
              // Can't satisfy range request for empty files
              if file_length == 0 {
                let mut header_map = HeaderMap::new();
                header_map.insert(header::VARY, HeaderValue::from_static(vary));
                return Ok(ResponseData {
                  request: Some(request),
                  response: None,
                  response_status: Some(StatusCode::RANGE_NOT_SATISFIABLE),
                  response_headers: Some(header_map),
                  new_remote_address: None,
                });
              }
              // Parse the range header to get start and end positions
              if let Some((range_begin, range_end)) = parse_range_header(range_header, file_length - 1) {
                // Validate the requested range is within file bounds
                if range_end > file_length - 1 || range_begin > file_length - 1 || range_begin > range_end {
                  let mut header_map = HeaderMap::new();
                  header_map.insert(header::VARY, HeaderValue::from_static(vary));
                  return Ok(ResponseData {
                    request: Some(request),
                    response: None,
                    response_status: Some(StatusCode::RANGE_NOT_SATISFIABLE),
                    response_headers: Some(header_map),
                    new_remote_address: None,
                  });
                }

                // Get the HTTP method and calculate content length for the partial response
                let request_method = request.method();
                let content_length = range_end - range_begin + 1;

                // Build the partial content response
                let mut response_builder = Response::builder()
                  .status(StatusCode::PARTIAL_CONTENT)
                  .header(header::CONTENT_LENGTH, content_length)
                  .header(
                    header::CONTENT_RANGE,
                    format!("bytes {range_begin}-{range_end}/{file_length}"),
                  );

                if let Some(etag) = etag_option {
                  response_builder = response_builder.header(header::ETAG, format!("W/\"{etag}\""));
                }

                if let Some(content_type) = content_type_option {
                  response_builder = response_builder.header(header::CONTENT_TYPE, content_type);
                }

                if let Some(cache_control) = cache_control {
                  response_builder = response_builder.header(header::CACHE_CONTROL, cache_control);
                }

                response_builder = response_builder.header(header::VARY, HeaderValue::from_static(vary));

                let response = match request_method {
                  &Method::HEAD => response_builder.body(Empty::new().map_err(|e| match e {}).boxed())?,
                  _ => {
                    // Open file for reading
                    let file = match fs::File::open(joined_pathbuf).await {
                      Ok(file) => file,
                      Err(err) => match err.kind() {
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory => {
                          return Ok(ResponseData {
                            request: Some(request),
                            response: None,
                            response_status: Some(StatusCode::NOT_FOUND),
                            response_headers: None,
                            new_remote_address: None,
                          });
                        }
                        std::io::ErrorKind::PermissionDenied => {
                          return Ok(ResponseData {
                            request: Some(request),
                            response: None,
                            response_status: Some(StatusCode::FORBIDDEN),
                            response_headers: None,
                            new_remote_address: None,
                          });
                        }
                        _ => Err(err)?,
                      },
                    };

                    // Construct a boxed body
                    #[cfg(feature = "runtime-monoio")]
                    let file_stream = MonoioFileStreamNoSpawn::new(file, Some(range_begin), Some(range_end + 1));
                    #[cfg(feature = "runtime-tokio")]
                    let file_stream = {
                      let mut file = file;

                      // Seek and limit the file reader
                      file.seek(SeekFrom::Start(range_begin)).await?;
                      let file_limited = file.take(content_length);

                      // Use BufReader for better performance.
                      let file_bufreader = BufReader::with_capacity(12800, file_limited);

                      // Create a reader stream
                      ReaderStream::new(file_bufreader)
                    };
                    let stream_body = StreamBody::new(file_stream.map_ok(Frame::data));
                    let boxed_body = stream_body.boxed();

                    response_builder.body(boxed_body)?
                  }
                };

                return Ok(ResponseData {
                  request: Some(request),
                  response: Some(response),
                  response_status: None,
                  response_headers: None,
                  new_remote_address: None,
                });
              } else {
                let mut header_map = HeaderMap::new();
                header_map.insert(header::VARY, HeaderValue::from_static(vary));
                header_map.insert(
                  header::CONTENT_RANGE,
                  HeaderValue::from_str(&format!("bytes */{file_length}"))?,
                );
                return Ok(ResponseData {
                  request: Some(request),
                  response: None,
                  response_status: Some(StatusCode::RANGE_NOT_SATISFIABLE),
                  response_headers: Some(header_map),
                  new_remote_address: None,
                });
              }
            } else {
              // Handle full file response (no range request)

              // Initialize compression flags
              let mut use_gzip = false;
              let mut use_deflate = false;
              let mut use_brotli = false;
              let mut use_zstd = false;

              // Determine the appropriate compression algorithm based on Accept-Encoding
              if compression_possible {
                // Get User-Agent for browser compatibility checks
                let user_agent = match request.headers().get(header::USER_AGENT) {
                  Some(user_agent_value) => user_agent_value.to_str().unwrap_or_default(),
                  None => "",
                };

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
                if !(content_type_option == Some("text/html".to_string())
                  && (is_netscape_4_broken_html_compression || is_w3m_broken_html_compression))
                  && !is_netscape_4_broken_compression
                {
                  // Get Accept-Encoding header to determine supported compression algorithms
                  let accept_encoding = match request.headers().get(header::ACCEPT_ENCODING) {
                    Some(header_value) => header_value.to_str().unwrap_or_default(),
                    None => "",
                  };

                  // Parse Accept-Encoding header to select the best compression method
                  // Check for supported compression algorithms in order of preference
                  if accept_encoding.contains("br") {
                    use_brotli = true;
                  }
                  if (enable_precompression || !use_brotli) && accept_encoding.contains("zstd") {
                    use_zstd = true;
                  }
                  if (enable_precompression || !(use_brotli || use_zstd)) && accept_encoding.contains("deflate") {
                    use_deflate = true;
                  }
                  if (enable_precompression || !(use_brotli || use_zstd || use_deflate))
                    && accept_encoding.contains("gzip")
                  {
                    use_gzip = true;
                  }
                }
              }

              // Handle precompression
              if enable_precompression {
                // Find the precompressed file
                let mut extensions = Vec::new();
                if use_brotli {
                  extensions.push("br");
                }
                if use_zstd {
                  extensions.push("zst");
                }
                if use_deflate {
                  extensions.push("deflate");
                }
                if use_gzip {
                  extensions.push("gz");
                }
                for extension in extensions {
                  let mut joined_pathbuf_with_extension = joined_pathbuf.clone();
                  joined_pathbuf_with_extension.set_extension(
                    format!(
                      "{}.{}",
                      joined_pathbuf
                        .extension()
                        .map_or(OsStr::new(""), |ext| ext)
                        .to_string_lossy(),
                      extension
                    )
                    .trim_matches('.'),
                  );
                  // Monoio's `fs` doesn't expose `metadata()` on Windows, so we have to spawn a blocking task to obtain the metadata on this platform
                  #[cfg(any(feature = "runtime-tokio", all(feature = "runtime-monoio", unix)))]
                  let metadata_obt = fs::metadata(&joined_pathbuf_with_extension).await;
                  #[cfg(all(feature = "runtime-monoio", windows))]
                  let metadata_obt = {
                    let joined_pathbuf_with_extension = joined_pathbuf_with_extension.clone();
                    monoio::spawn_blocking(move || std::fs::metadata(joined_pathbuf_with_extension))
                      .await
                      .unwrap_or(Err(std::io::Error::other(
                        "Can't spawn a blocking task to obtain the file metadata",
                      )))
                  };
                  if let Ok(metadata_obt_ok) = metadata_obt {
                    if metadata_obt_ok.is_file() {
                      joined_pathbuf = joined_pathbuf_with_extension;
                      metadata = metadata_obt_ok;
                      break;
                    }
                  }
                  match extension {
                    "br" => {
                      use_brotli = false;
                    }
                    "zst" => {
                      use_zstd = false;
                    }
                    "deflate" => {
                      use_deflate = false;
                    }
                    "gz" => {
                      use_gzip = false;
                    }
                    _ => {}
                  }
                }
              }

              // Get request method and file size
              let request_method = request.method();
              let content_length = metadata.len();

              // Build full file response
              let mut response_builder = Response::builder()
                .status(StatusCode::OK)
                .header(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));

              // Include ETag in response with suffix based on compression method
              if let Some(etag) = etag_option {
                if use_brotli {
                  response_builder = response_builder.header(header::ETAG, format!("W/\"{etag}-br\""));
                } else if use_zstd {
                  response_builder = response_builder.header(header::ETAG, format!("W/\"{etag}-zstd\""));
                } else if use_deflate {
                  response_builder = response_builder.header(header::ETAG, format!("W/\"{etag}-deflate\""));
                } else if use_gzip {
                  response_builder = response_builder.header(header::ETAG, format!("W/\"{etag}-gzip\""));
                } else {
                  // Uncompressed content
                  response_builder = response_builder.header(header::ETAG, format!("W/\"{etag}\""));
                }
              }

              response_builder = response_builder.header(header::VARY, vary);

              if let Some(content_type) = content_type_option {
                response_builder = response_builder.header(header::CONTENT_TYPE, content_type);
              }

              if let Some(cache_control) = cache_control {
                response_builder = response_builder.header(header::CACHE_CONTROL, cache_control);
              }

              // Set appropriate Content-Encoding header based on compression method
              if use_brotli {
                response_builder = response_builder.header(header::CONTENT_ENCODING, HeaderValue::from_static("br"));
              } else if use_zstd {
                response_builder = response_builder.header(header::CONTENT_ENCODING, HeaderValue::from_static("zstd"));
              } else if use_deflate {
                response_builder =
                  response_builder.header(header::CONTENT_ENCODING, HeaderValue::from_static("deflate"));
              } else if use_gzip {
                response_builder = response_builder.header(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
              } else {
                // Only include Content-Length for uncompressed responses
                // Content-Length header + HTTP compression = broken HTTP responses!
                response_builder = response_builder.header(header::CONTENT_LENGTH, content_length);
              }

              // Create the response based on the HTTP method
              let response = match request_method {
                // HEAD requests only need headers, no body
                &Method::HEAD => response_builder.body(Empty::new().map_err(|e| match e {}).boxed())?,
                // For GET and POST, include the file content
                _ => {
                  // Open file for reading
                  let file = match fs::File::open(joined_pathbuf).await {
                    Ok(file) => file,
                    Err(err) => match err.kind() {
                      std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory => {
                        return Ok(ResponseData {
                          request: Some(request),
                          response: None,
                          response_status: Some(StatusCode::NOT_FOUND),
                          response_headers: None,
                          new_remote_address: None,
                        });
                      }
                      std::io::ErrorKind::PermissionDenied => {
                        return Ok(ResponseData {
                          request: Some(request),
                          response: None,
                          response_status: Some(StatusCode::FORBIDDEN),
                          response_headers: None,
                          new_remote_address: None,
                        });
                      }
                      _ => Err(err)?,
                    },
                  };

                  // Create a file stream.
                  #[cfg(feature = "runtime-monoio")]
                  let file_stream = MonoioFileStreamNoSpawn::new(file, None, Some(content_length));
                  #[cfg(feature = "runtime-tokio")]
                  let file_stream = ReaderStream::new(BufReader::with_capacity(12800, file));

                  // Create the appropriate response body based on compression method, if precompression is disabled
                  let boxed_body = if !enable_precompression && use_brotli {
                    // Wrap the stream as a `AsyncRead`
                    let file_bufreader = StreamReader::new(file_stream);

                    // Use Brotli compression with moderate quality (4) for good compression/speed balance
                    // Also, set the window size and block size to optimize compression, and reduce memory usage
                    let reader_stream = ReaderStream::with_capacity(
                      BrotliEncoder::with_params(
                        file_bufreader,
                        EncoderParams::default()
                          .quality(Level::Precise(4))
                          .window_size(17)
                          .block_size(18),
                      ),
                      COMPRESSED_STREAM_READER_BUFFER_SIZE,
                    );
                    let stream_body = StreamBody::new(reader_stream.map_ok(Frame::data));
                    stream_body.boxed()
                  } else if !enable_precompression && use_zstd {
                    // Wrap the stream as a `AsyncRead`
                    let file_bufreader = StreamReader::new(file_stream);

                    // Limit the Zstandard window size to 128K (2^17 bytes) to support many HTTP clients
                    // Also, set the size of the initial probe table to reduce memory usage
                    let reader_stream = ReaderStream::with_capacity(
                      ZstdEncoder::with_quality_and_params(
                        file_bufreader,
                        Level::Default,
                        &[CParameter::window_log(17), CParameter::hash_log(10)],
                      ),
                      COMPRESSED_STREAM_READER_BUFFER_SIZE,
                    );
                    let stream_body = StreamBody::new(reader_stream.map_ok(Frame::data));
                    stream_body.boxed()
                  } else if !enable_precompression && use_deflate {
                    // Wrap the stream as a `AsyncRead`
                    let file_bufreader = StreamReader::new(file_stream);

                    let reader_stream = ReaderStream::with_capacity(
                      DeflateEncoder::new(file_bufreader),
                      COMPRESSED_STREAM_READER_BUFFER_SIZE,
                    );
                    let stream_body = StreamBody::new(reader_stream.map_ok(Frame::data));
                    stream_body.boxed()
                  } else if !enable_precompression && use_gzip {
                    // Wrap the stream as a `AsyncRead`
                    let file_bufreader = StreamReader::new(file_stream);

                    let reader_stream = ReaderStream::with_capacity(
                      GzipEncoder::new(file_bufreader),
                      COMPRESSED_STREAM_READER_BUFFER_SIZE,
                    );
                    let stream_body = StreamBody::new(reader_stream.map_ok(Frame::data));
                    stream_body.boxed()
                  } else {
                    let stream_body = StreamBody::new(file_stream.map_ok(Frame::data));
                    stream_body.boxed()
                  };

                  response_builder.body(boxed_body)?
                }
              };

              return Ok(ResponseData {
                request: Some(request),
                response: Some(response),
                response_status: None,
                response_headers: None,
                new_remote_address: None,
              });
            }
          } else if metadata.is_dir() {
            // Handle directory requests

            // Check if directory listing is enabled in config (defaults to false)
            if get_value!("directory_listing", config)
              .and_then(|v| v.as_bool())
              .unwrap_or(false)
            {
              // Look for a description file in the directory
              let joined_maindesc_pathbuf = joined_pathbuf.join(".maindesc");
              // Read the directory contents (using blocking task on Windows and with Monoio)
              #[cfg(feature = "runtime-monoio")]
              let directory_result = monoio::spawn_blocking(move || std::fs::read_dir(joined_pathbuf))
                .await
                .unwrap_or(Err(std::io::Error::other(
                  "Can't spawn a blocking task to read the directory",
                )));
              #[cfg(feature = "runtime-tokio")]
              let directory_result = fs::read_dir(joined_pathbuf).await;
              let directory = match directory_result {
                Ok(directory) => directory,
                Err(err) => match err.kind() {
                  std::io::ErrorKind::NotFound => {
                    return Ok(ResponseData {
                      request: Some(request),
                      response: None,
                      response_status: Some(StatusCode::NOT_FOUND),
                      response_headers: None,
                      new_remote_address: None,
                    });
                  }
                  std::io::ErrorKind::PermissionDenied => {
                    return Ok(ResponseData {
                      request: Some(request),
                      response: None,
                      response_status: Some(StatusCode::FORBIDDEN),
                      response_headers: None,
                      new_remote_address: None,
                    });
                  }
                  _ => Err(err)?,
                },
              };

              let description = (fs::read(joined_maindesc_pathbuf).await)
                .ok()
                .and_then(|d| String::from_utf8(d).ok());

              let directory_listing_html =
                generate_directory_listing(directory, original_request_path, description).await?;
              let content_length: Option<u64> = directory_listing_html.len().try_into().ok();

              let mut response_builder = Response::builder().status(StatusCode::OK);

              if let Some(content_length) = content_length {
                response_builder = response_builder.header(header::CONTENT_LENGTH, content_length)
              }
              response_builder = response_builder.header(header::CONTENT_TYPE, "text/html");

              let response = response_builder.body(
                Full::new(Bytes::from(directory_listing_html))
                  .map_err(|e| match e {})
                  .boxed(),
              )?;

              return Ok(ResponseData {
                request: Some(request),
                response: Some(response),
                response_status: None,
                response_headers: None,
                new_remote_address: None,
              });
            } else {
              // Directory listing is disabled
              return Ok(ResponseData {
                request: Some(request),
                response: None,
                response_status: Some(StatusCode::FORBIDDEN),
                response_headers: None,
                new_remote_address: None,
              });
            }
          } else {
            // Static file serving can't be used on anything that's not a file or directory in Ferron
            return Ok(ResponseData {
              request: Some(request),
              response: None,
              response_status: Some(StatusCode::NOT_IMPLEMENTED),
              response_headers: None,
              new_remote_address: None,
            });
          }
        }
        Err(err) => match err.kind() {
          std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory => {
            return Ok(ResponseData {
              request: Some(request),
              response: None,
              response_status: Some(StatusCode::NOT_FOUND),
              response_headers: None,
              new_remote_address: None,
            });
          }
          std::io::ErrorKind::PermissionDenied => {
            return Ok(ResponseData {
              request: Some(request),
              response: None,
              response_status: Some(StatusCode::FORBIDDEN),
              response_headers: None,
              new_remote_address: None,
            });
          }
          _ => Err(err)?,
        },
      }
    }

    Ok(ResponseData {
      request: Some(request),
      response: None,
      response_status: None,
      response_headers: None,
      new_remote_address: None,
    })
  }
}
