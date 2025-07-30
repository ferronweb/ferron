use std::collections::HashMap;
use std::env;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::future::Either;
use futures_util::stream::{StreamExt, TryStreamExt};
use hashlink::LinkedHashMap;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, StreamBody};
use httparse::EMPTY_HEADER;
use hyper::body::Frame;
use hyper::{header, Request, Response, StatusCode};
#[cfg(feature = "runtime-monoio")]
use monoio::io::IntoPollIo;
#[cfg(feature = "runtime-monoio")]
use monoio::net::TcpStream;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
#[cfg(feature = "runtime-tokio")]
use tokio::net::TcpStream;
use tokio::sync::RwLock;
use tokio_util::codec::{FramedRead, FramedWrite};
use tokio_util::io::{ReaderStream, SinkWriter, StreamReader};

use crate::util::cgi::CgiResponse;
use crate::util::fcgi::{
  construct_fastcgi_name_value_pair, construct_fastcgi_record, FcgiDecodedData, FcgiDecoder, FcgiEncoder,
};
use crate::util::{Copier, ReadToEndFuture, SplitStreamByMapExt};
use ferron_common::config::ServerConfiguration;
use ferron_common::logging::ErrorLogger;
use ferron_common::modules::{Module, ModuleHandlers, ModuleLoader, RequestData, ResponseData, SocketData};
use ferron_common::util::{ModuleCache, TtlCache, SERVER_SOFTWARE};
use ferron_common::{get_entries, get_entries_for_validation, get_entry, get_value};

const MAX_RESPONSE_CHANNEL_CAPACITY: usize = 2;

/// A FastCGI module loader
#[allow(clippy::type_complexity)]
pub struct FcgiModuleLoader {
  cache: ModuleCache<FcgiModule>,
  path_cache: Arc<RwLock<TtlCache<String, (Option<PathBuf>, Option<String>)>>>,
}

impl Default for FcgiModuleLoader {
  fn default() -> Self {
    Self::new()
  }
}

impl FcgiModuleLoader {
  /// Creates a new module loader
  pub fn new() -> Self {
    Self {
      cache: ModuleCache::new(vec![]),
      path_cache: Arc::new(RwLock::new(TtlCache::new(Duration::from_millis(100)))),
    }
  }
}

impl ModuleLoader for FcgiModuleLoader {
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
          Ok(Arc::new(FcgiModule {
            path_cache: self.path_cache.clone(),
          }))
        })?,
    )
  }

  fn get_requirements(&self) -> Vec<&'static str> {
    vec!["fcgi", "fcgi_php"]
  }

  fn validate_configuration(
    &self,
    config: &ServerConfiguration,
    used_properties: &mut std::collections::HashSet<String>,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    if let Some(entries) = get_entries_for_validation!("fcgi", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `fcgi` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_string() && !entry.values[0].is_null() {
          Err(anyhow::anyhow!("The FastCGI server base URL must be a string"))?
        } else if !entry.props.get("pass").is_none_or(|v| v.is_bool()) {
          Err(anyhow::anyhow!("The FastCGI passing option must be boolean"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("fcgi_php", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `fcgi_php` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_string() && !entry.values[0].is_null() {
          Err(anyhow::anyhow!(
            "The PHP through FastCGI server base URL must be a string"
          ))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("fcgi_extension", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `fcgi_extension` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_string() {
          Err(anyhow::anyhow!("The FastCGI file extension must be a string"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("fcgi_environment", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 2 {
          Err(anyhow::anyhow!(
            "The `fcgi_environment` configuration property must have exactly two values"
          ))?
        } else if !entry.values[0].is_string() {
          Err(anyhow::anyhow!(
            "The FastCGI environment variable name must be a string"
          ))?
        } else if !entry.values[1].is_string() {
          Err(anyhow::anyhow!(
            "The FastCGI environment variable value must be a string"
          ))?
        }
      }
    };

    Ok(())
  }
}

/// A FastCGI module
#[allow(clippy::type_complexity)]
struct FcgiModule {
  path_cache: Arc<RwLock<TtlCache<String, (Option<PathBuf>, Option<String>)>>>,
}

impl Module for FcgiModule {
  fn get_module_handlers(&self) -> Box<dyn ModuleHandlers> {
    Box::new(FcgiModuleHandlers {
      path_cache: self.path_cache.clone(),
    })
  }
}

/// Handlers for the FastCGI module
#[allow(clippy::type_complexity)]
struct FcgiModuleHandlers {
  path_cache: Arc<RwLock<TtlCache<String, (Option<PathBuf>, Option<String>)>>>,
}

#[async_trait(?Send)]
impl ModuleHandlers for FcgiModuleHandlers {
  async fn request_handler(
    &mut self,
    request: Request<BoxBody<Bytes, std::io::Error>>,
    config: &ServerConfiguration,
    socket_data: &SocketData,
    error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    // Determine if the request is a forward proxy request
    let fastcgi_entry = get_entry!("fcgi", config);
    let fastcgi_php_entry = get_entry!("fcgi_php", config);

    let mut fastcgi_to = fastcgi_entry.and_then(|e| e.values.first()).and_then(|v| v.as_str());
    let mut fastcgi_pass = fastcgi_entry
      .and_then(|e| e.props.get("pass"))
      .and_then(|v| v.as_bool())
      .unwrap_or(true);

    if fastcgi_to.is_none() {
      fastcgi_to = fastcgi_php_entry
        .and_then(|e| e.values.first())
        .and_then(|v| v.as_str());
      fastcgi_pass = false;
    }
    let is_php = fastcgi_entry.is_none() && fastcgi_php_entry.is_some();

    if let Some(fastcgi_to) = fastcgi_to {
      let mut fastcgi_script_exts = Vec::new();

      if is_php {
        fastcgi_script_exts.push(".php");
      } else {
        let fcgi_script_exts_config = get_entries!("fcgi_extension", config);
        if let Some(fcgi_script_exts_obtained) = fcgi_script_exts_config {
          for fcgi_script_ext_config in fcgi_script_exts_obtained.inner.iter() {
            if let Some(fcgi_script_ext) = fcgi_script_ext_config.values.first().and_then(|v| v.as_str()) {
              fastcgi_script_exts.push(fcgi_script_ext);
            }
          }
        }
      }

      let fastcgi_path = if fastcgi_pass { Some("/") } else { None };

      let request_path = request.uri().path();
      let mut request_path_bytes = request_path.bytes();
      if request_path_bytes.len() < 1 || request_path_bytes.nth(0) != Some(b'/') {
        return Ok(ResponseData {
          request: Some(request),
          response: None,
          response_status: Some(StatusCode::BAD_REQUEST),
          response_headers: None,
          new_remote_address: None,
        });
      }

      let mut execute_pathbuf = None;
      let mut execute_path_info = None;
      let mut wwwroot_detected = None;

      if let Some(fastcgi_path) = fastcgi_path {
        let mut canonical_fastcgi_path: &str = fastcgi_path;
        if canonical_fastcgi_path.bytes().last() == Some(b'/') {
          canonical_fastcgi_path = &canonical_fastcgi_path[..(canonical_fastcgi_path.len() - 1)];
        }

        let request_path_with_slashes = match request_path == canonical_fastcgi_path {
          true => format!("{request_path}/"),
          false => request_path.to_string(),
        };
        if let Some(stripped_request_path) = request_path_with_slashes.strip_prefix(canonical_fastcgi_path) {
          let wwwroot = get_entry!("root", config)
            .and_then(|e| e.values.first())
            .and_then(|v| v.as_str())
            .unwrap_or("/nonexistent");

          let wwwroot_unknown = PathBuf::from(wwwroot);
          let wwwroot_pathbuf = match wwwroot_unknown.as_path().is_absolute() {
            true => wwwroot_unknown,
            false => {
              #[cfg(feature = "runtime-monoio")]
              let canonicalize_result = {
                let wwwroot_unknown = wwwroot_unknown.clone();
                monoio::spawn_blocking(move || std::fs::canonicalize(wwwroot_unknown))
                  .await
                  .unwrap_or(Err(std::io::Error::other(
                    "Can't spawn a blocking task to obtain the canonical webroot path",
                  )))
              };
              #[cfg(feature = "runtime-tokio")]
              let canonicalize_result = tokio::fs::canonicalize(&wwwroot_unknown).await;

              match canonicalize_result {
                Ok(pathbuf) => pathbuf,
                Err(_) => wwwroot_unknown,
              }
            }
          };
          wwwroot_detected = Some(wwwroot_pathbuf.clone());
          let wwwroot = wwwroot_pathbuf.as_path();

          let mut relative_path = &request_path[1..];
          while relative_path.as_bytes().first().copied() == Some(b'/') {
            relative_path = &relative_path[1..];
          }

          let decoded_relative_path = match urlencoding::decode(relative_path) {
            Ok(path) => path.to_string(),
            Err(_) => {
              return Ok(ResponseData {
                request: Some(request),
                response: None,
                response_status: Some(StatusCode::BAD_REQUEST),
                response_headers: None,
                new_remote_address: None,
              });
            }
          };

          let joined_pathbuf = wwwroot.join(decoded_relative_path);
          execute_pathbuf = Some(joined_pathbuf);
          execute_path_info = stripped_request_path.strip_prefix("/").map(|s| s.to_string());
        }
      }

      if execute_pathbuf.is_none() {
        if let Some(wwwroot) = get_entry!("root", config)
          .and_then(|e| e.values.first())
          .and_then(|v| v.as_str())
        {
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

          let wwwroot_unknown = PathBuf::from(wwwroot);
          let wwwroot_pathbuf = match wwwroot_unknown.as_path().is_absolute() {
            true => wwwroot_unknown,
            false => {
              #[cfg(feature = "runtime-monoio")]
              let canonicalize_result = {
                let wwwroot_unknown = wwwroot_unknown.clone();
                monoio::spawn_blocking(move || std::fs::canonicalize(wwwroot_unknown))
                  .await
                  .unwrap_or(Err(std::io::Error::other(
                    "Can't spawn a blocking task to obtain the canonical webroot path",
                  )))
              };
              #[cfg(feature = "runtime-tokio")]
              let canonicalize_result = tokio::fs::canonicalize(&wwwroot_unknown).await;

              match canonicalize_result {
                Ok(pathbuf) => pathbuf,
                Err(_) => wwwroot_unknown,
              }
            }
          };
          wwwroot_detected = Some(wwwroot_pathbuf.clone());
          let wwwroot = wwwroot_pathbuf.as_path();

          let read_rwlock = self.path_cache.read().await;
          let (execute_pathbuf_got, execute_path_info_got) = match read_rwlock.get(&cache_key) {
            Some(data) => {
              drop(read_rwlock);
              data
            }
            None => {
              drop(read_rwlock);
              let mut relative_path = &request_path[1..];
              while relative_path.as_bytes().first().copied() == Some(b'/') {
                relative_path = &relative_path[1..];
              }

              let decoded_relative_path = match urlencoding::decode(relative_path) {
                Ok(path) => path.to_string(),
                Err(_) => {
                  return Ok(ResponseData {
                    request: Some(request),
                    response: None,
                    response_status: Some(StatusCode::BAD_REQUEST),
                    response_headers: None,
                    new_remote_address: None,
                  });
                }
              };

              let joined_pathbuf = wwwroot.join(decoded_relative_path);
              let mut execute_pathbuf: Option<PathBuf> = None;
              let mut execute_path_info: Option<String> = None;

              // Monoio's `fs` doesn't expose `metadata()` on Windows, so we have to spawn a blocking task to obtain the metadata on this platform
              #[cfg(feature = "runtime-tokio")]
              let metadata = {
                use tokio::fs;
                fs::metadata(&joined_pathbuf).await
              };
              #[cfg(all(feature = "runtime-monoio", unix))]
              let metadata = {
                use monoio::fs;
                fs::metadata(&joined_pathbuf).await
              };
              #[cfg(all(feature = "runtime-monoio", windows))]
              let metadata = {
                let joined_pathbuf = joined_pathbuf.clone();
                monoio::spawn_blocking(move || std::fs::metadata(joined_pathbuf))
                  .await
                  .unwrap_or(Err(std::io::Error::other(
                    "Can't spawn a blocking task to obtain the file metadata",
                  )))
              };

              match metadata {
                Ok(metadata) => {
                  if metadata.is_file() {
                    let contained_extension = joined_pathbuf.extension().map(|a| format!(".{}", a.to_string_lossy()));
                    if let Some(contained_extension) = contained_extension {
                      if fastcgi_script_exts.contains(&(&contained_extension as &str)) {
                        execute_pathbuf = Some(joined_pathbuf);
                      }
                    }
                  } else if metadata.is_dir() {
                    let indexes = vec!["index.php", "index.cgi"];
                    for index in indexes {
                      let temp_joined_pathbuf = joined_pathbuf.join(index);
                      // Monoio's `fs` doesn't expose `metadata()` on Windows, so we have to spawn a blocking task to obtain the metadata on this platform
                      #[cfg(feature = "runtime-tokio")]
                      let temp_metadata = {
                        use tokio::fs;
                        fs::metadata(&temp_joined_pathbuf).await
                      };
                      #[cfg(all(feature = "runtime-monoio", unix))]
                      let temp_metadata = {
                        use monoio::fs;
                        fs::metadata(&temp_joined_pathbuf).await
                      };
                      #[cfg(all(feature = "runtime-monoio", windows))]
                      let temp_metadata = {
                        let temp_joined_pathbuf = temp_joined_pathbuf.clone();
                        monoio::spawn_blocking(move || std::fs::metadata(temp_joined_pathbuf))
                          .await
                          .unwrap_or(Err(std::io::Error::other(
                            "Can't spawn a blocking task to obtain the file metadata",
                          )))
                      };
                      match temp_metadata {
                        Ok(temp_metadata) => {
                          if temp_metadata.is_file() {
                            let contained_extension = temp_joined_pathbuf
                              .extension()
                              .map(|a| format!(".{}", a.to_string_lossy()));
                            if let Some(contained_extension) = contained_extension {
                              if fastcgi_script_exts.contains(&(&contained_extension as &str)) {
                                execute_pathbuf = Some(temp_joined_pathbuf);
                                break;
                              }
                            }
                          }
                        }
                        Err(_) => continue,
                      };
                    }
                  }
                }
                Err(err) => {
                  if err.kind() == std::io::ErrorKind::NotADirectory {
                    // TODO: find a file
                    let mut temp_pathbuf = joined_pathbuf.clone();
                    loop {
                      if !temp_pathbuf.pop() {
                        break;
                      } // Monoio's `fs` doesn't expose `metadata()` on Windows, so we have to spawn a blocking task to obtain the metadata on this platform
                      #[cfg(feature = "runtime-tokio")]
                      let temp_metadata = {
                        use tokio::fs;
                        fs::metadata(&temp_pathbuf).await
                      };
                      #[cfg(all(feature = "runtime-monoio", unix))]
                      let temp_metadata = {
                        use monoio::fs;
                        fs::metadata(&temp_pathbuf).await
                      };
                      #[cfg(all(feature = "runtime-monoio", windows))]
                      let temp_metadata = {
                        let temp_pathbuf = temp_pathbuf.clone();
                        monoio::spawn_blocking(move || std::fs::metadata(temp_pathbuf))
                          .await
                          .unwrap_or(Err(std::io::Error::other(
                            "Can't spawn a blocking task to obtain the file metadata",
                          )))
                      };

                      match temp_metadata {
                        Ok(metadata) => {
                          if metadata.is_file() {
                            let temp_path = temp_pathbuf.as_path();
                            if !temp_path.starts_with(wwwroot) {
                              // Traversed above the webroot, so ignore that.
                              break;
                            }
                            let path_info = match joined_pathbuf.as_path().strip_prefix(temp_path) {
                              Ok(path) => {
                                let path = path.to_string_lossy().to_string();
                                Some(match cfg!(windows) {
                                  true => path.replace("\\", "/"),
                                  false => path,
                                })
                              }
                              Err(_) => None,
                            };
                            let mut request_path_normalized = match cfg!(windows) {
                              true => request_path.to_lowercase(),
                              false => request_path.to_string(),
                            };
                            while request_path_normalized.contains("//") {
                              request_path_normalized = request_path_normalized.replace("//", "/");
                            }
                            if request_path_normalized == "/cgi-bin" || request_path_normalized.starts_with("/cgi-bin/")
                            {
                              execute_pathbuf = Some(temp_pathbuf);
                              execute_path_info = path_info;
                              break;
                            } else {
                              let contained_extension =
                                temp_pathbuf.extension().map(|a| format!(".{}", a.to_string_lossy()));
                              if let Some(contained_extension) = contained_extension {
                                if fastcgi_script_exts.contains(&(&contained_extension as &str)) {
                                  execute_pathbuf = Some(temp_pathbuf);
                                  execute_path_info = path_info;
                                  break;
                                }
                              }
                            }
                          } else {
                            break;
                          }
                        }
                        Err(err) => match err.kind() {
                          std::io::ErrorKind::NotADirectory => (),
                          _ => break,
                        },
                      };
                    }
                  }
                }
              };
              let data = (execute_pathbuf, execute_path_info);

              let mut write_rwlock = self.path_cache.write().await;
              write_rwlock.cleanup();
              write_rwlock.insert(cache_key, data.clone());
              drop(write_rwlock);
              data
            }
          };

          execute_pathbuf = execute_pathbuf_got;
          execute_path_info = execute_path_info_got;
        }
      }

      if let Some(execute_pathbuf) = execute_pathbuf {
        if let Some(wwwroot_detected) = wwwroot_detected {
          let mut additional_environment_variables = HashMap::new();
          if let Some(additional_environment_variables_config) = get_entries!("fcgi_environment", config) {
            for additional_variable in additional_environment_variables_config.inner.iter() {
              if let Some(key) = additional_variable.values.first().and_then(|v| v.as_str()) {
                if let Some(value) = additional_variable.values.get(1).and_then(|v| v.as_str()) {
                  additional_environment_variables.insert(key.to_string(), value.to_string());
                }
              }
            }
          }

          return execute_fastcgi_with_environment_variables(
            request,
            socket_data,
            error_logger,
            wwwroot_detected.as_path(),
            execute_pathbuf,
            execute_path_info,
            get_value!("server_administrator_email", config).and_then(|v| v.as_str()),
            fastcgi_to,
            additional_environment_variables,
          )
          .await;
        }
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

#[allow(clippy::too_many_arguments)]
async fn execute_fastcgi_with_environment_variables(
  request: Request<BoxBody<Bytes, std::io::Error>>,
  socket_data: &SocketData,
  error_logger: &ErrorLogger,
  wwwroot: &Path,
  execute_pathbuf: PathBuf,
  path_info: Option<String>,
  server_administrator_email: Option<&str>,
  fastcgi_to: &str,
  additional_environment_variables: HashMap<String, String>,
) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
  let mut environment_variables: LinkedHashMap<String, String> = LinkedHashMap::new();

  let request_data = request.extensions().get::<RequestData>();

  let original_request_uri = request_data
    .and_then(|d| d.original_url.as_ref())
    .unwrap_or(request.uri());

  if let Some(auth_user) = request_data.and_then(|u| u.auth_user.as_ref()) {
    if let Some(authorization) = request.headers().get(header::AUTHORIZATION) {
      let authorization_value = String::from_utf8_lossy(authorization.as_bytes()).to_string();
      let mut authorization_value_split = authorization_value.split(" ");
      if let Some(authorization_type) = authorization_value_split.next() {
        environment_variables.insert("AUTH_TYPE".to_string(), authorization_type.to_string());
      }
    }
    environment_variables.insert("REMOTE_USER".to_string(), auth_user.to_string());
  }

  environment_variables.insert(
    "QUERY_STRING".to_string(),
    match request.uri().query() {
      Some(query) => query.to_string(),
      None => "".to_string(),
    },
  );

  environment_variables.insert("SERVER_SOFTWARE".to_string(), SERVER_SOFTWARE.to_string());
  environment_variables.insert(
    "SERVER_PROTOCOL".to_string(),
    match request.version() {
      hyper::Version::HTTP_09 => "HTTP/0.9".to_string(),
      hyper::Version::HTTP_10 => "HTTP/1.0".to_string(),
      hyper::Version::HTTP_11 => "HTTP/1.1".to_string(),
      hyper::Version::HTTP_2 => "HTTP/2.0".to_string(),
      hyper::Version::HTTP_3 => "HTTP/3.0".to_string(),
      _ => "HTTP/Unknown".to_string(),
    },
  );
  environment_variables.insert("SERVER_PORT".to_string(), socket_data.local_addr.port().to_string());
  environment_variables.insert(
    "SERVER_ADDR".to_string(),
    socket_data.local_addr.ip().to_canonical().to_string(),
  );
  if let Some(server_administrator_email) = server_administrator_email {
    environment_variables.insert("SERVER_ADMIN".to_string(), server_administrator_email.to_string());
  }
  if let Some(host) = request.headers().get(header::HOST) {
    environment_variables.insert(
      "SERVER_NAME".to_string(),
      String::from_utf8_lossy(host.as_bytes()).to_string(),
    );
  }

  environment_variables.insert("DOCUMENT_ROOT".to_string(), wwwroot.to_string_lossy().to_string());
  environment_variables.insert(
    "PATH_INFO".to_string(),
    match &path_info {
      Some(path_info) => format!("/{path_info}"),
      None => "".to_string(),
    },
  );
  environment_variables.insert(
    "PATH_TRANSLATED".to_string(),
    match &path_info {
      Some(path_info) => {
        let mut path_translated = execute_pathbuf.clone();
        path_translated.push(path_info);
        path_translated.to_string_lossy().to_string()
      }
      None => "".to_string(),
    },
  );
  environment_variables.insert("REQUEST_METHOD".to_string(), request.method().to_string());
  environment_variables.insert("GATEWAY_INTERFACE".to_string(), "CGI/1.1".to_string());
  environment_variables.insert(
    "REQUEST_URI".to_string(),
    format!(
      "{}{}",
      original_request_uri.path(),
      match original_request_uri.query() {
        Some(query) => format!("?{query}"),
        None => String::from(""),
      }
    ),
  );

  environment_variables.insert("REMOTE_PORT".to_string(), socket_data.remote_addr.port().to_string());
  environment_variables.insert(
    "REMOTE_ADDR".to_string(),
    socket_data.remote_addr.ip().to_canonical().to_string(),
  );

  environment_variables.insert(
    "SCRIPT_FILENAME".to_string(),
    execute_pathbuf.to_string_lossy().to_string(),
  );
  if let Ok(script_path) = execute_pathbuf.as_path().strip_prefix(wwwroot) {
    environment_variables.insert(
      "SCRIPT_NAME".to_string(),
      format!(
        "/{}",
        match cfg!(windows) {
          true => script_path.to_string_lossy().to_string().replace("\\", "/"),
          false => script_path.to_string_lossy().to_string(),
        }
      ),
    );
  }

  if socket_data.encrypted {
    environment_variables.insert("HTTPS".to_string(), "ON".to_string());
  }

  for (header_name, header_value) in request.headers().iter() {
    let env_header_name = match *header_name {
      header::CONTENT_LENGTH => "CONTENT_LENGTH".to_string(),
      header::CONTENT_TYPE => "CONTENT_TYPE".to_string(),
      _ => {
        let mut result = String::new();

        result.push_str("HTTP_");

        for c in header_name.as_str().to_uppercase().chars() {
          if c.is_alphanumeric() {
            result.push(c);
          } else {
            result.push('_');
          }
        }

        result
      }
    };
    if environment_variables.contains_key(&env_header_name) {
      let value = environment_variables.get_mut(&env_header_name);
      if let Some(value) = value {
        if env_header_name == "HTTP_COOKIE" {
          value.push_str("; ");
        } else {
          // See https://stackoverflow.com/a/1801191
          value.push_str(", ");
        }
        value.push_str(String::from_utf8_lossy(header_value.as_bytes()).as_ref());
      } else {
        environment_variables.insert(
          env_header_name,
          String::from_utf8_lossy(header_value.as_bytes()).to_string(),
        );
      }
    } else {
      environment_variables.insert(
        env_header_name,
        String::from_utf8_lossy(header_value.as_bytes()).to_string(),
      );
    }
  }

  for (env_var_key, env_var_value) in additional_environment_variables {
    if let hashlink::linked_hash_map::Entry::Vacant(entry) = environment_variables.entry(env_var_key) {
      entry.insert(env_var_value);
    }
  }

  execute_fastcgi(request, error_logger, fastcgi_to, environment_variables).await
}

async fn execute_fastcgi(
  request: Request<BoxBody<Bytes, std::io::Error>>,
  error_logger: &ErrorLogger,
  fastcgi_to: &str,
  mut environment_variables: LinkedHashMap<String, String>,
) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
  let (_, body) = request.into_parts();

  // Insert other environment variables
  for (key, value) in env::vars_os() {
    let key_string = key.to_string_lossy().to_string();
    let value_string = value.to_string_lossy().to_string();
    environment_variables.entry(key_string).or_insert(value_string);
  }

  let fastcgi_to_fixed = if let Some(stripped) = fastcgi_to.strip_prefix("unix:///") {
    // hyper::Uri fails to parse a string if there is an empty authority, so add an "ignore" authority to Unix socket URLs
    &format!("unix://ignore/{stripped}")
  } else {
    fastcgi_to
  };

  let fastcgi_to_url = fastcgi_to_fixed.parse::<hyper::Uri>()?;
  let scheme_str = fastcgi_to_url.scheme_str();

  let (socket_reader, mut socket_writer) = match scheme_str {
    Some("tcp") => {
      let host = match fastcgi_to_url.host() {
        Some(host) => host,
        None => Err(anyhow::anyhow!("The FastCGI URL doesn't include the host"))?,
      };

      let port = match fastcgi_to_url.port_u16() {
        Some(port) => port,
        None => Err(anyhow::anyhow!("The FastCGI URL doesn't include the port"))?,
      };

      let addr = format!("{host}:{port}");

      match connect_tcp(&addr).await {
        Ok(data) => data,
        Err(err) => match err.kind() {
          std::io::ErrorKind::ConnectionRefused
          | std::io::ErrorKind::NotFound
          | std::io::ErrorKind::HostUnreachable => {
            error_logger.log(&format!("Service unavailable: {err}")).await;
            return Ok(ResponseData {
              request: None,
              response: None,
              response_status: Some(StatusCode::SERVICE_UNAVAILABLE),
              response_headers: None,
              new_remote_address: None,
            });
          }
          _ => Err(err)?,
        },
      }
    }
    Some("unix") => {
      let path = fastcgi_to_url.path();
      match connect_unix(path).await {
        Ok(data) => data,
        Err(err) => match err.kind() {
          std::io::ErrorKind::ConnectionRefused
          | std::io::ErrorKind::NotFound
          | std::io::ErrorKind::HostUnreachable => {
            error_logger.log(&format!("Service unavailable: {err}")).await;
            return Ok(ResponseData {
              request: None,
              response: None,
              response_status: Some(StatusCode::SERVICE_UNAVAILABLE),
              response_headers: None,
              new_remote_address: None,
            });
          }
          _ => Err(err)?,
        },
      }
    }
    _ => Err(anyhow::anyhow!("Only HTTP and HTTPS reverse proxy URLs are supported."))?,
  };

  // Construct and send BEGIN_REQUEST record
  // Use the responder role and don't use keep-alive
  let begin_request_packet = construct_fastcgi_record(1, 1, &[0, 1, 0, 0, 0, 0, 0, 0]);
  socket_writer.write_all(&begin_request_packet).await?;

  // Construct and send PARAMS records
  let mut environment_variables_to_wrap = Vec::new();
  for (key, value) in environment_variables.iter() {
    let mut environment_variable = construct_fastcgi_name_value_pair(key.as_bytes(), value.as_bytes());
    environment_variables_to_wrap.append(&mut environment_variable);
  }
  if !environment_variables_to_wrap.is_empty() {
    let mut offset = 0;
    while offset < environment_variables_to_wrap.len() {
      let chunk_size = std::cmp::min(65536, environment_variables_to_wrap.len() - offset);
      let chunk = &environment_variables_to_wrap[offset..offset + chunk_size];

      // Record type 4 means PARAMS
      let params_packet = construct_fastcgi_record(4, 1, chunk);
      socket_writer.write_all(&params_packet).await?;

      offset += chunk_size;
    }
  }

  let params_packet_terminating = construct_fastcgi_record(4, 1, &[]);
  socket_writer.write_all(&params_packet_terminating).await?;

  let cgi_stdin_reader = StreamReader::new(body.into_data_stream().map_err(std::io::Error::other));

  // Emulated standard input, standard output, and standard error
  type EitherStream = Either<Result<Bytes, std::io::Error>, Result<Bytes, std::io::Error>>;
  let stdin = SinkWriter::new(FramedWrite::new(socket_writer, FcgiEncoder::new()));
  let stdout_and_stderr = FramedRead::new(socket_reader, FcgiDecoder::new());
  let (stdout_stream, stderr_stream) = stdout_and_stderr.split_by_map(|item| match item {
    Ok(FcgiDecodedData::Stdout(bytes)) => EitherStream::Left(Ok(bytes)),
    Ok(FcgiDecodedData::Stderr(bytes)) => EitherStream::Right(Ok(bytes)),
    Err(err) => EitherStream::Left(Err(err)),
  });
  let stdout = StreamReader::new(stdout_stream);
  let stderr = StreamReader::new(stderr_stream);

  let mut cgi_response = CgiResponse::new(stdout);

  ferron_common::runtime::spawn(Copier::with_zero_packet_writing(cgi_stdin_reader, stdin).copy());

  let stderr_read_future = ReadToEndFuture::new(stderr);
  let mut stderr_read_future_pinned = Box::pin(stderr_read_future);

  let mut headers = [EMPTY_HEADER; 128];

  // Needed to wrap this in another scope to prevent errors with multiple mutable borrows.
  {
    let stdout_parse_future = cgi_response.get_head();
    let mut stdout_parse_future_pinned = Box::pin(stdout_parse_future);

    ferron_common::runtime::select! {
        biased;

        result = &mut stdout_parse_future_pinned => {
          let obtained_head = result?;
          if !obtained_head.is_empty() {
            httparse::parse_headers(obtained_head, &mut headers)?;
          }
        }
        result = &mut stderr_read_future_pinned => {
          let stderr_vec = result?;
            let stderr_string = String::from_utf8_lossy(stderr_vec.as_slice()).to_string();
            if !stderr_string.is_empty() {
              error_logger
                .log(&format!("There were CGI errors: {stderr_string}"))
                .await;
            }
            return Ok(
              ResponseData { request: None, response: None, response_status: Some(StatusCode::INTERNAL_SERVER_ERROR), response_headers: None, new_remote_address: None }
            );
        },
    }
  }

  let mut response_builder = Response::builder();
  let mut status_code = 200;
  for header in headers {
    if header == EMPTY_HEADER {
      break;
    }
    let mut is_status_header = false;
    match &header.name.to_lowercase() as &str {
      "location" => {
        if !(300..=399).contains(&status_code) {
          status_code = 302;
        }
      }
      "status" => {
        is_status_header = true;
        let header_value_cow = String::from_utf8_lossy(header.value);
        let mut split_status = header_value_cow.split(" ");
        let first_part = split_status.next();
        if let Some(first_part) = first_part {
          if first_part.starts_with("HTTP/") {
            let second_part = split_status.next();
            if let Some(second_part) = second_part {
              if let Ok(parsed_status_code) = second_part.parse::<u16>() {
                status_code = parsed_status_code;
              }
            }
          } else if let Ok(parsed_status_code) = first_part.parse::<u16>() {
            status_code = parsed_status_code;
          }
        }
      }
      _ => (),
    }
    if !is_status_header {
      response_builder = response_builder.header(header.name, header.value);
    }
  }

  response_builder = response_builder.status(status_code);

  let mut reader_stream = ReaderStream::new(cgi_response);
  let (reader_stream_tx, reader_stream_rx) = async_channel::bounded(MAX_RESPONSE_CHANNEL_CAPACITY);
  ferron_common::runtime::spawn(async move {
    while let Some(chunk) = reader_stream.next().await {
      reader_stream_tx.send(chunk).await.unwrap_or_default();
    }
    reader_stream_tx.close();
  });
  let stream_body = StreamBody::new(reader_stream_rx.map_ok(Frame::data));
  let boxed_body = BodyExt::boxed(stream_body);

  let response = response_builder.body(boxed_body)?;

  let error_logger_clone = error_logger.clone();
  ferron_common::runtime::spawn(async move {
    let stderr_vec = stderr_read_future_pinned.await.unwrap_or(vec![]);
    let stderr_string = String::from_utf8_lossy(stderr_vec.as_slice()).to_string();
    if !stderr_string.is_empty() {
      error_logger_clone
        .log(&format!("There were FastCGI errors: {stderr_string}"))
        .await;
    }
  });

  Ok(ResponseData {
    request: None,
    response: Some(response),
    response_status: None,
    response_headers: None,
    new_remote_address: None,
  })
}

#[cfg(feature = "runtime-monoio")]
async fn connect_tcp(addr: &str) -> Result<(Box<dyn AsyncRead + Unpin>, Box<dyn AsyncWrite + Unpin>), std::io::Error> {
  let socket = TcpStream::connect(addr).await?;
  socket.set_nodelay(true)?;

  let (socket_reader_set, socket_writer_set) = tokio::io::split(socket.into_poll_io()?);
  Ok((Box::new(socket_reader_set), Box::new(socket_writer_set)))
}

#[cfg(feature = "runtime-tokio")]
async fn connect_tcp(
  addr: &str,
) -> Result<
  (
    Box<dyn AsyncRead + Send + Sync + Unpin>,
    Box<dyn AsyncWrite + Send + Sync + Unpin>,
  ),
  std::io::Error,
> {
  let socket = TcpStream::connect(addr).await?;
  socket.set_nodelay(true)?;

  let (socket_reader_set, socket_writer_set) = tokio::io::split(socket);
  Ok((Box::new(socket_reader_set), Box::new(socket_writer_set)))
}

#[allow(dead_code)]
#[cfg(all(feature = "runtime-monoio", unix))]
async fn connect_unix(path: &str) -> Result<(Box<dyn AsyncRead + Unpin>, Box<dyn AsyncWrite + Unpin>), std::io::Error> {
  use monoio::net::UnixStream;

  let socket = UnixStream::connect(path).await?;

  let (socket_reader_set, socket_writer_set) = tokio::io::split(socket.into_poll_io()?);
  Ok((Box::new(socket_reader_set), Box::new(socket_writer_set)))
}

#[allow(dead_code)]
#[cfg(all(feature = "runtime-tokio", unix))]
async fn connect_unix(
  path: &str,
) -> Result<
  (
    Box<dyn AsyncRead + Send + Sync + Unpin>,
    Box<dyn AsyncWrite + Send + Sync + Unpin>,
  ),
  std::io::Error,
> {
  use tokio::net::UnixStream;

  let socket = UnixStream::connect(path).await?;

  let (socket_reader_set, socket_writer_set) = tokio::io::split(socket);
  Ok((Box::new(socket_reader_set), Box::new(socket_writer_set)))
}

#[allow(dead_code)]
#[cfg(all(feature = "runtime-monoio", not(unix)))]
async fn connect_unix(
  _path: &str,
) -> Result<(Box<dyn AsyncRead + Unpin>, Box<dyn AsyncWrite + Unpin>), std::io::Error> {
  Err(std::io::Error::new(
    std::io::ErrorKind::Unsupported,
    "Unix sockets are not supports on non-Unix platforms.",
  ))
}

#[allow(dead_code)]
#[cfg(all(feature = "runtime-tokio", not(unix)))]
async fn connect_unix(
  _path: &str,
) -> Result<
  (
    Box<dyn AsyncRead + Send + Sync + Unpin>,
    Box<dyn AsyncWrite + Send + Sync + Unpin>,
  ),
  std::io::Error,
> {
  Err(std::io::Error::new(
    std::io::ErrorKind::Unsupported,
    "Unix sockets are not supports on non-Unix platforms.",
  ))
}
