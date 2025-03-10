// FastCGI handler code inspired by SVR.JS's GreenRhombus mod, translated from JavaScript to Rust.
// Based on the "cgi" and "scgi" module
use std::env;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use ferron_common::{
  ErrorLogger, HyperRequest, HyperResponse, RequestData, ResponseData, ServerConfig,
  ServerConfigRoot, ServerModule, ServerModuleHandlers, SocketData,
};
use ferron_common::{HyperUpgraded, WithRuntime};
use futures_util::future::Either;
use futures_util::TryStreamExt;
use hashlink::LinkedHashMap;
use http_body_util::{BodyExt, StreamBody};
use httparse::EMPTY_HEADER;
use hyper::body::{Bytes, Frame};
use hyper::{header, Response, StatusCode};
use hyper_tungstenite::HyperWebsocket;
use tokio::fs;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::runtime::Handle;
use tokio::sync::RwLock;
use tokio_util::codec::{FramedRead, FramedWrite};
use tokio_util::io::{ReaderStream, SinkWriter, StreamReader};

use crate::ferron_res::server_software::SERVER_SOFTWARE;
use crate::ferron_util::cgi_response::CgiResponse;
use crate::ferron_util::copy_move::Copier;
use crate::ferron_util::fcgi_decoder::{FcgiDecodedData, FcgiDecoder};
use crate::ferron_util::fcgi_encoder::FcgiEncoder;
use crate::ferron_util::fcgi_name_value_pair::construct_fastcgi_name_value_pair;
use crate::ferron_util::fcgi_record::construct_fastcgi_record;
use crate::ferron_util::read_to_end_move::ReadToEndFuture;
use crate::ferron_util::split_stream_by_map::SplitStreamByMapExt;
use crate::ferron_util::ttl_cache::TtlCache;

pub fn server_module_init(
  _config: &ServerConfig,
) -> Result<Box<dyn ServerModule + Send + Sync>, Box<dyn Error + Send + Sync>> {
  let cache = Arc::new(RwLock::new(TtlCache::new(Duration::from_millis(100))));
  Ok(Box::new(FcgiModule::new(cache)))
}

#[allow(clippy::type_complexity)]
struct FcgiModule {
  path_cache: Arc<RwLock<TtlCache<String, (Option<PathBuf>, Option<String>)>>>,
}

impl FcgiModule {
  #[allow(clippy::type_complexity)]
  fn new(path_cache: Arc<RwLock<TtlCache<String, (Option<PathBuf>, Option<String>)>>>) -> Self {
    FcgiModule { path_cache }
  }
}

impl ServerModule for FcgiModule {
  fn get_handlers(&self, handle: Handle) -> Box<dyn ServerModuleHandlers + Send> {
    Box::new(FcgiModuleHandlers {
      path_cache: self.path_cache.clone(),
      handle,
    })
  }
}

#[allow(clippy::type_complexity)]
struct FcgiModuleHandlers {
  handle: Handle,
  path_cache: Arc<RwLock<TtlCache<String, (Option<PathBuf>, Option<String>)>>>,
}

#[async_trait]
impl ServerModuleHandlers for FcgiModuleHandlers {
  async fn request_handler(
    &mut self,
    request: RequestData,
    config: &ServerConfigRoot,
    socket_data: &SocketData,
    error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    WithRuntime::new(self.handle.clone(), async move {
      let mut fastcgi_script_exts = Vec::new();

      let fastcgi_script_exts_yaml = config.get("fcgiScriptExtensions");
      if let Some(fastcgi_script_exts_obtained) = fastcgi_script_exts_yaml.as_vec() {
        for fastcgi_script_ext_yaml in fastcgi_script_exts_obtained.iter() {
          if let Some(fastcgi_script_ext) = fastcgi_script_ext_yaml.as_str() {
            fastcgi_script_exts.push(fastcgi_script_ext);
          }
        }
      }

      let mut fastcgi_to = "tcp://localhost:4000/";
      let fastcgi_to_yaml = config.get("fcgiTo");
      if let Some(fastcgi_to_obtained) = fastcgi_to_yaml.as_str() {
        fastcgi_to = fastcgi_to_obtained;
      }

      let mut fastcgi_path = None;
      if let Some(fastcgi_path_obtained) = config.get("fcgiPath").as_str() {
        fastcgi_path = Some(fastcgi_path_obtained.to_string());
      }

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

      let mut execute_pathbuf = None;
      let mut execute_path_info = None;
      let mut wwwroot_detected = None;

      if let Some(fastcgi_path) = fastcgi_path {
        let mut canonical_fastcgi_path: &str = &fastcgi_path;
        if canonical_fastcgi_path.bytes().last() == Some(b'/') {
          canonical_fastcgi_path = &canonical_fastcgi_path[..(canonical_fastcgi_path.len() - 1)];
        }

        let request_path_with_slashes = match request_path == canonical_fastcgi_path {
          true => format!("{}/", request_path),
          false => request_path.to_string(),
        };
        if let Some(stripped_request_path) =
          request_path_with_slashes.strip_prefix(canonical_fastcgi_path)
        {
          let wwwroot_yaml = config.get("wwwroot");
          let wwwroot = wwwroot_yaml.as_str().unwrap_or("/nonexistent");

          let wwwroot_unknown = PathBuf::from(wwwroot);
          let wwwroot_pathbuf = match wwwroot_unknown.as_path().is_absolute() {
            true => wwwroot_unknown,
            false => match fs::canonicalize(&wwwroot_unknown).await {
              Ok(pathbuf) => pathbuf,
              Err(_) => wwwroot_unknown,
            },
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
              return Ok(
                ResponseData::builder(request)
                  .status(StatusCode::BAD_REQUEST)
                  .build(),
              );
            }
          };

          let joined_pathbuf = wwwroot.join(decoded_relative_path);
          execute_pathbuf = Some(joined_pathbuf);
          execute_path_info = stripped_request_path
            .strip_prefix("/")
            .map(|s| s.to_string());
        }
      }

      if execute_pathbuf.is_none() {
        if let Some(wwwroot) = config.get("wwwroot").as_str() {
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
                    let contained_extension = joined_pathbuf
                      .extension()
                      .map(|a| format!(".{}", a.to_string_lossy()));
                    if let Some(contained_extension) = contained_extension {
                      if fastcgi_script_exts.contains(&(&contained_extension as &str)) {
                        execute_pathbuf = Some(joined_pathbuf);
                      }
                    }
                  } else if metadata.is_dir() {
                    let indexes = vec!["index.php", "index.cgi"];
                    for index in indexes {
                      let temp_joined_pathbuf = joined_pathbuf.join(index);
                      match fs::metadata(&temp_joined_pathbuf).await {
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

          execute_pathbuf = execute_pathbuf_got;
          execute_path_info = execute_path_info_got;
        }
      }

      if let Some(execute_pathbuf) = execute_pathbuf {
        if let Some(wwwroot_detected) = wwwroot_detected {
          return execute_fastcgi_with_environment_variables(
            request,
            socket_data,
            error_logger,
            wwwroot_detected.as_path(),
            execute_pathbuf,
            execute_path_info,
            config.get("serverAdministratorEmail").as_str(),
            fastcgi_to,
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
    _config: &ServerConfigRoot,
    _socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    Ok(())
  }

  fn does_websocket_requests(&mut self, _config: &ServerConfigRoot) -> bool {
    false
  }
}

#[allow(clippy::too_many_arguments)]
async fn execute_fastcgi_with_environment_variables(
  request: RequestData,
  socket_data: &SocketData,
  error_logger: &ErrorLogger,
  wwwroot: &Path,
  execute_pathbuf: PathBuf,
  path_info: Option<String>,
  server_administrator_email: Option<&str>,
  fastcgi_to: &str,
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

  let mut content_length_set = false;
  for (header_name, header_value) in hyper_request.headers().iter() {
    let env_header_name = match *header_name {
      header::CONTENT_LENGTH => {
        content_length_set = true;
        "CONTENT_LENGTH".to_string()
      }
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

  if !content_length_set {
    environment_variables.insert("CONTENT_LENGTH".to_string(), "0".to_string());
  }

  let (hyper_request, _) = request.into_parts();

  execute_fastcgi(
    hyper_request,
    error_logger,
    fastcgi_to,
    environment_variables,
  )
  .await
}

async fn execute_fastcgi(
  hyper_request: HyperRequest,
  error_logger: &ErrorLogger,
  fastcgi_to: &str,
  mut environment_variables: LinkedHashMap<String, String>,
) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
  let (_, body) = hyper_request.into_parts();

  // Insert other environment variables
  for (key, value) in env::vars_os() {
    let key_string = key.to_string_lossy().to_string();
    let value_string = value.to_string_lossy().to_string();
    environment_variables
      .entry(key_string)
      .or_insert(value_string);
  }

  let fastcgi_to_fixed = if let Some(stripped) = fastcgi_to.strip_prefix("unix:///") {
    // hyper::Uri fails to parse a string if there is an empty authority, so add an "ignore" authority to Unix socket URLs
    &format!("unix://ignore/{}", stripped)
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

      let addr = format!("{}:{}", host, port);

      match connect_tcp(&addr).await {
        Ok(data) => data,
        Err(err) => match err.kind() {
          tokio::io::ErrorKind::ConnectionRefused
          | tokio::io::ErrorKind::NotFound
          | tokio::io::ErrorKind::HostUnreachable => {
            error_logger
              .log(&format!("Service unavailable: {}", err))
              .await;
            return Ok(
              ResponseData::builder_without_request()
                .status(StatusCode::SERVICE_UNAVAILABLE)
                .build(),
            );
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
          tokio::io::ErrorKind::ConnectionRefused
          | tokio::io::ErrorKind::NotFound
          | tokio::io::ErrorKind::HostUnreachable => {
            error_logger
              .log(&format!("Service unavailable: {}", err))
              .await;
            return Ok(
              ResponseData::builder_without_request()
                .status(StatusCode::SERVICE_UNAVAILABLE)
                .build(),
            );
          }
          _ => Err(err)?,
        },
      }
    }
    _ => Err(anyhow::anyhow!(
      "Only HTTP and HTTPS reverse proxy URLs are supported."
    ))?,
  };

  // Construct and send BEGIN_REQUEST record
  // Use the responder role and don't use keep-alive
  let begin_request_packet = construct_fastcgi_record(1, 1, &[0, 1, 0, 0, 0, 0, 0, 0]);
  socket_writer.write_all(&begin_request_packet).await?;

  // Construct and send PARAMS records
  let mut environment_variables_to_wrap = Vec::new();
  for (key, value) in environment_variables.iter() {
    let mut environment_variable =
      construct_fastcgi_name_value_pair(key.as_bytes(), value.as_bytes());
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

  let cgi_stdin_reader = StreamReader::new(
    body
      .into_data_stream()
      .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err)),
  );

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

  let stdin_copy_future = Copier::with_zero_packet_writing(cgi_stdin_reader, stdin).copy();
  let mut stdin_copy_future_pinned = Box::pin(stdin_copy_future);

  let stderr_read_future = ReadToEndFuture::new(stderr);
  let mut stderr_read_future_pinned = Box::pin(stderr_read_future);

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

      result = &mut stdin_copy_future_pinned => {
        early_stdin_copied = true;
        result?;
      },
      obtained_head = &mut stdout_parse_future => {
        let obtained_head = obtained_head?;
        if !obtained_head.is_empty() {
          httparse::parse_headers(obtained_head, &mut headers)?;
        }
        head_obtained = true;
      },
      result = &mut stderr_read_future_pinned => {
        let stderr_vec = result?;
          let stderr_string = String::from_utf8_lossy(stderr_vec.as_slice()).to_string();
          if !stderr_string.is_empty() {
            error_logger
              .log(&format!("There were CGI errors: {}", stderr_string))
              .await;
          }
        return Ok(
          ResponseData::builder_without_request()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .build(),
        );
      },
    }

    if !head_obtained {
      // Kept it same as in the tokio::select macro
      tokio::select! {
        biased;

        result = &mut stderr_read_future_pinned => {
          let stderr_vec = result?;
            let stderr_string = String::from_utf8_lossy(stderr_vec.as_slice()).to_string();
            if !stderr_string.is_empty() {
              error_logger
                .log(&format!("There were FastCGI errors: {}", stderr_string))
                .await;
            }
          return Ok(
            ResponseData::builder_without_request()
              .status(StatusCode::INTERNAL_SERVER_ERROR)
              .build(),
          );
        },
        obtained_head = &mut stdout_parse_future => {
          let obtained_head = obtained_head?;
          if !obtained_head.is_empty() {
            httparse::parse_headers(obtained_head, &mut headers)?;
          }
        }
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

  let error_logger = error_logger.clone();

  Ok(
    ResponseData::builder_without_request()
      .response(response)
      .parallel_fn(async move {
        let mut stdin_copied = early_stdin_copied;

        if !stdin_copied {
          tokio::select! {

          biased;

          _ = &mut stdin_copy_future_pinned => {
            stdin_copied = true;
          },
            result = &mut stderr_read_future_pinned => {
              let stderr_vec = result.unwrap_or(vec![]);
              let stderr_string = String::from_utf8_lossy(stderr_vec.as_slice()).to_string();
              if !stderr_string.is_empty() {
                error_logger
                  .log(&format!("There were FastCGI errors: {}", stderr_string))
                  .await;
              }
            },
          }
        }

        if stdin_copied {
          let stderr_vec = stderr_read_future_pinned.await.unwrap_or(vec![]);
          let stderr_string = String::from_utf8_lossy(stderr_vec.as_slice()).to_string();
          if !stderr_string.is_empty() {
            error_logger
              .log(&format!("There were FastCGI errors: {}", stderr_string))
              .await;
          }
        } else {
          stdin_copy_future_pinned.await.unwrap_or_default();
        }
      })
      .build(),
  )
}

async fn connect_tcp(
  addr: &str,
) -> Result<
  (
    Box<dyn AsyncRead + Send + Sync + Unpin>,
    Box<dyn AsyncWrite + Send + Sync + Unpin>,
  ),
  tokio::io::Error,
> {
  let socket = TcpStream::connect(addr).await?;
  socket.set_nodelay(true)?;

  let (socket_reader_set, socket_writer_set) = tokio::io::split(socket);
  Ok((Box::new(socket_reader_set), Box::new(socket_writer_set)))
}

#[allow(dead_code)]
#[cfg(unix)]
async fn connect_unix(
  path: &str,
) -> Result<
  (
    Box<dyn AsyncRead + Send + Sync + Unpin>,
    Box<dyn AsyncWrite + Send + Sync + Unpin>,
  ),
  tokio::io::Error,
> {
  use tokio::net::UnixStream;

  let socket = UnixStream::connect(path).await?;

  let (socket_reader_set, socket_writer_set) = tokio::io::split(socket);
  Ok((Box::new(socket_reader_set), Box::new(socket_writer_set)))
}

#[allow(dead_code)]
#[cfg(not(unix))]
async fn connect_unix(
  _path: &str,
) -> Result<
  (
    Box<dyn AsyncRead + Send + Sync + Unpin>,
    Box<dyn AsyncWrite + Send + Sync + Unpin>,
  ),
  tokio::io::Error,
> {
  Err(tokio::io::Error::new(
    tokio::io::ErrorKind::Unsupported,
    "Unix sockets are not supports on non-Unix platforms.",
  ))
}
