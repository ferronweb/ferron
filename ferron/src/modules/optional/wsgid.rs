#[cfg(not(unix))]
compile_error!("This module is supported only on Unix and Unix-like systems.");

use std::collections::HashMap;
use std::error::Error;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::thread;

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::{StreamExt, TryStreamExt};
use hashlink::LinkedHashMap;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty, StreamBody};
use hyper::body::Frame;
use hyper::header::{self, HeaderName, HeaderValue};
use hyper::{HeaderMap, Request, Response, StatusCode};
use interprocess::unnamed_pipe::{Recver, Sender};
use pyo3::exceptions::{PyAssertionError, PyException};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyCFunction, PyDict, PyIterator, PyString, PyTuple};
use tokio::sync::Mutex;

use crate::config::ServerConfiguration;
use crate::logging::ErrorLogger;
use crate::modules::{Module, ModuleHandlers, ModuleLoader, RequestData, ResponseData, SocketData};
use crate::util::wsgi::load_wsgi_application;
use crate::util::wsgid::{
  ProcessPoolToServerMessage, ServerToProcessPoolMessage, WsgidBodyReader, WsgidErrorStream, WsgidInputStream,
};
use crate::util::{
  get_entries, get_entries_for_validation, get_entry, get_value, read_ipc_message, read_ipc_message_async,
  write_ipc_message, write_ipc_message_async, ModuleCache, PreforkedProcessPool, SERVER_SOFTWARE,
};

struct ResponseHead {
  status: u16,
  headers: Option<LinkedHashMap<String, Vec<String>>>,
  is_set: bool,
  is_sent: bool,
}

impl ResponseHead {
  fn new() -> Self {
    Self {
      status: 200,
      headers: None,
      is_set: false,
      is_sent: false,
    }
  }
}

/// The WSGI process pool function
fn wsgi_pool_fn(tx: Sender, rx: Recver, wsgi_script_path: PathBuf) {
  let wsgi_application_result: Result<Py<PyAny>, Box<dyn Error + Send + Sync>> =
    load_wsgi_application(wsgi_script_path.as_path(), false);
  let mut body_iterators = HashMap::new();
  let mut application_id = 0;
  let mut wsgi_head = Arc::new(Mutex::new(ResponseHead::new()));
  let rx_mutex = Arc::new(Mutex::new(rx));
  let tx_mutex = Arc::new(Mutex::new(tx));

  loop {
    let received_raw_message = match read_ipc_message(&mut rx_mutex.blocking_lock()) {
      Ok(message) => message,
      Err(_) => break,
    };

    let received_message = match postcard::from_bytes::<ServerToProcessPoolMessage>(&received_raw_message) {
      Ok(message) => message,
      Err(_) => continue,
    };

    if let Some(error) = (|| -> Result<(), Box<dyn Error + Send + Sync>> {
      let wsgi_application = wsgi_application_result
        .as_ref()
        .map_err(|x| anyhow::anyhow!(x.to_string()))?;
      if let Some(environment_variables) = received_message.environment_variables {
        wsgi_head = Arc::new(Mutex::new(ResponseHead::new()));
        let wsgi_head_clone = wsgi_head.clone();
        let tx_mutex_clone = tx_mutex.clone();
        let rx_mutex_clone = rx_mutex.clone();
        let body_iterator = Python::with_gil(move |py| -> PyResult<Py<PyIterator>> {
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
                wsgi_head_locked.status = status_code_string
                  .parse()
                  .map_err(|e: std::num::ParseIntError| anyhow::anyhow!(e))?;
              } else {
                Err(anyhow::anyhow!("Can't extract status code"))?;
              }
              let mut header_map: LinkedHashMap<String, Vec<String>> = LinkedHashMap::new();
              for header in args_native.1 {
                let header_name = header.0.to_lowercase();
                let header_value = header.1;
                if let Some(header_values) = header_map.get_mut(&header_name) {
                  header_values.push(header_value);
                } else {
                  header_map.insert(header_name, vec![header_value]);
                }
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
              WsgidInputStream::new(
                BufReader::new(WsgidBodyReader::new(tx_mutex_clone.clone(), rx_mutex_clone.clone()))
                  .take(content_length),
              )
            } else {
              WsgidInputStream::new(BufReader::new(WsgidBodyReader::new(
                tx_mutex_clone.clone(),
                rx_mutex_clone.clone(),
              )))
            })
            .into_pyobject(py)?
            .into_any(),
          );
          environment.insert(
            "wsgi.errors".to_string(),
            WsgidErrorStream::new(tx_mutex_clone.clone())
              .into_pyobject(py)?
              .into_any(),
          );
          environment.insert("wsgi.multithread".to_string(), PyBool::new(py, false).as_any().clone());
          environment.insert("wsgi.multiprocess".to_string(), PyBool::new(py, true).as_any().clone());
          environment.insert("wsgi.run_once".to_string(), PyBool::new(py, false).as_any().clone());
          let body_unknown = wsgi_application.call(py, (environment, start_response), None)?;
          let body_iterator = body_unknown.downcast_bound::<PyIterator>(py)?.clone().unbind();
          Ok(body_iterator)
        })?;
        let current_application_id = application_id;
        body_iterators.insert(current_application_id, Arc::new(body_iterator));
        application_id += 1;
        write_ipc_message(
          &mut tx_mutex.blocking_lock(),
          &postcard::to_allocvec::<ProcessPoolToServerMessage>(&ProcessPoolToServerMessage {
            application_id: Some(current_application_id),
            status_code: None,
            headers: None,
            body_chunk: None,
            error_log_line: None,
            error_message: None,
            requests_body_chunk: false,
          })?,
        )?
      } else if received_message.requests_body_chunk {
        if let Some(application_id) = received_message.application_id {
          if let Some(body_iterator_arc) = body_iterators.get(&application_id) {
            let wsgi_head_clone = wsgi_head.clone();
            let body_iterator_arc_clone = body_iterator_arc.clone();
            let body_chunk_result = Python::with_gil(|py| -> PyResult<Option<Vec<u8>>> {
              let mut body_iterator_bound = body_iterator_arc_clone.bind(py).clone();
              if let Some(body_chunk) = body_iterator_bound.next() {
                Ok(Some(body_chunk?.extract::<Vec<u8>>()?))
              } else {
                Ok(None)
              }
            });

            let body_chunk = (match body_chunk_result {
              Err(error) => Err(std::io::Error::other(error)),
              Ok(None) => Ok(None),
              Ok(Some(chunk)) => {
                let wsgi_head_locked = wsgi_head_clone.blocking_lock();
                if !wsgi_head_locked.is_set {
                  Err(std::io::Error::other(
                    "The \"start_response\" function hasn't been called.",
                  ))
                } else {
                  Ok(Some(chunk))
                }
              }
            })?;

            let status_code;
            let headers;

            let mut wsgi_head_locked = wsgi_head_clone.blocking_lock();
            if wsgi_head_locked.is_sent {
              status_code = None;
              headers = None;
            } else {
              status_code = Some(wsgi_head_locked.status);
              headers = wsgi_head_locked.headers.take();
              wsgi_head_locked.is_sent = true;
            }
            drop(wsgi_head_locked);

            if body_chunk.is_none() {
              body_iterators.remove(&application_id);
            }

            write_ipc_message(
              &mut tx_mutex.blocking_lock(),
              &postcard::to_allocvec::<ProcessPoolToServerMessage>(&ProcessPoolToServerMessage {
                application_id: None,
                status_code,
                headers,
                body_chunk,
                error_log_line: None,
                error_message: None,
                requests_body_chunk: false,
              })?,
            )?
          } else {
            Err(anyhow::anyhow!("The WSGI request wasn't initialized"))?
          }
        } else {
          Err(anyhow::anyhow!("The WSGI request wasn't initialized"))?
        }
      }

      Ok(())
    })()
    .err()
    {
      if write_ipc_message(
        &mut tx_mutex.blocking_lock(),
        &postcard::to_allocvec::<ProcessPoolToServerMessage>(&ProcessPoolToServerMessage {
          application_id: None,
          status_code: None,
          headers: None,
          body_chunk: None,
          error_log_line: None,
          error_message: Some(error.to_string()),
          requests_body_chunk: false,
        })
        .unwrap_or_default(),
      )
      .is_err()
      {
        break;
      }
    }
  }
}

/// Initializes a WSGI process pool
fn init_wsgi_process_pool(wsgi_script_path: PathBuf) -> Result<PreforkedProcessPool, Box<dyn Error + Send + Sync>> {
  let available_parallelism = thread::available_parallelism()?.get();
  // Safety: The execution of child processes is passed into the WSGI pool function. (?)
  unsafe {
    PreforkedProcessPool::new(available_parallelism, move |tx, rx| {
      let wsgi_script_path_clone = wsgi_script_path.clone();
      wsgi_pool_fn(tx, rx, wsgi_script_path_clone)
    })
  }
}

/// A WSGI (with pre-forked process pool) module loader
pub struct WsgidModuleLoader {
  cache: ModuleCache<WsgidModule>,
}

impl WsgidModuleLoader {
  /// Creates a new module loader
  pub fn new() -> Self {
    Self {
      cache: ModuleCache::new(vec![]),
    }
  }
}

impl ModuleLoader for WsgidModuleLoader {
  fn load_module(
    &mut self,
    config: &ServerConfiguration,
    _global_config: Option<&ServerConfiguration>,
    _secondary_runtime: &tokio::runtime::Runtime,
  ) -> Result<Arc<dyn Module + Send + Sync>, Box<dyn Error + Send + Sync>> {
    Ok(
      self
        .cache
        .get_or_init::<_, Box<dyn std::error::Error + Send + Sync>>(config, move |config| {
          if let Some(wsgi_application_path) = get_value!("wsgid", config).and_then(|v| v.as_str()) {
            Ok(Arc::new(WsgidModule {
              wsgi_process_pool: Some(Arc::new(
                init_wsgi_process_pool(PathBuf::from_str(wsgi_application_path)?)
                  .map_err(|e| anyhow::anyhow!("Cannot initialize a WSGI process pool: {}", e))?,
              )),
            }))
          } else {
            Ok(Arc::new(WsgidModule {
              wsgi_process_pool: None,
            }))
          }
        })?,
    )
  }

  fn get_requirements(&self) -> Vec<&'static str> {
    vec!["wsgid"]
  }

  fn validate_configuration(
    &self,
    config: &ServerConfiguration,
    used_properties: &mut std::collections::HashSet<String>,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    if let Some(entries) = get_entries_for_validation!("wsgdi", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `wsgid` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_string() && !entry.values[0].is_null() {
          Err(anyhow::anyhow!(
            "The WSGI application path (with a pre-forked process pool) must be a string"
          ))?
        }
      }
    };

    if let Some(entries) = get_entries_for_validation!("wsgid_environment", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 2 {
          Err(anyhow::anyhow!(
            "The `wsgid_environment` configuration property must have exactly two values"
          ))?
        } else if !entry.values[0].is_string() {
          Err(anyhow::anyhow!(
            "The WSGI (with a pre-forked process pool) environment variable name must be a string"
          ))?
        } else if !entry.values[1].is_string() {
          Err(anyhow::anyhow!(
            "The WSGI (with a pre-forked process pool) environment variable value must be a string"
          ))?
        }
      }
    };

    Ok(())
  }
}

/// A WSGI (with pre-forked process pool) module
struct WsgidModule {
  wsgi_process_pool: Option<Arc<PreforkedProcessPool>>,
}

impl Module for WsgidModule {
  fn get_module_handlers(&self) -> Box<dyn ModuleHandlers> {
    Box::new(WsgidModuleHandlers {
      wsgi_process_pool: self.wsgi_process_pool.clone(),
    })
  }
}

/// Handlers for the WSGI (with pre-forked process pool) module
struct WsgidModuleHandlers {
  wsgi_process_pool: Option<Arc<PreforkedProcessPool>>,
}

#[async_trait(?Send)]
impl ModuleHandlers for WsgidModuleHandlers {
  async fn request_handler(
    &mut self,
    request: Request<BoxBody<Bytes, std::io::Error>>,
    config: &ServerConfiguration,
    socket_data: &SocketData,
    error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    if let Some(wsgi_process_pool) = self.wsgi_process_pool.clone() {
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
      if let Some(additional_environment_variables_config) = get_entries!("wsgid_environment", config) {
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
        wsgi_process_pool,
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

struct ResponseHeadHyper {
  status: StatusCode,
  headers: Option<HeaderMap>,
}

impl ResponseHeadHyper {
  fn new() -> Self {
    Self {
      status: StatusCode::OK,
      headers: None,
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
  wsgi_process_pool: Arc<PreforkedProcessPool>,
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

  execute_wsgi(request, error_logger, wsgi_process_pool, environment_variables).await
}

async fn execute_wsgi(
  request: Request<BoxBody<Bytes, std::io::Error>>,
  error_logger: &ErrorLogger,
  wsgi_process_pool: Arc<PreforkedProcessPool>,
  environment_variables: LinkedHashMap<String, String>,
) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
  let ipc_mutex = wsgi_process_pool.obtain_process().await?;
  let (_, body) = request.into_parts();
  let mut body_stream = body.into_data_stream().map_err(std::io::Error::other);
  let application_id = {
    let (tx, rx) = &mut *ipc_mutex.lock().await;
    write_ipc_message_async(
      tx,
      &postcard::to_allocvec(&ServerToProcessPoolMessage {
        application_id: None,
        environment_variables: Some(environment_variables),
        body_chunk: None,
        body_error_message: None,
        requests_body_chunk: false,
      })?,
    )
    .await?;

    let application_id;
    loop {
      let received_message = postcard::from_bytes::<ProcessPoolToServerMessage>(&read_ipc_message_async(rx).await?)?;

      if let Some(error_message) = received_message.error_message {
        Err(anyhow::anyhow!(error_message))?
      }

      if let Some(application_id_obtained) = received_message.application_id {
        application_id = application_id_obtained;
        break;
      }

      if let Some(error_log_line) = received_message.error_log_line {
        error_logger.log(&error_log_line).await;
      } else if received_message.requests_body_chunk {
        let body_chunk;
        let body_error_message;
        match body_stream.next().await {
          None => {
            body_chunk = None;
            body_error_message = None;
          }
          Some(Err(err)) => {
            body_chunk = None;
            body_error_message = Some(err.to_string());
          }
          Some(Ok(chunk)) => {
            body_chunk = Some(chunk.to_vec());
            body_error_message = None;
          }
        };
        write_ipc_message_async(
          tx,
          &postcard::to_allocvec(&ServerToProcessPoolMessage {
            application_id: None,
            environment_variables: None,
            body_chunk,
            body_error_message,
            requests_body_chunk: false,
          })?,
        )
        .await?;
      }
    }

    application_id
  };

  let wsgi_head = Arc::new(Mutex::new(ResponseHeadHyper::new()));
  let wsgi_head_clone = wsgi_head.clone();
  let error_logger_arc = Arc::new(error_logger.clone());
  let body_stream_mutex = Arc::new(Mutex::new(body_stream));
  let mut response_stream = futures_util::stream::unfold(ipc_mutex, move |ipc_mutex: Arc<Mutex<_>>| {
    let wsgi_head_clone = wsgi_head_clone.clone();
    let error_logger_arc_clone = error_logger_arc.clone();
    let body_stream_mutex_clone = body_stream_mutex.clone();
    Box::pin(async move {
      let ipc_mutex_borrowed = &ipc_mutex;
      let chunk_result: Result<Option<Bytes>, Box<dyn Error + Send + Sync>> = async {
        let (tx, rx) = &mut *ipc_mutex_borrowed.lock().await;
        write_ipc_message_async(
          tx,
          &postcard::to_allocvec(&ServerToProcessPoolMessage {
            application_id: Some(application_id),
            environment_variables: None,
            body_chunk: None,
            body_error_message: None,
            requests_body_chunk: true,
          })?,
        )
        .await?;

        loop {
          let received_message =
            postcard::from_bytes::<ProcessPoolToServerMessage>(&read_ipc_message_async(rx).await?)?;

          if let Some(error_message) = received_message.error_message {
            Err(anyhow::anyhow!(error_message))?
          } else if let Some(body_chunk) = received_message.body_chunk {
            if let Some(status_code) = received_message.status_code {
              let mut wsgi_head_locked = wsgi_head_clone.lock().await;
              wsgi_head_locked.status = StatusCode::from_u16(status_code)?;
              if let Some(headers) = received_message.headers {
                let mut header_map = HeaderMap::new();
                for (key, value) in headers {
                  for value in value {
                    header_map.append(HeaderName::from_str(&key)?, HeaderValue::from_bytes(value.as_bytes())?);
                  }
                }
                wsgi_head_locked.headers = Some(header_map);
              }
            }
            return Ok(Some(Bytes::from(body_chunk)));
          } else if let Some(error_log_line) = received_message.error_log_line {
            error_logger_arc_clone.log(&error_log_line).await;
          } else if received_message.requests_body_chunk {
            let body_chunk;
            let body_error_message;
            match body_stream_mutex_clone.lock().await.next().await {
              None => {
                body_chunk = None;
                body_error_message = None;
              }
              Some(Err(err)) => {
                body_chunk = None;
                body_error_message = Some(err.to_string());
              }
              Some(Ok(chunk)) => {
                body_chunk = Some(chunk.to_vec());
                body_error_message = None;
              }
            };
            write_ipc_message_async(
              tx,
              &postcard::to_allocvec(&ServerToProcessPoolMessage {
                application_id: None,
                environment_variables: None,
                body_chunk,
                body_error_message,
                requests_body_chunk: false,
              })?,
            )
            .await?;
          } else {
            return Ok(None);
          }
        }
      }
      .await;

      match chunk_result {
        Err(error) => Some((Err(std::io::Error::other(error.to_string())), ipc_mutex)),
        Ok(None) => None,
        Ok(Some(chunk)) => Some((Ok(chunk), ipc_mutex)),
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

  Ok(ResponseData {
    request: None,
    response: Some(response),
    response_status: None,
    response_headers: None,
    new_remote_address: None,
  })
}
