// SCGI handler code inspired by SVR.JS's OrangeCircle mod, translated from JavaScript to Rust.
// Based on the "cgi" module
use std::env;
use std::error::Error;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use futures_util::TryStreamExt;
use hashlink::LinkedHashMap;
use http_body_util::{BodyExt, StreamBody};
use httparse::EMPTY_HEADER;
use hyper::body::Frame;
use hyper::{header, Response, StatusCode};
use project_karpacz_common::{
  ErrorLogger, HyperRequest, HyperResponse, RequestData, ResponseData, ServerConfig,
  ServerConfigRoot, ServerModule, ServerModuleHandlers, SocketData,
};
use project_karpacz_common::{HyperUpgraded, WithRuntime};
use tokio::fs;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::runtime::Handle;
use tokio_util::io::ReaderStream;

use crate::project_karpacz_res::server_software::SERVER_SOFTWARE;
use crate::project_karpacz_util::cgi_response::CgiResponse;
use crate::project_karpacz_util::cgi_stdin_reader::CgiStdinReader;
use crate::project_karpacz_util::copy_move::Copy;

pub fn server_module_init(
  _config: &ServerConfig,
) -> Result<Box<dyn ServerModule + Send + Sync>, Box<dyn Error + Send + Sync>> {
  Ok(Box::new(ScgiModule::new()))
}

struct ScgiModule;

impl ScgiModule {
  fn new() -> Self {
    ScgiModule
  }
}

impl ServerModule for ScgiModule {
  fn get_handlers(&self, handle: Handle) -> Box<dyn ServerModuleHandlers + Send> {
    Box::new(ScgiModuleHandlers { handle })
  }
}
struct ScgiModuleHandlers {
  handle: Handle,
}

#[async_trait]
impl ServerModuleHandlers for ScgiModuleHandlers {
  async fn request_handler(
    &mut self,
    request: RequestData,
    config: &ServerConfigRoot,
    socket_data: &SocketData,
    error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    WithRuntime::new(self.handle.clone(), async move {
      let mut scgi_to = "tcp://localhost:4000/";
      let scgi_to_yaml = config.get("scgiTo");
      if let Some(scgi_to_obtained) = scgi_to_yaml.as_str() {
        scgi_to = scgi_to_obtained;
      }

      let mut scgi_path = None;
      if let Some(scgi_path_obtained) = config.get("scgiPath").as_str() {
        scgi_path = Some(scgi_path_obtained.to_string());
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

      if let Some(scgi_path) = scgi_path {
        let mut canonical_scgi_path: &str = &scgi_path;
        if canonical_scgi_path.bytes().last() == Some(b'/') {
          canonical_scgi_path = &canonical_scgi_path[..(canonical_scgi_path.len() - 1)];
        }

        let request_path_with_slashes = match request_path == canonical_scgi_path {
          true => format!("{}/", request_path),
          false => request_path.to_string(),
        };
        if let Some(stripped_request_path) =
          request_path_with_slashes.strip_prefix(canonical_scgi_path)
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
          let execute_pathbuf = joined_pathbuf;
          let execute_path_info = stripped_request_path
            .strip_prefix("/")
            .map(|s| s.to_string());

          return execute_scgi_with_environment_variables(
            request,
            socket_data,
            error_logger,
            wwwroot,
            execute_pathbuf,
            execute_path_info,
            config.get("serverAdministratorEmail").as_str(),
            scgi_to,
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
}

#[allow(clippy::too_many_arguments)]
async fn execute_scgi_with_environment_variables(
  request: RequestData,
  socket_data: &SocketData,
  error_logger: &ErrorLogger,
  wwwroot: &Path,
  execute_pathbuf: PathBuf,
  path_info: Option<String>,
  server_administrator_email: Option<&str>,
  scgi_to: &str,
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
  environment_variables.insert("SCGI".to_string(), "1".to_string());
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

  execute_scgi(hyper_request, error_logger, scgi_to, environment_variables).await
}

async fn execute_scgi(
  hyper_request: HyperRequest,
  error_logger: &ErrorLogger,
  scgi_to: &str,
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

  let scgi_to_fixed = if let Some(stripped) = scgi_to.strip_prefix("unix:///") {
    // hyper::Uri fails to parse a string if there is an empty authority, so add an "ignore" authority to Unix socket URLs
    &format!("unix://ignore/{}", stripped)
  } else {
    scgi_to
  };

  let scgi_to_url = scgi_to_fixed.parse::<hyper::Uri>()?;
  let scheme_str = scgi_to_url.scheme_str();

  let (socket_reader, mut socket_writer) = match scheme_str {
    Some("tcp") => {
      let host = match scgi_to_url.host() {
        Some(host) => host,
        None => Err(anyhow::anyhow!("The SCGI URL doesn't include the host"))?,
      };

      let port = match scgi_to_url.port_u16() {
        Some(port) => port,
        None => Err(anyhow::anyhow!("The SCGI URL doesn't include the port"))?,
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
      let path = scgi_to_url.path();
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

  // Create environment variable netstring
  let mut environment_variables_to_wrap = Vec::new();
  for (key, value) in environment_variables.iter() {
    let mut environment_variable = Vec::new();
    environment_variable.extend_from_slice(key.as_bytes());
    environment_variable.push(b'\0');
    environment_variable.extend_from_slice(value.as_bytes());
    environment_variable.push(b'\0');
    if key == "CONTENT_LENGTH" {
      environment_variable.append(&mut environment_variables_to_wrap);
      environment_variables_to_wrap = environment_variable;
    } else {
      environment_variables_to_wrap.append(&mut environment_variable);
    }
  }

  let environment_variables_to_wrap_length = environment_variables_to_wrap.len();
  let mut environment_variables_netstring = Vec::new();
  environment_variables_netstring
    .extend_from_slice(environment_variables_to_wrap_length.to_string().as_bytes());
  environment_variables_netstring.push(b':');
  environment_variables_netstring.append(&mut environment_variables_to_wrap);
  environment_variables_netstring.push(b',');

  // Write environment variable netstring
  socket_writer
    .write_all(&environment_variables_netstring)
    .await?;

  let cgi_stdin_reader = CgiStdinReader::new(body);

  // Emulated standard input and standard output
  // SCGI doesn't support standard error
  let stdin = socket_writer;
  let stdout = socket_reader;

  let mut cgi_response = CgiResponse::new(stdout);

  let stdin_copy_future = Copy::new(cgi_stdin_reader, stdin);
  let mut stdin_copy_future_pinned = Box::pin(stdin_copy_future);

  let mut headers = [EMPTY_HEADER; 128];

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

  Ok(
    ResponseData::builder_without_request()
      .response(response)
      .parallel_fn(async move {
        stdin_copy_future_pinned.await.unwrap_or_default();
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
