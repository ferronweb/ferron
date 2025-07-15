// This module would provide higher WSGI application performance,
// if it used a process pool dedicated for WSGI applications (aka pre-fork model)
// instead of spawning blocking threads in a Monoio runtime,
// because of Python's GIL, which causes the WSGI application in current setup
// to effectively run as single-threaded single-process.
// Pre-forking a process pool isn't supported on Windows.

use std::collections::HashMap;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::{StreamExt, TryStreamExt};
use hashlink::LinkedHashMap;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty, StreamBody};
use hyper::body::Frame;
use hyper::header::{self, HeaderName, HeaderValue};
use hyper::{HeaderMap, Request, Response, StatusCode};
use pyo3::exceptions::{PyAssertionError, PyException};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBool, PyCFunction, PyDict, PyIterator, PyString, PyTuple};
use tokio::io::AsyncReadExt;
use tokio::sync::Mutex;
use tokio_util::io::StreamReader;

use crate::config::ServerConfiguration;
use crate::logging::ErrorLogger;
use crate::modules::{Module, ModuleHandlers, ModuleLoader, RequestData, ResponseData, SocketData};
use crate::util::wsgi::{load_wsgi_application, WsgiErrorStream, WsgiInputStream};
use crate::util::{get_entries, get_entries_for_validation, get_entry, get_value, ModuleCache, SERVER_SOFTWARE};

/// A WSGI module loader
pub struct WsgiModuleLoader {
  cache: ModuleCache<WsgiModule>,
}

impl WsgiModuleLoader {
  /// Creates a new module loader
  pub fn new() -> Self {
    Self {
      cache: ModuleCache::new(vec!["wsgi"]),
    }
  }
}

impl ModuleLoader for WsgiModuleLoader {
  fn load_module(
    &mut self,
    config: &ServerConfiguration,
    global_config: Option<&ServerConfiguration>,
  ) -> Result<Arc<dyn Module + Send + Sync>, Box<dyn Error + Send + Sync>> {
    Ok(
      self
        .cache
        .get_or_init::<_, Box<dyn std::error::Error + Send + Sync>>(config, move |config| {
          let clear_sys_path = global_config
            .and_then(|c| get_value!("wsgi_clear_imports", c))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
          if let Some(wsgi_application_path) = get_value!("wsgi", config).and_then(|v| v.as_str()) {
            Ok(Arc::new(WsgiModule {
              wsgi_application: Some(Arc::new(
                load_wsgi_application(PathBuf::from_str(wsgi_application_path)?.as_path(), clear_sys_path)
                  .map_err(|e| anyhow::anyhow!("Cannot load a WSGI application: {}", e))?,
              )),
            }))
          } else {
            Ok(Arc::new(WsgiModule { wsgi_application: None }))
          }
        })?,
    )
  }

  fn get_requirements(&self) -> Vec<&'static str> {
    vec!["wsgi"]
  }

  fn validate_configuration(
    &self,
    config: &ServerConfiguration,
    used_properties: &mut std::collections::HashSet<String>,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    if let Some(entries) = get_entries_for_validation!("wsgi", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `wsgi` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_string() && !entry.values[0].is_null() {
          Err(anyhow::anyhow!("The WSGI application path must be a string"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("wsgi_environment", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 2 {
          Err(anyhow::anyhow!(
            "The `wsgi_environment` configuration property must have exactly two values"
          ))?
        } else if !entry.values[0].is_string() {
          Err(anyhow::anyhow!("The WSGI environment variable name must be a string"))?
        } else if !entry.values[1].is_string() {
          Err(anyhow::anyhow!("The WSGI environment variable value must be a string"))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("wsgi_clear_imports", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `wsgi_clear_imports` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_string() && !entry.values[0].is_null() {
          Err(anyhow::anyhow!("Invalid WSGI Python import clearing option"))?
        }
      }
    };

    Ok(())
  }
}

/// A WSGI module
struct WsgiModule {
  wsgi_application: Option<Arc<Py<PyAny>>>,
}

impl Module for WsgiModule {
  fn get_module_handlers(&self) -> Box<dyn ModuleHandlers> {
    Box::new(WsgiModuleHandlers {
      wsgi_application: self.wsgi_application.clone(),
    })
  }
}

/// Handlers for the WSGI module
struct WsgiModuleHandlers {
  wsgi_application: Option<Arc<Py<PyAny>>>,
}

#[async_trait(?Send)]
impl ModuleHandlers for WsgiModuleHandlers {
  async fn request_handler(
    &mut self,
    request: Request<BoxBody<Bytes, std::io::Error>>,
    config: &ServerConfiguration,
    socket_data: &SocketData,
    error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    if let Some(wsgi_application) = self.wsgi_application.clone() {
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
            monoio::runtime::spawn_blocking(move || std::fs::canonicalize(wwwroot_unknown))
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
      if let Some(additional_environment_variables_config) = get_entries!("wsgi_environment", config) {
        for additional_variable in additional_environment_variables_config.inner.iter() {
          if let Some(key) = additional_variable.values.first().and_then(|v| v.as_str()) {
            if let Some(value) = additional_variable.values.get(1).and_then(|v| v.as_str()) {
              additional_environment_variables.insert(key.to_string(), value.to_string());
            }
          }
        }
      }

      return execute_wsgi_with_environment_variables(
        request,
        socket_data,
        error_logger,
        wwwroot,
        execute_pathbuf,
        execute_path_info,
        get_value!("server_administrator_email", config).and_then(|v| v.as_str()),
        wsgi_application,
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

struct ResponseHead {
  status: StatusCode,
  headers: Option<HeaderMap>,
  is_set: bool,
  is_sent: bool,
}

impl ResponseHead {
  fn new() -> Self {
    Self {
      status: StatusCode::OK,
      headers: None,
      is_set: false,
      is_sent: false,
    }
  }
}

#[allow(clippy::too_many_arguments)]
async fn execute_wsgi_with_environment_variables(
  request: Request<BoxBody<Bytes, std::io::Error>>,
  socket_data: &SocketData,
  error_logger: &ErrorLogger,
  wwwroot: &Path,
  execute_pathbuf: PathBuf,
  path_info: Option<String>,
  server_administrator_email: Option<&str>,
  wsgi_application: Arc<Py<PyAny>>,
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
    if let hashlink::linked_hash_map::Entry::Vacant(entry) = environment_variables.entry(env_var_key) {
      entry.insert(env_var_value);
    }
  }

  execute_wsgi(request, error_logger, wsgi_application, environment_variables).await
}

async fn execute_wsgi(
  request: Request<BoxBody<Bytes, std::io::Error>>,
  error_logger: &ErrorLogger,
  wsgi_application: Arc<Py<PyAny>>,
  environment_variables: LinkedHashMap<String, String>,
) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
  let (_, body) = request.into_parts();
  let body_reader = StreamReader::new(body.into_data_stream().map_err(std::io::Error::other));
  let wsgi_head = Arc::new(Mutex::new(ResponseHead::new()));
  let wsgi_head_clone = wsgi_head.clone();
  let error_logger_owned = error_logger.to_owned();
  let body_iterator = crate::runtime::spawn_blocking(move || {
    Python::with_gil(move |py| -> PyResult<Py<PyIterator>> {
      let start_response = PyCFunction::new_closure(
        py,
        None,
        None,
        move |args: &Bound<'_, PyTuple>, kwargs: Option<&Bound<'_, PyDict>>| -> PyResult<_> {
          let args_native = args.extract::<(String, Vec<(String, String)>)>()?;
          let exc_info = kwargs.map_or(Ok(None), |kwargs| {
            let exc_info = kwargs.get_item("exc_info");
            if let Ok(Some(exc_info)) = exc_info {
              if exc_info.is_none() {
                Ok(None)
              } else {
                Ok(Some(exc_info))
              }
            } else {
              exc_info
            }
          })?;
          let mut wsgi_head_locked = wsgi_head_clone.blocking_lock();
          if let Some(exc_info) = exc_info {
            if wsgi_head_locked.is_sent {
              let exc_info_tuple = exc_info.downcast::<PyTuple>()?;
              let exc_info_exception = exc_info_tuple
                .get_item(1)?
                .getattr("with_traceback")?
                .call((exc_info_tuple.get_item(2)?,), None)?
                .downcast::<PyException>()?
                .clone();
              Err(exc_info_exception)?
            }
          } else if wsgi_head_locked.is_set {
            Err(PyAssertionError::new_err("Headers already set"))?
          }
          let status_code_string_option = args_native.0.split(" ").next();
          if let Some(status_code_string) = status_code_string_option {
            wsgi_head_locked.status =
              StatusCode::from_u16(status_code_string.parse()?).map_err(|e| anyhow::anyhow!(e))?;
          } else {
            Err(anyhow::anyhow!("Can't extract status code"))?;
          }
          let mut header_map = HeaderMap::new();
          for header in args_native.1 {
            header_map.append(
              HeaderName::from_str(&header.0).map_err(|e| anyhow::anyhow!(e))?,
              HeaderValue::from_str(&header.1).map_err(|e| anyhow::anyhow!(e))?,
            );
          }
          wsgi_head_locked.headers = Some(header_map);
          wsgi_head_locked.is_set = true;
          Ok(())
        },
      )?;
      let mut environment: HashMap<String, Bound<'_, PyAny>> = HashMap::new();
      let is_https = environment_variables.contains_key("HTTPS");
      let content_length = if let Some(content_length) = environment_variables.get("CONTENT_LENGTH") {
        content_length.parse::<u64>().ok()
      } else {
        None
      };
      for (environment_variable, environment_variable_value) in environment_variables {
        environment.insert(
          environment_variable,
          PyString::new(py, &environment_variable_value).into_any(),
        );
      }
      environment.insert("wsgi.version".to_string(), PyTuple::new(py, [1, 0])?.into_any());
      environment.insert(
        "wsgi.url_scheme".to_string(),
        PyString::new(py, if is_https { "https" } else { "http" }).into_any(),
      );
      environment.insert(
        "wsgi.input".to_string(),
        (if let Some(content_length) = content_length {
          WsgiInputStream::new(body_reader.take(content_length))
        } else {
          WsgiInputStream::new(body_reader)
        })
        .into_pyobject(py)?
        .into_any(),
      );
      environment.insert(
        "wsgi.errors".to_string(),
        WsgiErrorStream::new(error_logger_owned).into_pyobject(py)?.into_any(),
      );
      environment.insert("wsgi.multithread".to_string(), PyBool::new(py, true).as_any().clone());
      environment.insert("wsgi.multiprocess".to_string(), PyBool::new(py, false).as_any().clone());
      environment.insert("wsgi.run_once".to_string(), PyBool::new(py, false).as_any().clone());
      let body_unknown = wsgi_application.call(py, (environment, start_response), None)?;
      let body_iterator = body_unknown.downcast_bound::<PyIterator>(py)?.clone().unbind();
      Ok(body_iterator)
    })
  })
  .await
  .unwrap_or(Err(
    std::io::Error::other("Can't spawn a blocking task for WSGI").into(),
  ))?;

  let wsgi_head_clone = wsgi_head.clone();
  let mut response_stream = futures_util::stream::unfold(Arc::new(body_iterator), move |body_iterator_arc| {
    let wsgi_head_clone = wsgi_head_clone.clone();
    Box::pin(async move {
      let body_iterator_arc_clone = body_iterator_arc.clone();
      let blocking_thread_result = crate::runtime::spawn_blocking(move || {
        Python::with_gil(|py| -> PyResult<Option<Bytes>> {
          let mut body_iterator_bound = body_iterator_arc_clone.bind(py).clone();
          if let Some(body_chunk) = body_iterator_bound.next() {
            Ok(Some(Bytes::from(body_chunk?.extract::<Vec<u8>>()?)))
          } else {
            Ok(None)
          }
        })
      })
      .await;

      match blocking_thread_result {
        Err(_) => Some((
          Err(std::io::Error::other("Can't spawn a blocking task for WSGI")),
          body_iterator_arc,
        )),
        Ok(Err(error)) => Some((Err(std::io::Error::other(error)), body_iterator_arc)),
        Ok(Ok(None)) => None,
        Ok(Ok(Some(chunk))) => {
          let wsgi_head_locked = wsgi_head_clone.lock().await;
          if !wsgi_head_locked.is_set {
            Some((
              Err(std::io::Error::other(
                "The \"start_response\" function hasn't been called.",
              )),
              body_iterator_arc,
            ))
          } else {
            Some((Ok(chunk), body_iterator_arc))
          }
        }
      }
    })
  });

  let first_chunk = response_stream.next().await;
  let response_body = if let Some(Err(first_chunk_error)) = first_chunk {
    Err(first_chunk_error)?
  } else if let Some(Ok(first_chunk)) = first_chunk {
    let response_stream_first_item = futures_util::stream::once(async move { Ok(first_chunk) });
    let response_stream_combined = response_stream_first_item.chain(response_stream);
    let stream_body = StreamBody::new(response_stream_combined.map_ok(Frame::data));

    BodyExt::boxed(stream_body)
  } else {
    BodyExt::boxed(Empty::new().map_err(|e| match e {}))
  };

  let mut wsgi_head_locked = wsgi_head.lock().await;
  let mut response = Response::new(response_body);
  *response.status_mut() = wsgi_head_locked.status;
  if let Some(headers) = wsgi_head_locked.headers.take() {
    *response.headers_mut() = headers;
  }
  wsgi_head_locked.is_sent = true;

  Ok(ResponseData {
    request: None,
    response: Some(response),
    response_status: None,
    response_headers: None,
    new_remote_address: None,
  })
}
