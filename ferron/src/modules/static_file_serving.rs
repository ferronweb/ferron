use std::error::Error;
use std::fmt::Write;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use async_compression::tokio::bufread::{BrotliEncoder, DeflateEncoder, GzipEncoder, ZstdEncoder};
use async_trait::async_trait;
use chrono::offset::Local;
use chrono::DateTime;
use ferron_common::{
  ErrorLogger, HyperResponse, RequestData, ResponseData, ServerConfigRoot, ServerModule,
  ServerModuleHandlers, SocketData,
};
use ferron_common::{HyperUpgraded, WithRuntime};
use futures_util::TryStreamExt;
use hashlink::LruCache;
use http_body_util::{BodyExt, Empty, Full, StreamBody};
use hyper::body::Bytes;
use hyper::{body::Frame, Response, StatusCode};
use hyper::{header, HeaderMap, Method};
use hyper_tungstenite::HyperWebsocket;
use sha2::{Digest, Sha256};
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncSeekExt, BufReader};
use tokio::runtime::Handle;
use tokio::sync::RwLock;
use tokio_util::io::ReaderStream;

use crate::ferron_util::generate_directory_listing::generate_directory_listing;
use crate::ferron_util::ttl_cache::TtlCache;

pub fn server_module_init(
) -> Result<Box<dyn ServerModule + Send + Sync>, Box<dyn Error + Send + Sync>> {
  let pathbuf_cache = Arc::new(RwLock::new(TtlCache::new(Duration::from_millis(100))));
  let etag_cache = Arc::new(RwLock::new(LruCache::new(1000)));
  Ok(Box::new(StaticFileServingModule::new(
    pathbuf_cache,
    etag_cache,
  )))
}

struct StaticFileServingModule {
  pathbuf_cache: Arc<RwLock<TtlCache<String, PathBuf>>>,
  etag_cache: Arc<RwLock<LruCache<String, String>>>,
}

impl StaticFileServingModule {
  fn new(
    pathbuf_cache: Arc<RwLock<TtlCache<String, PathBuf>>>,
    etag_cache: Arc<RwLock<LruCache<String, String>>>,
  ) -> Self {
    StaticFileServingModule {
      pathbuf_cache,
      etag_cache,
    }
  }
}

impl ServerModule for StaticFileServingModule {
  fn get_handlers(&self, handle: Handle) -> Box<dyn ServerModuleHandlers + Send> {
    Box::new(StaticFileServingModuleHandlers {
      pathbuf_cache: self.pathbuf_cache.clone(),
      etag_cache: self.etag_cache.clone(),
      handle,
    })
  }
}
struct StaticFileServingModuleHandlers {
  pathbuf_cache: Arc<RwLock<TtlCache<String, PathBuf>>>,
  etag_cache: Arc<RwLock<LruCache<String, String>>>,
  handle: Handle,
}

fn parse_range_header(range_str: &str, default_end: u64) -> Option<(u64, u64)> {
  if let Some(range_part) = range_str.strip_prefix("bytes=") {
    let parts: Vec<&str> = range_part.split('-').collect();
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

#[async_trait]
impl ServerModuleHandlers for StaticFileServingModuleHandlers {
  async fn request_handler(
    &mut self,
    request: RequestData,
    config: &ServerConfigRoot,
    _socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    WithRuntime::new(self.handle.clone(), async move {
      if let Some(wwwroot) = config.get("wwwroot").as_str() {
        let hyper_request = request.get_hyper_request();
        let request_path = hyper_request.uri().path();
        //let request_query = hyper_request.uri().query();
        let mut request_path_bytes = request_path.bytes();
        if request_path_bytes.len() < 1 || request_path_bytes.nth(0) != Some(b'/') {
          return Ok(
            ResponseData::builder(request)
              .status(StatusCode::BAD_REQUEST)
              .build(),
          );
        }

        let cache_key = format!(
          "{}{}{}",
          match config.get("ip").as_str() {
            Some(ip) => format!("{}-", ip),
            None => String::from(""),
          },
          match config.get("domain").as_str() {
            Some(domain) => format!("{}-", domain),
            None => String::from(""),
          },
          request_path
        );

        let rwlock_read = self.pathbuf_cache.read().await;
        let joined_pathbuf_option = rwlock_read.get(&cache_key);
        drop(rwlock_read);

        let joined_pathbuf_cached = joined_pathbuf_option.is_some();
        let mut joined_pathbuf = match joined_pathbuf_option {
          Some(joined_pathbuf) => joined_pathbuf,
          None => {
            let path = Path::new(wwwroot);
            let mut relative_path = &request_path[1..];
            while relative_path.as_bytes().first().copied() == Some(b'/') {
              relative_path = &relative_path[1..];
            }

            let decoded_relative_path = match urlencoding::decode(relative_path) {
              Ok(path) => path.to_string(),
              Err(_) => {
                return Ok(
                  ResponseData::builder(request)
                    .status(StatusCode::BAD_REQUEST)
                    .build(),
                );
              }
            };

            path.join(decoded_relative_path)
          }
        };

        match fs::metadata(&joined_pathbuf).await {
          Ok(mut metadata) => {
            if !joined_pathbuf_cached {
              if metadata.is_dir() {
                let indexes = vec!["index.html", "index.htm", "index.xhtml"];
                for index in indexes {
                  let temp_joined_pathbuf = joined_pathbuf.join(index);
                  match fs::metadata(&temp_joined_pathbuf).await {
                    Ok(temp_metadata) => {
                      if temp_metadata.is_file() {
                        metadata = temp_metadata;
                        joined_pathbuf = temp_joined_pathbuf;
                        break;
                      }
                    }
                    Err(err) => match err.kind() {
                      tokio::io::ErrorKind::NotFound | tokio::io::ErrorKind::NotADirectory => {
                        continue;
                      }
                      tokio::io::ErrorKind::PermissionDenied => {
                        return Ok(
                          ResponseData::builder(request)
                            .status(StatusCode::FORBIDDEN)
                            .build(),
                        );
                      }
                      _ => Err(err)?,
                    },
                  };
                }
              }
              let mut rwlock_write = self.pathbuf_cache.write().await;
              rwlock_write.cleanup();
              rwlock_write.insert(cache_key, joined_pathbuf.clone());
              drop(rwlock_write);
            }

            if metadata.is_file() {
              // Handle ETags
              let mut etag_option = None;
              if config.get("enableETag").as_bool() != Some(false) {
                let etag_cache_key = format!(
                  "{}-{}-{}",
                  joined_pathbuf.to_string_lossy(),
                  metadata.len(),
                  match metadata.modified() {
                    Ok(mtime) => {
                      let datetime: DateTime<Local> = mtime.into();
                      datetime.format("%Y-%m-%d %H:%M:%S").to_string()
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
                    let etag_cache_key_clone = etag_cache_key.clone();
                    let etag = tokio::task::spawn_blocking(move || {
                      let mut hasher = Sha256::new();
                      hasher.update(etag_cache_key_clone);
                      hasher
                        .finalize()
                        .iter()
                        .fold(String::new(), |mut output, b| {
                          let _ = write!(output, "{b:02x}");
                          output
                        })
                    })
                    .await?;

                    let mut rwlock_write = self.etag_cache.write().await;
                    rwlock_write.insert(etag_cache_key, etag.clone());
                    drop(rwlock_write);

                    etag
                  }
                };

                if let Some(if_none_match_value) =
                  hyper_request.headers().get(header::IF_NONE_MATCH)
                {
                  match if_none_match_value.to_str() {
                    Ok(if_none_match) => {
                      if if_none_match == etag {
                        return Ok(
                          ResponseData::builder(request)
                            .response(
                              Response::builder()
                                .status(StatusCode::NOT_MODIFIED)
                                .header(header::ETAG, etag)
                                .body(Empty::new().map_err(|e| match e {}).boxed())?,
                            )
                            .build(),
                        );
                      }
                    }
                    Err(_) => {
                      return Ok(
                        ResponseData::builder(request)
                          .status(StatusCode::BAD_REQUEST)
                          .build(),
                      )
                    }
                  }
                }

                if let Some(if_match_value) = hyper_request.headers().get(header::IF_MATCH) {
                  match if_match_value.to_str() {
                    Ok(if_match) => {
                      if if_match != "*" && if_match != etag {
                        let mut header_map = HeaderMap::new();
                        header_map.insert(header::ETAG, if_match_value.clone());
                        return Ok(
                          ResponseData::builder(request)
                            .status(StatusCode::PRECONDITION_FAILED)
                            .headers(header_map)
                            .build(),
                        );
                      }
                    }
                    Err(_) => {
                      return Ok(
                        ResponseData::builder(request)
                          .status(StatusCode::BAD_REQUEST)
                          .build(),
                      )
                    }
                  }
                }
                etag_option = Some(etag);
              }

              let content_type_option = new_mime_guess::from_path(&joined_pathbuf)
                .first()
                .map(|mime_type| mime_type.to_string());

              let range_header = match hyper_request.headers().get(header::RANGE) {
                Some(value) => match value.to_str() {
                  Ok(value) => Some(value),
                  Err(_) => {
                    return Ok(
                      ResponseData::builder(request)
                        .status(StatusCode::BAD_REQUEST)
                        .build(),
                    )
                  }
                },
                None => None,
              };

              if let Some(range_header) = range_header {
                let file_length = metadata.len();
                if file_length == 0 {
                  return Ok(
                    ResponseData::builder(request)
                      .status(StatusCode::RANGE_NOT_SATISFIABLE)
                      .build(),
                  );
                }
                if let Some((range_begin, range_end)) =
                  parse_range_header(range_header, file_length - 1)
                {
                  if range_end > file_length - 1
                    || range_begin > file_length - 1
                    || range_begin > range_end
                  {
                    return Ok(
                      ResponseData::builder(request)
                        .status(StatusCode::RANGE_NOT_SATISFIABLE)
                        .build(),
                    );
                  }

                  let request_method = hyper_request.method();
                  let content_length = range_end - range_begin + 1;

                  // Build response
                  let mut response_builder = Response::builder()
                    .status(StatusCode::PARTIAL_CONTENT)
                    .header(header::CONTENT_LENGTH, content_length)
                    .header(
                      header::CONTENT_RANGE,
                      format!("bytes {}-{}/{}", range_begin, range_end, file_length),
                    );

                  if let Some(etag) = etag_option {
                    response_builder = response_builder.header(header::ETAG, etag);
                  }

                  if let Some(content_type) = content_type_option {
                    response_builder = response_builder.header(header::CONTENT_TYPE, content_type);
                  }

                  let response = match request_method {
                    &Method::HEAD => {
                      response_builder.body(Empty::new().map_err(|e| match e {}).boxed())?
                    }
                    _ => {
                      // Open file for reading
                      let mut file = match fs::File::open(joined_pathbuf).await {
                        Ok(file) => file,
                        Err(err) => match err.kind() {
                          tokio::io::ErrorKind::NotFound | tokio::io::ErrorKind::NotADirectory => {
                            return Ok(
                              ResponseData::builder(request)
                                .status(StatusCode::NOT_FOUND)
                                .build(),
                            );
                          }
                          tokio::io::ErrorKind::PermissionDenied => {
                            return Ok(
                              ResponseData::builder(request)
                                .status(StatusCode::FORBIDDEN)
                                .build(),
                            );
                          }
                          _ => Err(err)?,
                        },
                      };

                      // Seek and limit the file reader
                      file.seek(SeekFrom::Start(range_begin)).await?;
                      let file_limited = file.take(content_length);

                      // Use BufReader for better performance.
                      let file_bufreader = BufReader::with_capacity(12800, file_limited);

                      // Construct a boxed body
                      let reader_stream = ReaderStream::new(file_bufreader);
                      let stream_body = StreamBody::new(reader_stream.map_ok(Frame::data));
                      let boxed_body = stream_body.boxed();

                      response_builder.body(boxed_body)?
                    }
                  };

                  return Ok(ResponseData::builder(request).response(response).build());
                } else {
                  return Ok(
                    ResponseData::builder(request)
                      .status(StatusCode::RANGE_NOT_SATISFIABLE)
                      .build(),
                  );
                }
              } else {
                let mut use_gzip = false;
                let mut use_deflate = false;
                let mut use_brotli = false;
                let mut use_zstd = false;

                if config.get("enableCompression").as_bool() != Some(false) {
                  // A hard-coded list of non-compressible file extension
                  let non_compressible_file_extensions = vec![
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
                  ];
                  let file_extension = joined_pathbuf
                    .extension()
                    .map_or_else(|| "".to_string(), |ext| ext.to_string_lossy().to_string());
                  let file_extension_compressible =
                    !non_compressible_file_extensions.contains(&(&file_extension as &str));

                  let user_agent = match hyper_request.headers().get(header::USER_AGENT) {
                    Some(user_agent_value) => user_agent_value.to_str().unwrap_or_default(),
                    None => "",
                  };

                  if metadata.len() > 256 && file_extension_compressible {
                    // Some web browsers have broken HTTP compression handling
                    let is_netscape_4_broken_html_compression =
                      user_agent.starts_with("Mozilla/4.");
                    let is_netscape_4_broken_compression =
                      match user_agent.strip_prefix("Mozilla/4.") {
                        Some(stripped_user_agent) => matches!(
                          stripped_user_agent.chars().nth(0),
                          Some('6') | Some('7') | Some('8')
                        ),
                        None => false,
                      };
                    let is_w3m_broken_html_compression = user_agent.starts_with("w3m/");
                    if !(content_type_option == Some("text/html".to_string())
                      && (is_netscape_4_broken_html_compression || is_w3m_broken_html_compression))
                      && !is_netscape_4_broken_compression
                    {
                      let accept_encoding =
                        match hyper_request.headers().get(header::ACCEPT_ENCODING) {
                          Some(header_value) => header_value.to_str().unwrap_or_default(),
                          None => "",
                        };

                      // Checking the Accept-Encoding header naively...
                      if accept_encoding.contains("br") {
                        use_brotli = true;
                      } else if accept_encoding.contains("zstd") {
                        use_zstd = true;
                      } else if accept_encoding.contains("deflate") {
                        use_deflate = true;
                      } else if accept_encoding.contains("gzip") {
                        use_gzip = true;
                      }
                    }
                  }
                }

                let request_method = hyper_request.method();
                let content_length = metadata.len();

                // Build response
                let mut response_builder = Response::builder()
                  .status(StatusCode::OK)
                  .header(header::ACCEPT_RANGES, "bytes");

                if let Some(etag) = etag_option {
                  response_builder = response_builder.header(header::ETAG, etag);
                }

                if let Some(content_type) = content_type_option {
                  response_builder = response_builder.header(header::CONTENT_TYPE, content_type);
                }

                if use_brotli {
                  response_builder = response_builder.header(header::CONTENT_ENCODING, "br");
                } else if use_zstd {
                  response_builder = response_builder.header(header::CONTENT_ENCODING, "zstd");
                } else if use_deflate {
                  response_builder = response_builder.header(header::CONTENT_ENCODING, "deflate");
                } else if use_gzip {
                  response_builder = response_builder.header(header::CONTENT_ENCODING, "gzip");
                } else {
                  // Content-Length header + HTTP compression = broken HTTP responses!
                  response_builder =
                    response_builder.header(header::CONTENT_LENGTH, content_length);
                }

                let response = match request_method {
                  &Method::HEAD => {
                    response_builder.body(Empty::new().map_err(|e| match e {}).boxed())?
                  }
                  _ => {
                    // Open file for reading
                    let file = match fs::File::open(joined_pathbuf).await {
                      Ok(file) => file,
                      Err(err) => match err.kind() {
                        tokio::io::ErrorKind::NotFound | tokio::io::ErrorKind::NotADirectory => {
                          return Ok(
                            ResponseData::builder(request)
                              .status(StatusCode::NOT_FOUND)
                              .build(),
                          );
                        }
                        tokio::io::ErrorKind::PermissionDenied => {
                          return Ok(
                            ResponseData::builder(request)
                              .status(StatusCode::FORBIDDEN)
                              .build(),
                          );
                        }
                        _ => Err(err)?,
                      },
                    };

                    // Use BufReader for better performance.
                    let file_bufreader = BufReader::with_capacity(12800, file);

                    // Construct a boxed body
                    let boxed_body = if use_brotli {
                      let reader_stream = ReaderStream::new(BrotliEncoder::new(file_bufreader));
                      let stream_body = StreamBody::new(reader_stream.map_ok(Frame::data));
                      stream_body.boxed()
                    } else if use_zstd {
                      let reader_stream = ReaderStream::new(ZstdEncoder::new(file_bufreader));
                      let stream_body = StreamBody::new(reader_stream.map_ok(Frame::data));
                      stream_body.boxed()
                    } else if use_deflate {
                      let reader_stream = ReaderStream::new(DeflateEncoder::new(file_bufreader));
                      let stream_body = StreamBody::new(reader_stream.map_ok(Frame::data));
                      stream_body.boxed()
                    } else if use_gzip {
                      let reader_stream = ReaderStream::new(GzipEncoder::new(file_bufreader));
                      let stream_body = StreamBody::new(reader_stream.map_ok(Frame::data));
                      stream_body.boxed()
                    } else {
                      let reader_stream = ReaderStream::new(file_bufreader);
                      let stream_body = StreamBody::new(reader_stream.map_ok(Frame::data));
                      stream_body.boxed()
                    };

                    response_builder.body(boxed_body)?
                  }
                };

                return Ok(ResponseData::builder(request).response(response).build());
              }
            } else if metadata.is_dir() {
              if config.get("enableDirectoryListing").as_bool() == Some(true) {
                let joined_maindesc_pathbuf = joined_pathbuf.join(".maindesc");
                let directory = match fs::read_dir(joined_pathbuf).await {
                  Ok(directory) => directory,
                  Err(err) => match err.kind() {
                    tokio::io::ErrorKind::NotFound => {
                      return Ok(
                        ResponseData::builder(request)
                          .status(StatusCode::NOT_FOUND)
                          .build(),
                      );
                    }
                    tokio::io::ErrorKind::PermissionDenied => {
                      return Ok(
                        ResponseData::builder(request)
                          .status(StatusCode::FORBIDDEN)
                          .build(),
                      );
                    }
                    _ => Err(err)?,
                  },
                };

                let description = match fs::read_to_string(joined_maindesc_pathbuf).await {
                  Ok(contents) => Some(contents),
                  Err(_) => None,
                };

                let directory_listing_html =
                  generate_directory_listing(directory, request_path, description).await?;
                let content_length: Option<u64> = match directory_listing_html.len().try_into() {
                  Ok(content_length) => Some(content_length),
                  Err(_) => None,
                };

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

                return Ok(ResponseData::builder(request).response(response).build());
              } else {
                return Ok(
                  ResponseData::builder(request)
                    .status(StatusCode::FORBIDDEN)
                    .build(),
                );
              }
            } else {
              return Ok(
                ResponseData::builder(request)
                  .status(StatusCode::NOT_IMPLEMENTED)
                  .build(),
              );
            }
          }
          Err(err) => match err.kind() {
            tokio::io::ErrorKind::NotFound | tokio::io::ErrorKind::NotADirectory => {
              return Ok(
                ResponseData::builder(request)
                  .status(StatusCode::NOT_FOUND)
                  .build(),
              );
            }
            tokio::io::ErrorKind::PermissionDenied => {
              return Ok(
                ResponseData::builder(request)
                  .status(StatusCode::FORBIDDEN)
                  .build(),
              );
            }
            _ => Err(err)?,
          },
        }
      }

      Ok(ResponseData::builder(request).build())
    })
    .await
  }

  async fn proxy_request_handler(
    &mut self,
    request: RequestData,
    _config: &ServerConfigRoot,
    _socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    Ok(ResponseData::builder(request).build())
  }

  async fn response_modifying_handler(
    &mut self,
    response: HyperResponse,
  ) -> Result<HyperResponse, Box<dyn Error + Send + Sync>> {
    Ok(response)
  }

  async fn proxy_response_modifying_handler(
    &mut self,
    response: HyperResponse,
  ) -> Result<HyperResponse, Box<dyn Error + Send + Sync>> {
    Ok(response)
  }

  async fn connect_proxy_request_handler(
    &mut self,
    _upgraded_request: HyperUpgraded,
    _connect_address: &str,
    _config: &ServerConfigRoot,
    _socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    Ok(())
  }

  fn does_connect_proxy_requests(&mut self) -> bool {
    false
  }

  async fn websocket_request_handler(
    &mut self,
    _websocket: HyperWebsocket,
    _uri: &hyper::Uri,
    _config: &ServerConfigRoot,
    _socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    Ok(())
  }

  fn does_websocket_requests(
    &mut self,
    _config: &ServerConfigRoot,
    _socket_data: &SocketData,
  ) -> bool {
    false
  }
}
