#[cfg(not(unix))]
compile_error!("This module is supported only on Unix and Unix-like systems.");

use std::collections::HashMap;
use std::error::Error;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::thread;

use crate::ferron_common::{
  ErrorLogger, HyperRequest, HyperUpgraded, RequestData, ResponseData, ServerConfig, ServerModule,
  ServerModuleHandlers, SocketData,
};
use crate::ferron_common::{HyperResponse, WithRuntime};
use crate::ferron_res::server_software::SERVER_SOFTWARE;
use crate::ferron_util::obtain_config_struct::ObtainConfigStruct;
use crate::ferron_util::preforked_process_pool::{
  read_ipc_message, read_ipc_message_async, write_ipc_message, write_ipc_message_async,
  PreforkedProcessPool,
};
use crate::ferron_util::wsgi_load_application::load_wsgi_application;
use crate::ferron_util::wsgid_body_reader::WsgidBodyReader;
use crate::ferron_util::wsgid_error_stream::WsgidErrorStream;
use crate::ferron_util::wsgid_input_stream::WsgidInputStream;
use crate::ferron_util::wsgid_message_structs::{
  ProcessPoolToServerMessage, ServerToProcessPoolMessage,
};
use async_trait::async_trait;
use futures_util::{StreamExt, TryStreamExt};
use hashlink::LinkedHashMap;
use http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use http_body_util::{BodyExt, Empty, StreamBody};
use hyper::body::{Bytes, Frame};
use hyper::header;
use hyper::Response;
use hyper_tungstenite::HyperWebsocket;
use interprocess::unnamed_pipe::{Recver, Sender};
use pyo3::exceptions::{PyAssertionError, PyException};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyCFunction, PyDict, PyIterator, PyString, PyTuple};
//use postcard::{DeOptions, SerOptions};
use tokio::fs;
use tokio::runtime::Handle;
use tokio::sync::Mutex;

struct WsgidApplicationData {
  wsgi_process_pool: Option<Arc<PreforkedProcessPool>>,
  wsgi_path: Option<String>,
}

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

    let received_message =
      match postcard::from_bytes::<ServerToProcessPoolMessage>(&received_raw_message) {
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
        let body_iterator = Python::attach(move |py| -> PyResult<Py<PyIterator>> {
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
                  let exc_info_tuple = exc_info.cast::<PyTuple>()?;
                  let exc_info_exception = exc_info_tuple
                    .get_item(1)?
                    .getattr("with_traceback")?
                    .call((exc_info_tuple.get_item(2)?,), None)?
                    .cast::<PyException>()?
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
          let content_length =
            if let Some(content_length) = environment_variables.get("CONTENT_LENGTH") {
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
          environment.insert(
            "wsgi.version".to_string(),
            PyTuple::new(py, [1, 0])?.into_any(),
          );
          environment.insert(
            "wsgi.url_scheme".to_string(),
            PyString::new(py, if is_https { "https" } else { "http" }).into_any(),
          );
          environment.insert(
            "wsgi.input".to_string(),
            (if let Some(content_length) = content_length {
              WsgidInputStream::new(
                BufReader::new(WsgidBodyReader::new(
                  tx_mutex_clone.clone(),
                  rx_mutex_clone.clone(),
                ))
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
          environment.insert(
            "wsgi.multithread".to_string(),
            PyBool::new(py, false).as_any().clone(),
          );
          environment.insert(
            "wsgi.multiprocess".to_string(),
            PyBool::new(py, true).as_any().clone(),
          );
          environment.insert(
            "wsgi.run_once".to_string(),
            PyBool::new(py, false).as_any().clone(),
          );
          let body_unknown = wsgi_application.call(py, (environment, start_response), None)?;
          let body_iterator = body_unknown.cast_bound::<PyIterator>(py)?.clone().unbind();
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
            let body_chunk_result = Python::attach(|py| -> PyResult<Option<Vec<u8>>> {
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

fn init_wsgi_process_pool(
  wsgi_script_path: PathBuf,
) -> Result<PreforkedProcessPool, Box<dyn Error + Send + Sync>> {
  let available_parallelism = thread::available_parallelism()?.get();
  // Safety: The function depends on `nix::unistd::fork`, which is executed before any threads are spawned.
  // The forking function is safe to call for single-threaded applications.
  unsafe {
    PreforkedProcessPool::new(available_parallelism, move |tx, rx| {
      let wsgi_script_path_clone = wsgi_script_path.clone();
      wsgi_pool_fn(tx, rx, wsgi_script_path_clone)
    })
  }
}

pub fn server_module_init(
  config: &ServerConfig,
) -> Result<Box<dyn ServerModule + Send + Sync>, Box<dyn Error + Send + Sync>> {
  Ok(Box::new(WsgidModule::new(ObtainConfigStruct::new(
    config,
    |config| {
      let wsgi_process_pool =
        if let Some(wsgi_application_path) = config["global"]["wsgidApplicationPath"].as_str() {
          Some(Arc::new(init_wsgi_process_pool(PathBuf::from_str(
            wsgi_application_path,
          )?)?))
        } else {
          None
        };
      let wsgi_path = config["global"]["wsgidPath"]
        .as_str()
        .map(|s| s.to_string());
      Ok(Some(Arc::new(WsgidApplicationData {
        wsgi_process_pool,
        wsgi_path,
      })))
    },
  )?)))
}

struct WsgidModule {
  wsgi_process_pools: ObtainConfigStruct<Arc<WsgidApplicationData>>,
}

impl WsgidModule {
  fn new(wsgi_process_pools: ObtainConfigStruct<Arc<WsgidApplicationData>>) -> Self {
    Self { wsgi_process_pools }
  }
}

impl ServerModule for WsgidModule {
  fn get_handlers(&self, handle: Handle) -> Box<dyn ServerModuleHandlers + Send> {
    Box::new(WsgidModuleHandlers {
      handle,
      wsgi_process_pools: self.wsgi_process_pools.clone(),
    })
  }
}

struct WsgidModuleHandlers {
  handle: Handle,
  wsgi_process_pools: ObtainConfigStruct<Arc<WsgidApplicationData>>,
}

#[async_trait]
impl ServerModuleHandlers for WsgidModuleHandlers {
  async fn request_handler(
    &mut self,
    request: RequestData,
    config: &ServerConfig,
    socket_data: &SocketData,
    error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    WithRuntime::new(self.handle.clone(), async move {
      let hyper_request = request.get_hyper_request();
      let wsgi_data = self.wsgi_process_pools.obtain(
        match hyper_request.headers().get(header::HOST) {
          Some(value) => value.to_str().ok(),
          None => None,
        },
        socket_data.remote_addr.ip(),
        request
          .get_original_url()
          .unwrap_or(request.get_hyper_request().uri())
          .path(),
        request.get_error_status_code().map(|x| x.as_u16()),
      );

      let wsgi_process_pool = wsgi_data.clone().and_then(|x| x.wsgi_process_pool.clone());
      let wsgi_path = wsgi_data.clone().and_then(|x| x.wsgi_path.clone());

      let request_path = hyper_request.uri().path();
      let mut request_path_bytes = request_path.bytes();
      if request_path_bytes.len() < 1 || request_path_bytes.nth(0) != Some(b'/') {
        return Ok(
          ResponseData::builder(request)
            .status(StatusCode::BAD_REQUEST)
            .build(),
        );
      }

      if let Some(wsgi_process_pool) = wsgi_process_pool {
        let wsgi_path = wsgi_path.unwrap_or("/".to_string());
        let mut canonical_wsgi_path: &str = &wsgi_path;
        if canonical_wsgi_path.bytes().last() == Some(b'/') {
          canonical_wsgi_path = &canonical_wsgi_path[..(canonical_wsgi_path.len() - 1)];
        }

        let request_path_with_slashes = match request_path == canonical_wsgi_path {
          true => format!("{}/", request_path),
          false => request_path.to_string(),
        };
        if let Some(stripped_request_path) =
          request_path_with_slashes.strip_prefix(canonical_wsgi_path)
        {
          let wwwroot_yaml = &config["wwwroot"];
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

          return execute_wsgi_with_environment_variables(
            request,
            socket_data,
            error_logger,
            wwwroot,
            execute_pathbuf,
            execute_path_info,
            config["serverAdministratorEmail"].as_str(),
            wsgi_process_pool,
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
    _config: &ServerConfig,
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
    _config: &ServerConfig,
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
    _headers: &hyper::HeaderMap,
    _config: &ServerConfig,
    _socket_data: &SocketData,
    _error_logger: &ErrorLogger,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    Ok(())
  }

  fn does_websocket_requests(&mut self, _config: &ServerConfig, _socket_data: &SocketData) -> bool {
    false
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
  request: RequestData,
  socket_data: &SocketData,
  error_logger: &ErrorLogger,
  wwwroot: &Path,
  execute_pathbuf: PathBuf,
  path_info: Option<String>,
  server_administrator_email: Option<&str>,
  wsgi_process_pool: Arc<PreforkedProcessPool>,
) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
  let mut environment_variables: LinkedHashMap<String, String> = LinkedHashMap::new();

  let hyper_request = request.get_hyper_request();
  let original_request_uri = request.get_original_url().unwrap_or(hyper_request.uri());

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
    environment_variables.insert("HTTPS".to_string(), "on".to_string());
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

  let (hyper_request, _, _, _) = request.into_parts();

  execute_wsgi(
    hyper_request,
    error_logger,
    wsgi_process_pool,
    environment_variables,
  )
  .await
}

async fn execute_wsgi(
  hyper_request: HyperRequest,
  error_logger: &ErrorLogger,
  wsgi_process_pool: Arc<PreforkedProcessPool>,
  environment_variables: LinkedHashMap<String, String>,
) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
  let ipc_mutex = wsgi_process_pool
    .obtain_process_with_init_async_ipc()
    .await?;
  let (_, body) = hyper_request.into_parts();
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
      let received_message =
        postcard::from_bytes::<ProcessPoolToServerMessage>(&read_ipc_message_async(rx).await?)?;

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
  let mut response_stream = futures_util::stream::unfold(ipc_mutex, move |ipc_mutex| {
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
                    header_map.append(
                      HeaderName::from_str(&key)?,
                      HeaderValue::from_bytes(value.as_bytes())?,
                    );
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
  let mut hyper_response = Response::new(response_body);
  *hyper_response.status_mut() = wsgi_head_locked.status;
  if let Some(headers) = wsgi_head_locked.headers.take() {
    *hyper_response.headers_mut() = headers;
  }

  Ok(
    ResponseData::builder_without_request()
      .response(hyper_response)
      .build(),
  )
}
