use std::collections::HashMap;
use std::env;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
use monoio::io::IntoPollIo;
#[cfg(feature = "runtime-monoio")]
use monoio::net::TcpStream;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
#[cfg(feature = "runtime-tokio")]
use tokio::net::TcpStream;
#[cfg(feature = "runtime-tokio")]
use tokio_util::io::ReaderStream;
use tokio_util::io::StreamReader;

use crate::config::ServerConfiguration;
use crate::logging::ErrorLogger;
use crate::modules::{Module, ModuleHandlers, ModuleLoader, RequestData, ResponseData, SocketData};
use crate::util::cgi::CgiResponse;
#[cfg(feature = "runtime-monoio")]
use crate::util::SendReadStream;
use crate::util::{
  get_entries, get_entries_for_validation, get_entry, get_value, Copier, ModuleCache,
  SERVER_SOFTWARE,
};

/// A SCGI module loader
pub struct ScgiModuleLoader {
  cache: ModuleCache<ScgiModule>,
}

impl ScgiModuleLoader {
  /// Creates a new module loader
  pub fn new() -> Self {
    Self {
      cache: ModuleCache::new(vec![]),
    }
  }
}

impl ModuleLoader for ScgiModuleLoader {
  fn load_module(
    &mut self,
    config: &ServerConfiguration,
    _global_config: Option<&ServerConfiguration>,
  ) -> Result<Arc<dyn Module + Send + Sync>, Box<dyn Error + Send + Sync>> {
    Ok(
      self
        .cache
        .get_or_init::<_, Box<dyn std::error::Error + Send + Sync>>(config, move |_| {
          Ok(Arc::new(ScgiModule))
        })?,
    )
  }

  fn get_requirements(&self) -> Vec<&'static str> {
    vec!["scgi"]
  }

  fn validate_configuration(
    &self,
    config: &ServerConfiguration,
    used_properties: &mut std::collections::HashSet<String>,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    if let Some(entries) = get_entries_for_validation!("scgi", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `scgi` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_string() && !entry.values[0].is_null() {
          Err(anyhow::anyhow!("The SCGI server base URL must be a string"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("scgi_environment", config, used_properties)
    {
      for entry in &entries.inner {
        if entry.values.len() != 2 {
          Err(anyhow::anyhow!(
            "The `scgi_environment` configuration property must have exactly two values"
          ))?
        } else if !entry.values[0].is_string() {
          Err(anyhow::anyhow!(
            "The SCGI environment variable name must be a string"
          ))?
        } else if !entry.values[1].is_string() {
          Err(anyhow::anyhow!(
            "The SCGI environment variable value must be a string"
          ))?
        }
      }
    };

    Ok(())
  }
}

/// A SCGI module
struct ScgiModule;

impl Module for ScgiModule {
  fn get_module_handlers(&self) -> Box<dyn ModuleHandlers> {
    Box::new(ScgiModuleHandlers)
  }
}

/// Handlers for the SCGI module
struct ScgiModuleHandlers;

#[async_trait(?Send)]
impl ModuleHandlers for ScgiModuleHandlers {
  async fn request_handler(
    &mut self,
    request: Request<BoxBody<Bytes, std::io::Error>>,
    config: &ServerConfiguration,
    socket_data: &SocketData,
    error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    if let Some(scgi_to) = get_entry!("scgi", config)
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
      let execute_pathbuf = joined_pathbuf;
      let execute_path_info = request_path.strip_prefix("/").map(|s| s.to_string());

      let mut additional_environment_variables = HashMap::new();
      if let Some(additional_environment_variables_config) =
        get_entries!("scgi_environment", config)
      {
        for additional_variable in additional_environment_variables_config.inner.iter() {
          if let Some(key) = additional_variable.values.first().and_then(|v| v.as_str()) {
            if let Some(value) = additional_variable.values.get(1).and_then(|v| v.as_str()) {
              additional_environment_variables.insert(key.to_string(), value.to_string());
            }
          }
        }
      }

      return execute_scgi_with_environment_variables(
        request,
        socket_data,
        error_logger,
        wwwroot,
        execute_pathbuf,
        execute_path_info,
        get_value!("server_administrator_email", config).and_then(|v| v.as_str()),
        scgi_to,
        additional_environment_variables,
      )
      .await;
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
async fn execute_scgi_with_environment_variables(
  request: Request<BoxBody<Bytes, std::io::Error>>,
  socket_data: &SocketData,
  error_logger: &ErrorLogger,
  wwwroot: &Path,
  execute_pathbuf: PathBuf,
  path_info: Option<String>,
  server_administrator_email: Option<&str>,
  scgi_to: &str,
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
  if let Some(host) = request.headers().get(header::HOST) {
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
  environment_variables.insert("REQUEST_METHOD".to_string(), request.method().to_string());
  environment_variables.insert("GATEWAY_INTERFACE".to_string(), "CGI/1.1".to_string());
  environment_variables.insert("SCGI".to_string(), "1".to_string());
  environment_variables.insert(
    "REQUEST_URI".to_string(),
    format!(
      "{}{}",
      original_request_uri.path(),
      match original_request_uri.query() {
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

  let mut content_length_set = false;
  for (header_name, header_value) in request.headers().iter() {
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

  if !content_length_set {
    environment_variables.insert("CONTENT_LENGTH".to_string(), "0".to_string());
  }

  for (env_var_key, env_var_value) in additional_environment_variables {
    if let hashlink::linked_hash_map::Entry::Vacant(entry) =
      environment_variables.entry(env_var_key)
    {
      entry.insert(env_var_value);
    }
  }

  execute_scgi(request, error_logger, scgi_to, environment_variables).await
}

async fn execute_scgi(
  request: Request<BoxBody<Bytes, std::io::Error>>,
  error_logger: &ErrorLogger,
  scgi_to: &str,
  mut environment_variables: LinkedHashMap<String, String>,
) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
  let (_, body) = request.into_parts();

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
          std::io::ErrorKind::ConnectionRefused
          | std::io::ErrorKind::NotFound
          | std::io::ErrorKind::HostUnreachable => {
            error_logger
              .log(&format!("Service unavailable: {}", err))
              .await;
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
      let path = scgi_to_url.path();
      match connect_unix(path).await {
        Ok(data) => data,
        Err(err) => match err.kind() {
          std::io::ErrorKind::ConnectionRefused
          | std::io::ErrorKind::NotFound
          | std::io::ErrorKind::HostUnreachable => {
            error_logger
              .log(&format!("Service unavailable: {}", err))
              .await;
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

  let cgi_stdin_reader = StreamReader::new(body.into_data_stream().map_err(std::io::Error::other));

  // Emulated standard input and standard output
  // SCGI doesn't support standard error
  let stdin = socket_writer;
  let stdout = socket_reader;

  let mut cgi_response = CgiResponse::new(stdout);

  crate::runtime::spawn(Copier::new(cgi_stdin_reader, stdin).copy());

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

  #[cfg(feature = "runtime-monoio")]
  let reader_stream = SendReadStream::new(cgi_response);
  #[cfg(feature = "runtime-tokio")]
  let reader_stream = ReaderStream::new(cgi_response);
  let stream_body = StreamBody::new(reader_stream.map_ok(Frame::data));
  let boxed_body = stream_body.boxed();

  let response = response_builder.body(boxed_body)?;

  Ok(ResponseData {
    request: None,
    response: Some(response),
    response_status: None,
    response_headers: None,
    new_remote_address: None,
  })
}

#[cfg(feature = "runtime-monoio")]
async fn connect_tcp(
  addr: &str,
) -> Result<(Box<dyn AsyncRead + Unpin>, Box<dyn AsyncWrite + Unpin>), std::io::Error> {
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
async fn connect_unix(
  path: &str,
) -> Result<(Box<dyn AsyncRead + Unpin>, Box<dyn AsyncWrite + Unpin>), std::io::Error> {
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
