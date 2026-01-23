use std::collections::HashMap;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use cegla::client::{convert_to_http_response, CgiRequest};
use cegla::CgiEnvironment;
#[cfg(feature = "runtime-monoio")]
use ferron_common::util::SendAsyncIo;
use http_body_util::combinators::BoxBody;
use http_body_util::BodyExt;
use hyper::{header, Request, Response, StatusCode};
#[cfg(feature = "runtime-monoio")]
use monoio::io::IntoPollIo;
#[cfg(feature = "runtime-monoio")]
use monoio::net::TcpStream;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
#[cfg(feature = "runtime-tokio")]
use tokio::net::TcpStream;
use tokio_util::io::StreamReader;

use ferron_common::config::ServerConfiguration;
use ferron_common::logging::ErrorLogger;
use ferron_common::modules::{Module, ModuleHandlers, ModuleLoader, RequestData, ResponseData, SocketData};
use ferron_common::util::{ModuleCache, SERVER_SOFTWARE};
use ferron_common::{get_entries, get_entries_for_validation, get_entry, get_value};

/// A SCGI module loader
pub struct ScgiModuleLoader {
  cache: ModuleCache<ScgiModule>,
}

impl Default for ScgiModuleLoader {
  fn default() -> Self {
    Self::new()
  }
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
    _secondary_runtime: &tokio::runtime::Runtime,
  ) -> Result<Arc<dyn Module + Send + Sync>, Box<dyn Error + Send + Sync>> {
    Ok(
      self
        .cache
        .get_or_init::<_, Box<dyn std::error::Error + Send + Sync>>(config, move |_| Ok(Arc::new(ScgiModule)))?,
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

    if let Some(entries) = get_entries_for_validation!("scgi_environment", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 2 {
          Err(anyhow::anyhow!(
            "The `scgi_environment` configuration property must have exactly two values"
          ))?
        } else if !entry.values[0].is_string() {
          Err(anyhow::anyhow!("The SCGI environment variable name must be a string"))?
        } else if !entry.values[1].is_string() {
          Err(anyhow::anyhow!("The SCGI environment variable value must be a string"))?
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

      // Check for possible path traversal attack, if the URL sanitizer is disabled.
      if get_value!("disable_url_sanitizer", config)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
      {
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
        let canonicalize_result = tokio::fs::canonicalize(&joined_pathbuf).await;

        let canonical_joined_pathbuf = match canonicalize_result {
          Ok(pathbuf) => pathbuf,
          Err(_) => joined_pathbuf.clone(),
        };

        // Webroot is already canonicalized, so no need to canonicalize it again

        // Return 403 Forbidden if the path is outside the webroot
        if !canonical_joined_pathbuf.starts_with(wwwroot) {
          return Ok(ResponseData {
            request: Some(request),
            response: None,
            response_status: Some(StatusCode::FORBIDDEN),
            response_headers: None,
            new_remote_address: None,
          });
        }
      }

      let execute_pathbuf = joined_pathbuf;
      let execute_path_info = request_path.strip_prefix("/").map(|s| s.to_string());

      let mut additional_environment_variables = HashMap::new();
      if let Some(additional_environment_variables_config) = get_entries!("scgi_environment", config) {
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
        config.filters.hostname.as_deref(),
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
  mut request: Request<BoxBody<Bytes, std::io::Error>>,
  socket_data: &SocketData,
  error_logger: &ErrorLogger,
  wwwroot: &Path,
  execute_pathbuf: PathBuf,
  path_info: Option<String>,
  server_name: Option<&str>,
  server_administrator_email: Option<&str>,
  scgi_to: &str,
  additional_environment_variables: HashMap<String, String>,
) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
  let request_data = request.extensions_mut().remove::<RequestData>();

  let original_request_uri = request_data
    .as_ref()
    .and_then(|d| d.original_url.as_ref())
    .unwrap_or(request.uri());
  let mut env_builder = cegla::client::CgiBuilder::new();

  if let Some(auth_user) = request_data.as_ref().and_then(|u| u.auth_user.as_ref()) {
    let authorization_type = if let Some(authorization) = request.headers().get(header::AUTHORIZATION) {
      let authorization_value = String::from_utf8_lossy(authorization.as_bytes()).to_string();
      let mut authorization_value_split = authorization_value.split(" ");
      authorization_value_split
        .next()
        .map(|authorization_type| authorization_type.to_string())
    } else {
      None
    };
    env_builder = env_builder.auth(authorization_type, auth_user.to_string());
  }

  if let Some(server_administrator_email) = server_administrator_email {
    env_builder = env_builder.server_admin(server_administrator_email.to_string());
  }

  if socket_data.encrypted {
    env_builder = env_builder.https();
  }

  env_builder = env_builder
    .var("SCGI".to_string(), "1".to_string())
    .server(SERVER_SOFTWARE.to_string())
    .server_address(socket_data.local_addr)
    .client_address(socket_data.remote_addr)
    .hostname(
      server_name
        .map(|name| name.to_string())
        .unwrap_or_else(|| socket_data.local_addr.ip().to_canonical().to_string()),
    )
    .script_path(execute_pathbuf.clone(), wwwroot.to_path_buf(), path_info)
    .request_uri(original_request_uri)
    .system();

  for (env_var_key, env_var_value) in additional_environment_variables {
    env_builder = env_builder.var_noreplace(env_var_key, env_var_value);
  }

  let (cgi_environment, cgi_request) = env_builder.build(request);

  execute_scgi(cgi_request, error_logger, scgi_to, cgi_environment).await
}

async fn execute_scgi(
  cgi_request: CgiRequest<BoxBody<Bytes, std::io::Error>>,
  error_logger: &ErrorLogger,
  scgi_to: &str,
  cgi_environment: CgiEnvironment,
) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
  let scgi_to_fixed = if let Some(stripped) = scgi_to.strip_prefix("unix:///") {
    // hyper::Uri fails to parse a string if there is an empty authority, so add an "ignore" authority to Unix socket URLs
    &format!("unix://ignore/{stripped}")
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
      let path = scgi_to_url.path();
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
    _ => Err(anyhow::anyhow!("Only TCP and Unix socket URLs are supported."))?,
  };

  // Create environment variable netstring
  let mut environment_variables_to_wrap = Vec::new();
  for (key, value) in cgi_environment.iter() {
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
  environment_variables_netstring.extend_from_slice(environment_variables_to_wrap_length.to_string().as_bytes());
  environment_variables_netstring.push(b':');
  environment_variables_netstring.append(&mut environment_variables_to_wrap);
  environment_variables_netstring.push(b',');

  // Write environment variable netstring
  socket_writer.write_all(&environment_variables_netstring).await?;

  let cgi_stdin_reader = StreamReader::new(cgi_request);

  // Emulated standard input and standard output
  // SCGI doesn't support standard error
  let stdin = socket_writer;
  let stdout = socket_reader;

  ferron_common::runtime::spawn(async move {
    let (mut cgi_stdin_reader, mut stdin) = (cgi_stdin_reader, stdin);
    let _ = tokio::io::copy(&mut cgi_stdin_reader, &mut stdin).await;
  });

  #[cfg(feature = "runtime-monoio")]
  let stdout = SendAsyncIo::new(stdout);
  let response = convert_to_http_response(stdout).await?;
  let (parts, body) = response.into_parts();
  let response = Response::from_parts(parts, body.boxed());

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
