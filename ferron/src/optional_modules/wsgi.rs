use std::collections::HashMap;
use std::error::Error;
use std::ffi::CString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use crate::ferron_common::{
  ErrorLogger, HyperRequest, HyperUpgraded, RequestData, ResponseData, ServerConfig, ServerModule,
  ServerModuleHandlers, SocketData,
};
use crate::ferron_common::{HyperResponse, WithRuntime};
use crate::ferron_res::server_software::SERVER_SOFTWARE;
use crate::ferron_util::ip_match::ip_match;
use crate::ferron_util::match_hostname::match_hostname;
use crate::ferron_util::match_location::match_location;
use crate::ferron_util::wsgi_structs::{WsgiApplicationLocationWrap, WsgiApplicationWrap};
use async_trait::async_trait;
use futures_util::{StreamExt, TryStreamExt};
use hashlink::LinkedHashMap;
use http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use http_body_util::{BodyExt, StreamBody};
use hyper::body::{Bytes, Frame};
use hyper::header;
use hyper::Response;
use hyper_tungstenite::HyperWebsocket;
use pyo3::prelude::*;
use pyo3::types::{PyCFunction, PyDict, PyFunction, PyIterator, PyString, PyTuple};
use tokio::fs;
use tokio::runtime::Handle;
use tokio::sync::Mutex;

fn load_wsgi_application(file_path: &Path) -> Result<Py<PyFunction>, Box<dyn Error + Send + Sync>> {
  let script_name = file_path.to_string_lossy().to_string();
  let script_name_cstring = CString::from_str(&script_name)?;
  let module_name = script_name
    .strip_suffix(".py")
    .unwrap_or(&script_name)
    .to_lowercase()
    .chars()
    .map(|c| if c.is_lowercase() { '_' } else { c })
    .collect::<String>();
  let module_name_cstring = CString::from_str(&module_name)?;
  let mut script_data = String::from(
    r#"
import sys
import os
sys.path.append(os.path.dirname(__file__))

"#,
  );
  std::fs::File::open(file_path)?.read_to_string(&mut script_data)?;
  let script_data_cstring = CString::from_str(&script_data)?;
  let wsgi_application = Python::with_gil(move |py| -> PyResult<Py<PyFunction>> {
    let wsgi_application = PyModule::from_code(
      py,
      &script_data_cstring,
      &script_name_cstring,
      &module_name_cstring,
    )?
    .getattr("application")?
    .unbind()
    .downcast_bound::<PyFunction>(py)?
    .clone()
    .unbind();
    Ok(wsgi_application)
  })?;
  Ok(wsgi_application)
}

pub fn server_module_init(
  config: &ServerConfig,
) -> Result<Box<dyn ServerModule + Send + Sync>, Box<dyn Error + Send + Sync>> {
  let mut global_wsgi_application = None;
  let mut host_wsgi_applications = Vec::new();
  if let Some(wsgi_application_path) = config["global"]["wsgiApplicationPath"].as_str() {
    global_wsgi_application = Some(Arc::new(load_wsgi_application(
      PathBuf::from_str(wsgi_application_path)?.as_path(),
    )?));
  }
  let global_wsgi_path = config["global"]["wsgiPath"].as_str().map(|s| s.to_string());

  if let Some(hosts) = config["hosts"].as_vec() {
    for host_yaml in hosts.iter() {
      let domain = host_yaml["domain"].as_str().map(String::from);
      let ip = host_yaml["ip"].as_str().map(String::from);
      let mut locations = Vec::new();
      if let Some(locations_yaml) = host_yaml["locations"].as_vec() {
        for location_yaml in locations_yaml.iter() {
          if let Some(path_str) = location_yaml["path"].as_str() {
            let path = String::from(path_str);
            if let Some(wsgi_application_path) = location_yaml["wsgiApplicationPath"].as_str() {
              locations.push(WsgiApplicationLocationWrap::new(
                path,
                Arc::new(load_wsgi_application(
                  PathBuf::from_str(wsgi_application_path)?.as_path(),
                )?),
                location_yaml["wsgiPath"].as_str().map(|s| s.to_string()),
              ));
            }
          }
        }
      }
      if let Some(wsgi_application_path) = host_yaml["wsgiApplicationPath"].as_str() {
        host_wsgi_applications.push(WsgiApplicationWrap::new(
          domain,
          ip,
          Some(Arc::new(load_wsgi_application(
            PathBuf::from_str(wsgi_application_path)?.as_path(),
          )?)),
          host_yaml["wsgiPath"].as_str().map(|s| s.to_string()),
          locations,
        ));
      } else if !locations.is_empty() {
        host_wsgi_applications.push(WsgiApplicationWrap::new(
          domain,
          ip,
          None,
          host_yaml["wsgiPath"].as_str().map(|s| s.to_string()),
          locations,
        ));
      }
    }
  }

  Ok(Box::new(WsgiModule::new(
    global_wsgi_application,
    global_wsgi_path,
    Arc::new(host_wsgi_applications),
  )))
}

struct WsgiModule {
  global_wsgi_application: Option<Arc<Py<PyFunction>>>,
  global_wsgi_path: Option<String>,
  host_wsgi_applications: Arc<Vec<WsgiApplicationWrap>>,
}

impl WsgiModule {
  fn new(
    global_wsgi_application: Option<Arc<Py<PyFunction>>>,
    global_wsgi_path: Option<String>,
    host_wsgi_applications: Arc<Vec<WsgiApplicationWrap>>,
  ) -> Self {
    Self {
      global_wsgi_application,
      global_wsgi_path,
      host_wsgi_applications,
    }
  }
}

impl ServerModule for WsgiModule {
  fn get_handlers(&self, handle: Handle) -> Box<dyn ServerModuleHandlers + Send> {
    Box::new(WsgiModuleHandlers {
      handle,
      global_wsgi_application: self.global_wsgi_application.clone(),
      global_wsgi_path: self.global_wsgi_path.clone(),
      host_wsgi_applications: self.host_wsgi_applications.clone(),
    })
  }
}

struct WsgiModuleHandlers {
  handle: Handle,
  global_wsgi_application: Option<Arc<Py<PyFunction>>>,
  global_wsgi_path: Option<String>,
  host_wsgi_applications: Arc<Vec<WsgiApplicationWrap>>,
}

#[async_trait]
impl ServerModuleHandlers for WsgiModuleHandlers {
  async fn request_handler(
    &mut self,
    request: RequestData,
    config: &ServerConfig,
    socket_data: &SocketData,
    error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    WithRuntime::new(self.handle.clone(), async move {
      let hyper_request = request.get_hyper_request();

      // Use .take() instead of .clone(), since the values in Options will only be used once.
      let mut wsgi_application = self.global_wsgi_application.take();
      let mut wsgi_path = self.global_wsgi_path.take();

      // Should have used a HashMap instead of iterating over an array for better performance...
      for host_wsgi_application_wrap in self.host_wsgi_applications.iter() {
        if match_hostname(
          match &host_wsgi_application_wrap.domain {
            Some(value) => Some(value as &str),
            None => None,
          },
          match hyper_request.headers().get(header::HOST) {
            Some(value) => value.to_str().ok(),
            None => None,
          },
        ) && match &host_wsgi_application_wrap.ip {
          Some(value) => ip_match(value as &str, socket_data.remote_addr.ip()),
          None => true,
        } {
          wsgi_application = host_wsgi_application_wrap.wsgi_application.clone();
          wsgi_path = host_wsgi_application_wrap.wsgi_path.clone();
          if let Ok(path_decoded) = urlencoding::decode(
            request
              .get_original_url()
              .unwrap_or(request.get_hyper_request().uri())
              .path(),
          ) {
            for location_wrap in host_wsgi_application_wrap.locations.iter() {
              if match_location(&location_wrap.path, &path_decoded) {
                wsgi_application = Some(location_wrap.wsgi_application.clone());
                wsgi_path = location_wrap.wsgi_path.clone();
                break;
              }
            }
          }
          break;
        }
      }

      let request_path = hyper_request.uri().path();
      let mut request_path_bytes = request_path.bytes();
      if request_path_bytes.len() < 1 || request_path_bytes.nth(0) != Some(b'/') {
        return Ok(
          ResponseData::builder(request)
            .status(StatusCode::BAD_REQUEST)
            .build(),
        );
      }

      if let Some(wsgi_application) = wsgi_application {
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
            wsgi_application,
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

struct ResponseHead {
  status: StatusCode,
  headers: Option<HeaderMap>,
  is_set: bool,
}

impl ResponseHead {
  fn new() -> Self {
    Self {
      status: StatusCode::OK,
      headers: None,
      is_set: false,
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
  wsgi_application: Arc<Py<PyFunction>>,
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
    environment_variables.insert("HTTPS".to_string(), "ON".to_string());
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

  let (hyper_request, _, _) = request.into_parts();

  execute_wsgi(
    hyper_request,
    error_logger,
    wsgi_application,
    environment_variables,
  )
  .await
}

async fn execute_wsgi(
  _hyper_request: HyperRequest,
  _error_logger: &ErrorLogger,
  wsgi_application: Arc<Py<PyFunction>>,
  environment_variables: LinkedHashMap<String, String>,
) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
  let wsgi_head = Arc::new(Mutex::new(ResponseHead::new()));
  let wsgi_head_clone = wsgi_head.clone();
  let body_iterator = tokio::task::spawn_blocking(move || {
    Python::with_gil(move |py| -> PyResult<Py<PyIterator>> {
      let start_response = PyCFunction::new_closure(
        py,
        None,
        None,
        move |args: &Bound<'_, PyTuple>, _kwargs: Option<&Bound<'_, PyDict>>| -> PyResult<_> {
          let args_native = args.extract::<(String, Vec<(String, String)>)>()?;
          let mut wsgi_head_locked = wsgi_head_clone.blocking_lock();
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
              HeaderValue::from_str(&header.0).map_err(|e| anyhow::anyhow!(e))?,
            );
          }
          wsgi_head_locked.headers = Some(header_map);
          wsgi_head_locked.is_set = true;
          Ok(())
        },
      )?;
      let mut environment = HashMap::new();
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
      // TODO: more WSGI-specific environment variables
      let body_unknown = wsgi_application.call(py, (environment, start_response), None)?;
      let body_iterator = body_unknown
        .downcast_bound::<PyIterator>(py)?
        .clone()
        .unbind();
      Ok(body_iterator)
    })
  })
  .await??;

  let wsgi_head_clone = wsgi_head.clone();
  let mut response_stream =
    futures_util::stream::unfold(Arc::new(body_iterator), move |body_iterator_arc| {
      let wsgi_head_clone = wsgi_head_clone.clone();
      Box::pin(async move {
        let body_iterator_arc_clone = body_iterator_arc.clone();
        let blocking_thread_result = tokio::task::spawn_blocking(move || {
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
          Err(error) => Some((Err(std::io::Error::other(error)), body_iterator_arc)),
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
    let response_body = BodyExt::boxed(stream_body);
    response_body
  } else {
    let stream_body = StreamBody::new(response_stream.map_ok(Frame::data));
    let response_body = BodyExt::boxed(stream_body);
    response_body
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
