use std::collections::HashMap;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

#[cfg(feature = "runtime-monoio")]
use async_process::Command;
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::stream::TryStreamExt;
use hashlink::LinkedHashMap;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, StreamBody};
use httparse::EMPTY_HEADER;
use hyper::body::Frame;
use hyper::{header, Request, Response, StatusCode};
#[cfg(feature = "runtime-monoio")]
use monoio::fs;
#[cfg(feature = "runtime-tokio")]
use tokio::fs;
use tokio::io::AsyncReadExt;
#[cfg(feature = "runtime-tokio")]
use tokio::process::Command;
use tokio::sync::RwLock;
#[cfg(feature = "runtime-monoio")]
use tokio_util::compat::{FuturesAsyncReadCompatExt, FuturesAsyncWriteCompatExt};
use tokio_util::io::{ReaderStream, StreamReader};

use crate::util::cgi::CgiResponse;
use crate::util::Copier;
use ferron_common::config::ServerConfiguration;
use ferron_common::logging::ErrorLogger;
use ferron_common::modules::{Module, ModuleHandlers, ModuleLoader, RequestData, ResponseData, SocketData};
use ferron_common::util::{ModuleCache, TtlCache, SERVER_SOFTWARE};
use ferron_common::{get_entries, get_entries_for_validation, get_entry, get_value};

/// A CGI module loader
#[allow(clippy::type_complexity)]
pub struct CgiModuleLoader {
  cache: ModuleCache<CgiModule>,
  path_cache: Arc<RwLock<TtlCache<String, (Option<PathBuf>, Option<String>)>>>,
}

impl Default for CgiModuleLoader {
  fn default() -> Self {
    Self::new()
  }
}

impl CgiModuleLoader {
  /// Creates a new module loader
  pub fn new() -> Self {
    Self {
      cache: ModuleCache::new(vec![]),
      path_cache: Arc::new(RwLock::new(TtlCache::new(Duration::from_millis(100)))),
    }
  }
}

impl ModuleLoader for CgiModuleLoader {
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
          Ok(Arc::new(CgiModule {
            path_cache: self.path_cache.clone(),
          }))
        })?,
    )
  }

  fn get_requirements(&self) -> Vec<&'static str> {
    vec!["cgi"]
  }

  fn validate_configuration(
    &self,
    config: &ServerConfiguration,
    used_properties: &mut std::collections::HashSet<String>,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    if let Some(entries) = get_entries_for_validation!("cgi", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `cgi` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!("Invalid CGI enabling option"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("cgi_interpreter", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.first().is_some_and(|v| v.is_null())
          || !entry.values.get(1).is_some_and(|v| v.is_null() || v.is_string())
        {
          Err(anyhow::anyhow!("Invalid CGI extension interpreter specification"))?
        }
        for value in &entry.values {
          if !value.is_string() {
            Err(anyhow::anyhow!("Invalid CGI extension interpreter specification"))?
          }
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("cgi_extension", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `cgi_extension` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_string() {
          Err(anyhow::anyhow!("The CGI file extension must be a string"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("cgi_environment", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 2 {
          Err(anyhow::anyhow!(
            "The `cgi_environment` configuration property must have exactly two values"
          ))?
        } else if !entry.values[0].is_string() {
          Err(anyhow::anyhow!("The CGI environment variable name must be a string"))?
        } else if !entry.values[1].is_string() {
          Err(anyhow::anyhow!("The CGI environment variable value must be a string"))?
        }
      }
    };

    Ok(())
  }
}

/// A CGI module
#[allow(clippy::type_complexity)]
struct CgiModule {
  path_cache: Arc<RwLock<TtlCache<String, (Option<PathBuf>, Option<String>)>>>,
}

impl Module for CgiModule {
  fn get_module_handlers(&self) -> Box<dyn ModuleHandlers> {
    Box::new(CgiModuleHandlers {
      path_cache: self.path_cache.clone(),
    })
  }
}

/// Handlers for the CGI module
#[allow(clippy::type_complexity)]
struct CgiModuleHandlers {
  path_cache: Arc<RwLock<TtlCache<String, (Option<PathBuf>, Option<String>)>>>,
}

#[async_trait(?Send)]
impl ModuleHandlers for CgiModuleHandlers {
  async fn request_handler(
    &mut self,
    request: Request<BoxBody<Bytes, std::io::Error>>,
    config: &ServerConfiguration,
    socket_data: &SocketData,
    error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    let mut cgi_script_exts = Vec::new();

    let cgi_script_exts_config = get_entries!("cgi_extension", config);
    if let Some(cgi_script_exts_obtained) = cgi_script_exts_config {
      for cgi_script_ext_config in cgi_script_exts_obtained.inner.iter() {
        if let Some(cgi_script_ext) = cgi_script_ext_config.values.first().and_then(|v| v.as_str()) {
          cgi_script_exts.push(cgi_script_ext);
        }
      }
    }

    if let Some(wwwroot) = get_entry!("root", config)
      .and_then(|e| e.values.first())
      .and_then(|v| v.as_str())
    {
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
          let canonicalize_result = fs::canonicalize(&wwwroot_unknown).await;

          match canonicalize_result {
            Ok(pathbuf) => pathbuf,
            Err(_) => wwwroot_unknown,
          }
        }
      };
      let wwwroot = wwwroot_pathbuf.as_path();

      let read_rwlock = self.path_cache.read().await;
      let (execute_pathbuf, execute_path_info) = match read_rwlock.get(&cache_key) {
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
                let mut request_path_normalized = match cfg!(windows) {
                  true => request_path.to_lowercase(),
                  false => request_path.to_string(),
                };
                while request_path_normalized.contains("//") {
                  request_path_normalized = request_path_normalized.replace("//", "/");
                }
                if request_path_normalized == "/cgi-bin" || request_path_normalized.starts_with("/cgi-bin/") {
                  execute_pathbuf = Some(joined_pathbuf);
                } else {
                  let contained_extension = joined_pathbuf.extension().map(|a| format!(".{}", a.to_string_lossy()));
                  if let Some(contained_extension) = contained_extension {
                    if cgi_script_exts.contains(&(&contained_extension as &str)) {
                      execute_pathbuf = Some(joined_pathbuf);
                    }
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
                        let request_path_normalized = match cfg!(windows) {
                          true => request_path.to_lowercase(),
                          false => request_path.to_string(),
                        };
                        if request_path_normalized == "/cgi-bin" || request_path_normalized.starts_with("/cgi-bin/") {
                          execute_pathbuf = Some(temp_joined_pathbuf);
                          break;
                        } else {
                          let contained_extension = temp_joined_pathbuf
                            .extension()
                            .map(|a| format!(".{}", a.to_string_lossy()));
                          if let Some(contained_extension) = contained_extension {
                            if cgi_script_exts.contains(&(&contained_extension as &str)) {
                              execute_pathbuf = Some(temp_joined_pathbuf);
                              break;
                            }
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
                let mut temp_pathbuf = joined_pathbuf.clone();
                loop {
                  if !temp_pathbuf.pop() {
                    break;
                  }
                  // Monoio's `fs` doesn't expose `metadata()` on Windows, so we have to spawn a blocking task to obtain the metadata on this platform
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
                        if request_path_normalized == "/cgi-bin" || request_path_normalized.starts_with("/cgi-bin/") {
                          execute_pathbuf = Some(temp_pathbuf);
                          execute_path_info = path_info;
                          break;
                        } else {
                          let contained_extension =
                            temp_pathbuf.extension().map(|a| format!(".{}", a.to_string_lossy()));
                          if let Some(contained_extension) = contained_extension {
                            if cgi_script_exts.contains(&(&contained_extension as &str)) {
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

      if let Some(execute_pathbuf) = execute_pathbuf {
        let mut cgi_interpreters = HashMap::new();
        cgi_interpreters.insert(".pl".to_string(), vec!["perl".to_string()]);
        cgi_interpreters.insert(".py".to_string(), vec!["python".to_string()]);
        cgi_interpreters.insert(".sh".to_string(), vec!["bash".to_string()]);
        cgi_interpreters.insert(".ksh".to_string(), vec!["ksh".to_string()]);
        cgi_interpreters.insert(".csh".to_string(), vec!["csh".to_string()]);
        cgi_interpreters.insert(".rb".to_string(), vec!["ruby".to_string()]);
        cgi_interpreters.insert(".php".to_string(), vec!["php-cgi".to_string()]);
        if cfg!(windows) {
          cgi_interpreters.insert(".exe".to_string(), vec![]);
          cgi_interpreters.insert(".bat".to_string(), vec!["cmd".to_string(), "/c".to_string()]);
          cgi_interpreters.insert(".vbs".to_string(), vec!["cscript".to_string()]);
        }

        if let Some(cgi_interpreters_entries) = get_entries!("cgi_interpreter", config) {
          for entry in cgi_interpreters_entries.inner.iter() {
            if let Some(key) = entry.values.first().and_then(|v| v.as_str()) {
              if entry.values.get(1).is_none_or(|v| v.is_null()) {
                cgi_interpreters.remove(key);
              } else {
                let mut params = Vec::new();
                for param_index in 1..entry.values.len() {
                  if let Some(param) = entry.values.get(param_index).and_then(|v| v.as_str()) {
                    params.push(param.to_string());
                  }
                }
                cgi_interpreters.insert(key.to_string(), params);
              }
            }
          }
        }

        let mut additional_environment_variables = HashMap::new();
        if let Some(additional_environment_variables_config) = get_entries!("cgi_environment", config) {
          for additional_variable in additional_environment_variables_config.inner.iter() {
            if let Some(key) = additional_variable.values.first().and_then(|v| v.as_str()) {
              if let Some(value) = additional_variable.values.get(1).and_then(|v| v.as_str()) {
                additional_environment_variables.insert(key.to_string(), value.to_string());
              }
            }
          }
        }

        return execute_cgi_with_environment_variables(
          request,
          socket_data,
          error_logger,
          wwwroot,
          execute_pathbuf,
          execute_path_info,
          config.filters.hostname.as_deref(),
          get_value!("server_administrator_email", config).and_then(|v| v.as_str()),
          cgi_interpreters,
          additional_environment_variables,
        )
        .await;
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
async fn execute_cgi_with_environment_variables(
  request: Request<BoxBody<Bytes, std::io::Error>>,
  socket_data: &SocketData,
  error_logger: &ErrorLogger,
  wwwroot: &Path,
  execute_pathbuf: PathBuf,
  path_info: Option<String>,
  server_name: Option<&str>,
  server_administrator_email: Option<&str>,
  cgi_interpreters: HashMap<String, Vec<String>>,
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
  environment_variables.insert(
    "SERVER_NAME".to_string(),
    server_name
      .map(|name| name.to_string())
      .unwrap_or_else(|| socket_data.local_addr.ip().to_canonical().to_string()),
  );
  if let Some(server_administrator_email) = server_administrator_email {
    environment_variables.insert("SERVER_ADMIN".to_string(), server_administrator_email.to_string());
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

  execute_cgi(
    request,
    error_logger,
    execute_pathbuf,
    cgi_interpreters,
    environment_variables,
  )
  .await
}

async fn execute_cgi(
  request: Request<BoxBody<Bytes, std::io::Error>>,
  error_logger: &ErrorLogger,
  execute_pathbuf: PathBuf,
  cgi_interpreters: HashMap<String, Vec<String>>,
  environment_variables: LinkedHashMap<String, String>,
) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
  let (_, body) = request.into_parts();

  let executable_params = match get_executable(&execute_pathbuf).await {
    Ok(params) => params,
    Err(err) => {
      let contained_extension = execute_pathbuf.extension().map(|a| format!(".{}", a.to_string_lossy()));
      if let Some(contained_extension) = contained_extension {
        if let Some(params_init) = cgi_interpreters.get(&contained_extension) {
          let mut params: Vec<String> = params_init.iter().map(|s| s.to_owned()).collect();
          params.push(execute_pathbuf.to_string_lossy().to_string());
          params
        } else {
          Err(err)?
        }
      } else {
        Err(err)?
      }
    }
  };

  let mut executable_params_iter = executable_params.iter();

  let mut command = Command::new(match executable_params_iter.next() {
    Some(executable_name) => executable_name,
    None => Err(anyhow::anyhow!("Cannot determine the executable"))?,
  });

  // Set standard I/O to be piped
  command.stdin(Stdio::piped());
  command.stdout(Stdio::piped());
  command.stderr(Stdio::piped());

  for param in executable_params_iter {
    command.arg(param);
  }

  command.envs(environment_variables);

  let mut execute_dir_pathbuf = execute_pathbuf.clone();
  execute_dir_pathbuf.pop();
  command.current_dir(execute_dir_pathbuf);

  let mut child = command.spawn()?;

  let cgi_stdin_reader = StreamReader::new(body.into_data_stream().map_err(std::io::Error::other));

  #[cfg(feature = "runtime-monoio")]
  let stdin = match child.stdin.take() {
    Some(stdin) => stdin.compat_write(),
    None => Err(anyhow::anyhow!("The CGI process doesn't have standard input"))?,
  };
  #[cfg(feature = "runtime-monoio")]
  let stdout = match child.stdout.take() {
    Some(stdout) => stdout.compat(),
    None => Err(anyhow::anyhow!("The CGI process doesn't have standard output"))?,
  };
  #[cfg(feature = "runtime-monoio")]
  let stderr = child.stderr.take().map(|x| x.compat());

  #[cfg(feature = "runtime-tokio")]
  let stdin = match child.stdin.take() {
    Some(stdin) => stdin,
    None => Err(anyhow::anyhow!("The CGI process doesn't have standard input"))?,
  };
  #[cfg(feature = "runtime-tokio")]
  let stdout = match child.stdout.take() {
    Some(stdout) => stdout,
    None => Err(anyhow::anyhow!("The CGI process doesn't have standard output"))?,
  };
  #[cfg(feature = "runtime-tokio")]
  let stderr = child.stderr.take();

  let mut cgi_response = CgiResponse::new(stdout);

  ferron_common::runtime::spawn(Copier::new(cgi_stdin_reader, stdin).copy());

  let mut headers = [EMPTY_HEADER; 128];

  let obtained_head = cgi_response.get_head().await?;
  if !obtained_head.is_empty() {
    httparse::parse_headers(obtained_head, &mut headers)?;
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

  let reader_stream = ReaderStream::new(cgi_response);
  let stream_body = StreamBody::new(reader_stream.map_ok(Frame::data));
  let boxed_body = stream_body.boxed();

  let response = response_builder.body(boxed_body)?;

  #[cfg(feature = "runtime-monoio")]
  let exit_code_option = child.try_status()?;
  #[cfg(feature = "runtime-tokio")]
  let exit_code_option = child.try_wait()?;

  if let Some(exit_code) = exit_code_option {
    if !exit_code.success() {
      if let Some(mut stderr) = stderr {
        let mut stderr_string = String::new();
        stderr.read_to_string(&mut stderr_string).await.unwrap_or_default();
        let stderr_string_trimmed = stderr_string.trim();
        if !stderr_string_trimmed.is_empty() {
          error_logger
            .log(&format!("There were CGI errors: {stderr_string_trimmed}"))
            .await;
        }
      }
      return Ok(ResponseData {
        request: None,
        response: None,
        response_status: Some(StatusCode::INTERNAL_SERVER_ERROR),
        response_headers: None,
        new_remote_address: None,
      });
    }
  }

  let error_logger = error_logger.clone();

  ferron_common::runtime::spawn(async move {
    if let Some(mut stderr) = stderr {
      let mut stderr_string = String::new();
      stderr.read_to_string(&mut stderr_string).await.unwrap_or_default();
      let stderr_string_trimmed = stderr_string.trim();
      if !stderr_string_trimmed.is_empty() {
        error_logger
          .log(&format!("There were CGI errors: {stderr_string_trimmed}"))
          .await;
      }
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


#[allow(dead_code)]
#[cfg(unix)]
async fn get_executable(execute_pathbuf: &PathBuf) -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
  use std::os::unix::fs::PermissionsExt;

  // `monoio::fs::metadata` is available on Unix
  let metadata = fs::metadata(&execute_pathbuf).await?;
  let permissions = metadata.permissions();
  let is_executable = permissions.mode() & 0o111 != 0;

  if !is_executable {
    Err(anyhow::anyhow!("The CGI program is not executable"))?
  }

  let executable_params_vector = vec![execute_pathbuf.to_string_lossy().to_string()];
  Ok(executable_params_vector)
}


#[allow(dead_code)]
#[cfg(all(feature = "runtime-monoio", not(unix)))]
async fn get_executable(execute_pathbuf: &PathBuf) -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
  use bytes::BytesMut;

  let magic_signature_buffer = BytesMut::with_capacity(2);
  let open_file = fs::File::open(&execute_pathbuf).await?;
  let open_file_result = open_file.read_exact_at(magic_signature_buffer, 0).await;
  if open_file_result.0.is_err() {
    Err(anyhow::anyhow!("Failed to read the CGI program signature"))?
  }

  match open_file_result.1.freeze().as_ref() {
    b"PE" => {
      // Windows executables
      let executable_params_vector = vec![execute_pathbuf.to_string_lossy().to_string()];
      Ok(executable_params_vector)
    }
    b"#!" => {
      // Scripts with a shebang line
      let mut shebang_line_bytes = Vec::new();
      let mut shebang_bytes_read = 0;
      loop {
        let buf = BytesMut::with_capacity(1024);
        let read_result = open_file.read_at(buf, shebang_bytes_read).await;
        read_result.0?;
        let buf = read_result.1.freeze();

        shebang_bytes_read += shebang_bytes_read;
        if let Some(index) = memchr::memchr(b'\n', &buf) {
          shebang_line_bytes.extend_from_slice(&buf[..index + 1]);
          break;
        } else if let Some(index) = memchr::memchr(b'\r', &buf) {
          shebang_line_bytes.extend_from_slice(&buf[..index + 1]);
          break;
        } else {
          shebang_line_bytes.extend_from_slice(&buf);
        }
      }
      let shebang_line = String::from_utf8_lossy(&shebang_line_bytes);

      let mut command_begin: Vec<String> = shebang_line[2..]
        .replace("\r", "")
        .replace("\n", "")
        .split(" ")
        .map(|s| s.to_owned())
        .collect();
      command_begin.push(execute_pathbuf.to_string_lossy().to_string());
      Ok(command_begin)
    }
    _ => {
      // It's not executable
      Err(anyhow::anyhow!("The CGI program is not executable"))?
    }
  }
}

#[allow(dead_code)]
#[cfg(all(feature = "runtime-tokio", not(unix)))]
async fn get_executable(execute_pathbuf: &PathBuf) -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
  use tokio::io::{AsyncBufReadExt, AsyncSeekExt, BufReader};

  let mut magic_signature_buffer = [0u8; 2];
  let mut open_file = fs::File::open(&execute_pathbuf).await?;
  if open_file.read_exact(&mut magic_signature_buffer).await.is_err() {
    Err(anyhow::anyhow!("Failed to read the CGI program signature"))?
  }

  match &magic_signature_buffer {
    b"PE" => {
      // Windows executables
      let executable_params_vector = vec![execute_pathbuf.to_string_lossy().to_string()];
      Ok(executable_params_vector)
    }
    b"#!" => {
      // Scripts with a shebang line
      open_file.rewind().await?;
      let mut buffered_file = BufReader::new(open_file);
      let mut shebang_line = String::new();
      buffered_file.read_line(&mut shebang_line).await?;

      let mut command_begin: Vec<String> = (&shebang_line[2..])
        .replace("\r", "")
        .replace("\n", "")
        .split(" ")
        .map(|s| s.to_owned())
        .collect();
      command_begin.push(execute_pathbuf.to_string_lossy().to_string());
      Ok(command_begin)
    }
    _ => {
      // It's not executable
      Err(anyhow::anyhow!("The CGI program is not executable"))?
    }
  }
}
