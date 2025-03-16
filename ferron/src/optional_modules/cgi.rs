// CGI handler code inspired by SVR.JS's RedBrick mod, translated from JavaScript to Rust.
use std::collections::HashMap;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use ferron_common::{
  ErrorLogger, HyperRequest, HyperResponse, RequestData, ResponseData, ServerConfig,
  ServerConfigRoot, ServerModule, ServerModuleHandlers, SocketData,
};
use ferron_common::{HyperUpgraded, WithRuntime};
use futures_util::TryStreamExt;
use hashlink::LinkedHashMap;
use http_body_util::{BodyExt, StreamBody};
use httparse::EMPTY_HEADER;
use hyper::body::Frame;
use hyper::{header, Response, StatusCode};
use hyper_tungstenite::HyperWebsocket;
use tokio::fs;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::runtime::Handle;
use tokio::sync::RwLock;
use tokio_util::io::{ReaderStream, StreamReader};

use crate::ferron_res::server_software::SERVER_SOFTWARE;
use crate::ferron_util::cgi_response::CgiResponse;
use crate::ferron_util::copy_move::Copier;
use crate::ferron_util::ttl_cache::TtlCache;

pub fn server_module_init(
  _config: &ServerConfig,
) -> Result<Box<dyn ServerModule + Send + Sync>, Box<dyn Error + Send + Sync>> {
  let cache = Arc::new(RwLock::new(TtlCache::new(Duration::from_millis(100))));
  Ok(Box::new(CgiModule::new(cache)))
}

#[allow(clippy::type_complexity)]
struct CgiModule {
  path_cache: Arc<RwLock<TtlCache<String, (Option<PathBuf>, Option<String>)>>>,
}

impl CgiModule {
  #[allow(clippy::type_complexity)]
  fn new(path_cache: Arc<RwLock<TtlCache<String, (Option<PathBuf>, Option<String>)>>>) -> Self {
    CgiModule { path_cache }
  }
}

impl ServerModule for CgiModule {
  fn get_handlers(&self, handle: Handle) -> Box<dyn ServerModuleHandlers + Send> {
    Box::new(CgiModuleHandlers {
      path_cache: self.path_cache.clone(),
      handle,
    })
  }
}

#[allow(clippy::type_complexity)]
struct CgiModuleHandlers {
  handle: Handle,
  path_cache: Arc<RwLock<TtlCache<String, (Option<PathBuf>, Option<String>)>>>,
}

#[async_trait]
impl ServerModuleHandlers for CgiModuleHandlers {
  async fn request_handler(
    &mut self,
    request: RequestData,
    config: &ServerConfigRoot,
    socket_data: &SocketData,
    error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    WithRuntime::new(self.handle.clone(), async move {
      let mut cgi_script_exts = Vec::new();

      let cgi_script_exts_yaml = config.get("cgiScriptExtensions");
      if let Some(cgi_script_exts_obtained) = cgi_script_exts_yaml.as_vec() {
        for cgi_script_ext_yaml in cgi_script_exts_obtained.iter() {
          if let Some(cgi_script_ext) = cgi_script_ext_yaml.as_str() {
            cgi_script_exts.push(cgi_script_ext);
          }
        }
      }

      if let Some(wwwroot) = config.get("wwwroot").as_str() {
        let hyper_request = request.get_hyper_request();

        let request_path = hyper_request.uri().path();
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

        let wwwroot_unknown = PathBuf::from(wwwroot);
        let wwwroot_pathbuf = match wwwroot_unknown.as_path().is_absolute() {
          true => wwwroot_unknown,
          false => match fs::canonicalize(&wwwroot_unknown).await {
            Ok(pathbuf) => pathbuf,
            Err(_) => wwwroot_unknown,
          },
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
                return Ok(
                  ResponseData::builder(request)
                    .status(StatusCode::BAD_REQUEST)
                    .build(),
                );
              }
            };

            let joined_pathbuf = wwwroot.join(decoded_relative_path);
            let mut execute_pathbuf: Option<PathBuf> = None;
            let mut execute_path_info: Option<String> = None;

            match fs::metadata(&joined_pathbuf).await {
              Ok(metadata) => {
                if metadata.is_file() {
                  let mut request_path_normalized = match cfg!(windows) {
                    true => request_path.to_lowercase(),
                    false => request_path.to_string(),
                  };
                  while request_path_normalized.contains("//") {
                    request_path_normalized = request_path_normalized.replace("//", "/");
                  }
                  if request_path_normalized == "/cgi-bin"
                    || request_path_normalized.starts_with("/cgi-bin/")
                  {
                    execute_pathbuf = Some(joined_pathbuf);
                  } else {
                    let contained_extension = joined_pathbuf
                      .extension()
                      .map(|a| format!(".{}", a.to_string_lossy()));
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
                    match fs::metadata(&temp_joined_pathbuf).await {
                      Ok(temp_metadata) => {
                        if temp_metadata.is_file() {
                          let request_path_normalized = match cfg!(windows) {
                            true => request_path.to_lowercase(),
                            false => request_path.to_string(),
                          };
                          if request_path_normalized == "/cgi-bin"
                            || request_path_normalized.starts_with("/cgi-bin/")
                          {
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
                if err.kind() == tokio::io::ErrorKind::NotADirectory {
                  // TODO: find a file
                  let mut temp_pathbuf = joined_pathbuf.clone();
                  loop {
                    if !temp_pathbuf.pop() {
                      break;
                    }
                    match fs::metadata(&temp_pathbuf).await {
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
                          if request_path_normalized == "/cgi-bin"
                            || request_path_normalized.starts_with("/cgi-bin/")
                          {
                            execute_pathbuf = Some(temp_pathbuf);
                            execute_path_info = path_info;
                            break;
                          } else {
                            let contained_extension = temp_pathbuf
                              .extension()
                              .map(|a| format!(".{}", a.to_string_lossy()));
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
                        tokio::io::ErrorKind::NotADirectory => (),
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
            cgi_interpreters.insert(
              ".bat".to_string(),
              vec!["cmd".to_string(), "/c".to_string()],
            );
            cgi_interpreters.insert(".vbs".to_string(), vec!["cscript".to_string()]);
          }

          let cgi_interpreters_yaml = config.get("cgiScriptInterpreters");
          if let Some(cgi_interpreters_hashmap) = cgi_interpreters_yaml.as_hash() {
            for (key_yaml, value_yaml) in cgi_interpreters_hashmap.iter() {
              if let Some(key) = key_yaml.as_str() {
                if value_yaml.is_null() {
                  cgi_interpreters.remove(key);
                } else if let Some(value) = value_yaml.as_vec() {
                  let mut params = Vec::new();
                  for param_yaml in value.iter() {
                    if let Some(param) = param_yaml.as_str() {
                      params.push(param.to_string());
                    }
                  }
                  cgi_interpreters.insert(key.to_string(), params);
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
            config.get("serverAdministratorEmail").as_str(),
            cgi_interpreters,
          )
          .await;
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

#[allow(clippy::too_many_arguments)]
async fn execute_cgi_with_environment_variables(
  request: RequestData,
  socket_data: &SocketData,
  error_logger: &ErrorLogger,
  wwwroot: &Path,
  execute_pathbuf: PathBuf,
  path_info: Option<String>,
  server_administrator_email: Option<&str>,
  cgi_interpreters: HashMap<String, Vec<String>>,
) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
  let mut environment_variables: LinkedHashMap<String, String> = LinkedHashMap::new();

  let hyper_request = request.get_hyper_request();
  if let Some(auth_user) = request.get_auth_user() {
    if let Some(authorization) = hyper_request.headers().get(header::AUTHORIZATION) {
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
    match hyper_request.uri().query() {
      Some(query) => query.to_string(),
      None => "".to_string(),
    },
  );

  environment_variables.insert("SERVER_SOFTWARE".to_string(), SERVER_SOFTWARE.to_string());
  environment_variables.insert(
    "SERVER_PROTOCOL".to_string(),
    match hyper_request.version() {
      hyper::Version::HTTP_09 => "HTTP/0.9".to_string(),
      hyper::Version::HTTP_10 => "HTTP/1.0".to_string(),
      hyper::Version::HTTP_11 => "HTTP/1.1".to_string(),
      hyper::Version::HTTP_2 => "HTTP/2.0".to_string(),
      hyper::Version::HTTP_3 => "HTTP/3.0".to_string(),
      _ => "HTTP/Unknown".to_string(),
    },
  );
  environment_variables.insert(
    "SERVER_PORT".to_string(),
    socket_data.local_addr.port().to_string(),
  );
  environment_variables.insert(
    "SERVER_ADDR".to_string(),
    socket_data.local_addr.ip().to_canonical().to_string(),
  );
  if let Some(server_administrator_email) = server_administrator_email {
    environment_variables.insert(
      "SERVER_ADMIN".to_string(),
      server_administrator_email.to_string(),
    );
  }
  if let Some(host) = hyper_request.headers().get(header::HOST) {
    environment_variables.insert(
      "SERVER_NAME".to_string(),
      String::from_utf8_lossy(host.as_bytes()).to_string(),
    );
  }

  environment_variables.insert(
    "DOCUMENT_ROOT".to_string(),
    wwwroot.to_string_lossy().to_string(),
  );
  environment_variables.insert(
    "PATH_INFO".to_string(),
    match &path_info {
      Some(path_info) => format!("/{}", path_info),
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
  environment_variables.insert(
    "REQUEST_METHOD".to_string(),
    hyper_request.method().to_string(),
  );
  environment_variables.insert("GATEWAY_INTERFACE".to_string(), "CGI/1.1".to_string());
  environment_variables.insert(
    "REQUEST_URI".to_string(),
    format!(
      "{}{}",
      hyper_request.uri().path(),
      match hyper_request.uri().query() {
        Some(query) => format!("?{}", query),
        None => String::from(""),
      }
    ),
  );

  environment_variables.insert(
    "REMOTE_PORT".to_string(),
    socket_data.remote_addr.port().to_string(),
  );
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

  for (header_name, header_value) in hyper_request.headers().iter() {
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
    environment_variables.insert(
      env_header_name,
      String::from_utf8_lossy(header_value.as_bytes()).to_string(),
    );
  }

  let (hyper_request, _) = request.into_parts();

  execute_cgi(
    hyper_request,
    error_logger,
    execute_pathbuf,
    cgi_interpreters,
    environment_variables,
  )
  .await
}

async fn execute_cgi(
  hyper_request: HyperRequest,
  error_logger: &ErrorLogger,
  execute_pathbuf: PathBuf,
  cgi_interpreters: HashMap<String, Vec<String>>,
  environment_variables: LinkedHashMap<String, String>,
) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
  let (_, body) = hyper_request.into_parts();

  let executable_params = match get_executable(&execute_pathbuf).await {
    Ok(params) => params,
    Err(err) => {
      let contained_extension = execute_pathbuf
        .extension()
        .map(|a| format!(".{}", a.to_string_lossy()));
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

  let mut child = command.spawn()?;

  let cgi_stdin_reader = StreamReader::new(
    body
      .into_data_stream()
      .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err)),
  );

  let stdin = match child.stdin.take() {
    Some(stdin) => stdin,
    None => Err(anyhow::anyhow!(
      "The CGI process doesn't have standard input"
    ))?,
  };
  let stdout = match child.stdout.take() {
    Some(stdout) => stdout,
    None => Err(anyhow::anyhow!(
      "The CGI process doesn't have standard output"
    ))?,
  };
  let stderr = child.stderr.take();

  let mut cgi_response = CgiResponse::new(stdout);

  let stdin_copy_future = Copier::new(cgi_stdin_reader, stdin).copy();
  let mut stdin_copy_future_pinned = Box::pin(stdin_copy_future);

  let mut headers = [EMPTY_HEADER; 128];

  let mut early_stdin_copied = false;

  // Needed to wrap this in another scope to prevent errors with multiple mutable borrows.
  {
    let mut head_obtained = false;
    let stdout_parse_future = cgi_response.get_head();
    tokio::pin!(stdout_parse_future);

    // Cannot use a loop with tokio::select, since stdin_copy_future_pinned being constantly ready will make the web server stop responding to HTTP requests
    tokio::select! {
      biased;

      obtained_head = &mut stdout_parse_future => {
        let obtained_head = obtained_head?;
        if !obtained_head.is_empty() {
          httparse::parse_headers(obtained_head, &mut headers)?;
        }
        head_obtained = true;
      },
      result = &mut stdin_copy_future_pinned => {
        early_stdin_copied = true;
        result?;
      }
    }

    if !head_obtained {
      // Kept it same as in the tokio::select macro
      let obtained_head = stdout_parse_future.await?;
      if !obtained_head.is_empty() {
        httparse::parse_headers(obtained_head, &mut headers)?;
      }
    }
  }

  let mut response_builder = Response::builder();
  let mut status_code = 200;
  for header in headers {
    if header == EMPTY_HEADER {
      break;
    }
    match &header.name.to_lowercase() as &str {
      "location" => {
        if !(300..=399).contains(&status_code) {
          status_code = 302;
        }
      }
      "status" => {
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
    response_builder = response_builder.header(header.name, header.value);
  }

  response_builder = response_builder.status(status_code);

  let reader_stream = ReaderStream::new(cgi_response);
  let stream_body = StreamBody::new(reader_stream.map_ok(Frame::data));
  let boxed_body = stream_body.boxed();

  let response = response_builder.body(boxed_body)?;

  if let Some(exit_code) = child.try_wait()? {
    if !exit_code.success() {
      if let Some(mut stderr) = stderr {
        let mut stderr_string = String::new();
        stderr
          .read_to_string(&mut stderr_string)
          .await
          .unwrap_or_default();
        let stderr_string_trimmed = stderr_string.trim();
        if !stderr_string_trimmed.is_empty() {
          error_logger
            .log(&format!("There were CGI errors: {}", stderr_string_trimmed))
            .await;
        }
      }
      return Ok(
        ResponseData::builder_without_request()
          .status(StatusCode::INTERNAL_SERVER_ERROR)
          .build(),
      );
    }
  }

  let error_logger = error_logger.clone();

  Ok(
    ResponseData::builder_without_request()
      .response(response)
      .parallel_fn(async move {
        if !early_stdin_copied {
          stdin_copy_future_pinned.await.unwrap_or_default();
        }

        if let Some(mut stderr) = stderr {
          let mut stderr_string = String::new();
          stderr
            .read_to_string(&mut stderr_string)
            .await
            .unwrap_or_default();
          let stderr_string_trimmed = stderr_string.trim();
          if !stderr_string_trimmed.is_empty() {
            error_logger
              .log(&format!("There were CGI errors: {}", stderr_string_trimmed))
              .await;
          }
        }
      })
      .build(),
  )
}

#[allow(dead_code)]
#[cfg(unix)]
async fn get_executable(
  execute_pathbuf: &PathBuf,
) -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
  use std::os::unix::fs::PermissionsExt;

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
#[cfg(not(unix))]
async fn get_executable(
  execute_pathbuf: &PathBuf,
) -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
  use tokio::io::{AsyncBufReadExt, AsyncSeekExt, BufReader};

  let mut magic_signature_buffer = [0u8; 2];
  let mut open_file = fs::File::open(&execute_pathbuf).await?;
  if open_file
    .read_exact(&mut magic_signature_buffer)
    .await
    .is_err()
  {
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
