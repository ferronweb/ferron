// WARNING: We have measured this module on our computers, and found it to be slower than Uvicorn (with 1 worker),
//          with FastAPI application, vanilla ASGI application is found out to be faster than Uvicorn (with 1 worker).
//          It might be more performant to just use Ferron as a reverse proxy for Uvicorn (or any other ASGI server)
// TODO: WebSocket protocol

use std::error::Error;
use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::thread;

use crate::ferron_common::{
  ErrorLogger, HyperUpgraded, RequestData, ResponseData, ServerConfig, ServerModule,
  ServerModuleHandlers, SocketData,
};
use crate::ferron_common::{HyperResponse, WithRuntime};
use crate::ferron_util::asgi_messages::{
  asgi_event_to_outgoing_struct, incoming_struct_to_asgi_event, AsgiHttpBody, AsgiHttpInitData,
  AsgiInitData, IncomingAsgiMessage, IncomingAsgiMessageInner, OutgoingAsgiMessage,
  OutgoingAsgiMessageInner,
};
use crate::ferron_util::asgi_structs::{AsgiApplicationLocationWrap, AsgiApplicationWrap};
use crate::ferron_util::ip_match::ip_match;
use crate::ferron_util::match_hostname::match_hostname;
use crate::ferron_util::match_location::match_location;
use async_channel::{Receiver, Sender};
use async_trait::async_trait;
use futures_util::StreamExt;
use http::{HeaderMap, HeaderName, HeaderValue, Response, Version};
use http_body_util::{BodyExt, StreamBody};
use hyper::body::{Bytes, Frame};
use hyper::{header, StatusCode};
use hyper_tungstenite::HyperWebsocket;
use pyo3::exceptions::{PyIOError, PyOSError, PyRuntimeError, PyTypeError};
use pyo3::prelude::*;
use pyo3::types::{PyCFunction, PyDict, PyList, PyTuple, PyType};
use tokio::fs;
use tokio::runtime::{Handle, Runtime};
use tokio_util::sync::CancellationToken;

type AsgiChannelResult =
  Result<(Sender<IncomingAsgiMessage>, Receiver<OutgoingAsgiMessage>), anyhow::Error>;
type AsgiEventLoopCommunication = Vec<(Sender<()>, Receiver<AsgiChannelResult>)>;

async fn asgi_application_fn(
  asgi_application: Arc<Py<PyAny>>,
  tx: Sender<OutgoingAsgiMessage>,
  rx: Receiver<IncomingAsgiMessage>,
) {
  let init_message = match rx.recv().await {
    Ok(IncomingAsgiMessage::Init(message)) => message,
    Err(err) => {
      tx.send(OutgoingAsgiMessage::Error(PyErr::new::<PyIOError, _>(
        err.to_string(),
      )))
      .await
      .unwrap_or_default();
      return;
    }
    _ => {
      tx.send(OutgoingAsgiMessage::Error(PyErr::new::<PyIOError, _>(
        "Unexpected message received",
      )))
      .await
      .unwrap_or_default();
      return;
    }
  };
  let tx_clone = tx.clone();
  let rx_clone = rx.clone();
  match Python::with_gil(move |py| -> PyResult<_> {
    let tx_clone = tx_clone.clone();
    let rx_clone = rx_clone.clone();

    let scope = PyDict::new(py);
    let scope_asgi = PyDict::new(py);

    match init_message {
      AsgiInitData::Lifespan => {
        scope.set_item("type", "lifespan")?;
        scope_asgi.set_item("version", "3.0")?;
      }
      AsgiInitData::Http(http_init_data) => {
        let path = http_init_data.hyper_request_parts.uri.path().to_owned();
        let query_string = http_init_data
          .hyper_request_parts
          .uri
          .query()
          .unwrap_or("")
          .to_owned();
        let original_request_uri = http_init_data
          .original_request_uri
          .unwrap_or(http_init_data.hyper_request_parts.uri);
        scope.set_item("type", "http")?;
        scope_asgi.set_item("version", "2.5")?;
        scope.set_item(
          "http_version",
          match http_init_data.hyper_request_parts.version {
            Version::HTTP_09 => "1.0", // ASGI doesn't support HTTP/0.9
            Version::HTTP_10 => "1.0",
            Version::HTTP_11 => "1.1",
            Version::HTTP_2 => "2",
            Version::HTTP_3 => "2", // ASGI doesn't support HTTP/3
            _ => "1.1",             // Some other HTTP versions, of course...
          },
        )?;
        scope.set_item(
          "method",
          http_init_data.hyper_request_parts.method.to_string(),
        )?;
        scope.set_item(
          "scheme",
          if http_init_data.socket_data.encrypted {
            "https"
          } else {
            "http"
          },
        )?;
        scope.set_item("path", urlencoding::decode(&path)?)?;
        scope.set_item("raw_path", original_request_uri.to_string().as_bytes())?;
        scope.set_item("query_string", query_string.as_bytes())?;
        if let Ok(script_path) = http_init_data
          .execute_pathbuf
          .as_path()
          .strip_prefix(http_init_data.wwwroot)
        {
          scope.set_item(
            "root_path",
            format!(
              "/{}",
              match cfg!(windows) {
                true => script_path.to_string_lossy().to_string().replace("\\", "/"),
                false => script_path.to_string_lossy().to_string(),
              }
            ),
          )?;
        }
        let headers = PyList::empty(py);
        for (header_name, header_value) in http_init_data.hyper_request_parts.headers.iter() {
          let header_name = header_name.as_str().as_bytes();
          let header_value = header_value.as_bytes();
          if !header_name.is_empty() && header_name[0] != b':' {
            headers.append(PyTuple::new(py, [header_name, header_value].into_iter())?)?;
          }
        }
        scope.set_item("headers", headers)?;
        scope.set_item(
          "client",
          (
            http_init_data
              .socket_data
              .remote_addr
              .ip()
              .to_canonical()
              .to_string(),
            http_init_data.socket_data.remote_addr.port(),
          ),
        )?;
        scope.set_item(
          "server",
          (
            http_init_data
              .socket_data
              .local_addr
              .ip()
              .to_canonical()
              .to_string(),
            http_init_data.socket_data.local_addr.port(),
          ),
        )?;
      }
    };

    scope_asgi.set_item("spec_version", "1.0")?;
    scope.set_item("asgi", scope_asgi)?;
    let scope_extensions = PyDict::new(py);
    scope_extensions.set_item("http.response.trailers", PyDict::new(py))?;
    scope.set_item("extensions", scope_extensions)?;

    let receive = PyCFunction::new_closure(
      py,
      None,
      None,
      move |args: &Bound<'_, PyTuple>, _: Option<&Bound<'_, PyDict>>| -> PyResult<_> {
        let rx = rx_clone.clone();
        Ok(
          pyo3_async_runtimes::tokio::future_into_py(args.py(), async move {
            let message = rx
              .recv()
              .await
              .map_err(|e| PyErr::new::<PyOSError, _>(e.to_string()))?;
            match message {
              IncomingAsgiMessage::Init(_) => Err(PyErr::new::<PyOSError, _>(
                "Unexpected ASGI initialization message",
              )),
              IncomingAsgiMessage::ClientDisconnected => {
                Err(PyErr::new::<PyOSError, _>("Client disconnected"))
              }
              IncomingAsgiMessage::Message(message) => incoming_struct_to_asgi_event(message),
            }
          })?
          .unbind(),
        )
      },
    )?;
    let send = PyCFunction::new_closure(
      py,
      None,
      None,
      move |args: &Bound<'_, PyTuple>, _: Option<&Bound<'_, PyDict>>| -> PyResult<_> {
        let event = args.get_item(0)?.downcast::<PyDict>()?.clone();
        let message = asgi_event_to_outgoing_struct(event)?;
        let tx = tx_clone.clone();
        Ok(
          pyo3_async_runtimes::tokio::future_into_py(args.py(), async move {
            tx.send(OutgoingAsgiMessage::Message(message))
              .await
              .map_err(|e| PyErr::new::<PyOSError, _>(e.to_string()))?;
            Ok(())
          })?
          .unbind(),
        )
      },
    )?;

    let asgi_coroutine =
      match asgi_application.call(py, (scope.clone(), receive.clone(), send.clone()), None) {
        Ok(coroutine) => coroutine,
        Err(err) => {
          if !err.get_type(py).is(&PyType::new::<PyTypeError>(py)) {
            return Err(err);
          } else {
            asgi_application
              .call(py, (scope,), None)?
              .call(py, (receive, send), None)?
          }
        }
      };

    pyo3_async_runtimes::tokio::into_future(asgi_coroutine.into_bound(py))
  }) {
    Err(err) => tx
      .send(OutgoingAsgiMessage::Error(PyErr::new::<PyRuntimeError, _>(
        err.to_string(),
      )))
      .await
      .unwrap_or_default(),
    Ok(asgi_future) => match asgi_future.await {
      Err(err) => tx
        .send(OutgoingAsgiMessage::Error(err))
        .await
        .unwrap_or_default(),
      Ok(_) => tx
        .send(OutgoingAsgiMessage::Finished)
        .await
        .unwrap_or_default(),
    },
  }
}

async fn asgi_lifetime_init_fn(asgi_applications: Vec<Arc<Py<PyAny>>>) -> Vec<AsgiChannelResult> {
  let mut results = Vec::new();
  for asgi_application in asgi_applications {
    results.push(
      async {
        let (tx, rx_task) = async_channel::unbounded::<IncomingAsgiMessage>();
        let (tx_task, rx) = async_channel::unbounded::<OutgoingAsgiMessage>();
        if let Ok(locals) = Python::with_gil(pyo3_async_runtimes::tokio::get_current_locals) {
          tokio::spawn(pyo3_async_runtimes::tokio::scope(
            locals,
            asgi_application_fn(asgi_application, tx_task, rx_task),
          ));
          tx.send(IncomingAsgiMessage::Init(AsgiInitData::Lifespan))
            .await
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
          Ok((tx, rx))
        } else {
          Err(anyhow::anyhow!("Cannot obtain task locals"))
        }
      }
      .await,
    );
  }
  results
}

async fn asgi_event_loop_fn(
  asgi_application: Arc<Py<PyAny>>,
  tx: Sender<AsgiChannelResult>,
  rx: Receiver<()>,
) {
  loop {
    if rx.recv().await.is_err() {
      continue;
    }

    let (tx_send, rx_task) = async_channel::unbounded::<IncomingAsgiMessage>();
    let (tx_task, rx_send) = async_channel::unbounded::<OutgoingAsgiMessage>();
    let asgi_application_cloned = asgi_application.clone();
    if let Ok(locals) = Python::with_gil(pyo3_async_runtimes::tokio::get_current_locals) {
      tokio::spawn(pyo3_async_runtimes::tokio::scope(
        locals,
        asgi_application_fn(asgi_application_cloned, tx_task, rx_task),
      ));
      tx.send(Ok((tx_send, rx_send))).await.unwrap_or_default();
    }
  }
}

async fn asgi_init_event_loop_fn(
  cancel_token: CancellationToken,
  asgi_applications: Vec<Arc<Py<PyAny>>>,
  mut channels: Vec<(Sender<AsgiChannelResult>, Receiver<()>)>,
) {
  Python::with_gil(|py| {
    // Try installing `uvloop`, when it fails, use `asyncio` fallback instead.
    if let Ok(uvloop) = py.import("uvloop") {
      let _ = uvloop.call_method0("install");
    }

    pyo3_async_runtimes::tokio::run::<_, ()>(py, async move {
      let asgi_lifetime_channels = asgi_lifetime_init_fn(asgi_applications.clone()).await;
      for asgi_lifetime_channel_result in &asgi_lifetime_channels {
        if let Ok((tx, rx)) = asgi_lifetime_channel_result.as_ref() {
          tx.send(IncomingAsgiMessage::Message(
            IncomingAsgiMessageInner::LifespanStartup,
          ))
          .await
          .unwrap_or_default();
          loop {
            match rx.recv().await {
              Ok(OutgoingAsgiMessage::Message(
                OutgoingAsgiMessageInner::LifespanStartupComplete,
              ))
              | Ok(OutgoingAsgiMessage::Message(
                OutgoingAsgiMessageInner::LifespanStartupFailed(_),
              ))
              | Ok(OutgoingAsgiMessage::Finished)
              | Ok(OutgoingAsgiMessage::Error(_))
              | Err(_) => break,
              _ => (),
            }
          }
        }
      }
      let init_closure = async move {
        let mut channels_len = channels.len();
        if let Some((tx_last, rx_last)) = channels.pop() {
          channels_len -= 1;
          let last_channel_id = channels_len;
          for (tx, rx) in channels {
            channels_len -= 1;
            if let Ok(locals) = Python::with_gil(pyo3_async_runtimes::tokio::get_current_locals) {
              tokio::spawn(pyo3_async_runtimes::tokio::scope(
                locals,
                asgi_event_loop_fn(asgi_applications[channels_len].clone(), tx, rx),
              ));
            }
          }

          if let Ok(locals) = Python::with_gil(pyo3_async_runtimes::tokio::get_current_locals) {
            tokio::spawn(pyo3_async_runtimes::tokio::scope(
              locals,
              asgi_event_loop_fn(asgi_applications[last_channel_id].clone(), tx_last, rx_last),
            ))
            .await
            .unwrap_or_default();
          }
        }
      };
      tokio::select! {
        _ = cancel_token.cancelled() => {}
        _ = init_closure => {}
      }
      for asgi_lifetime_channel_result in &asgi_lifetime_channels {
        if let Ok((tx, rx)) = asgi_lifetime_channel_result.as_ref() {
          tx.send(IncomingAsgiMessage::Message(
            IncomingAsgiMessageInner::LifespanShutdown,
          ))
          .await
          .unwrap_or_default();
          loop {
            match rx.recv().await {
              Ok(OutgoingAsgiMessage::Message(
                OutgoingAsgiMessageInner::LifespanShutdownComplete,
              ))
              | Ok(OutgoingAsgiMessage::Message(
                OutgoingAsgiMessageInner::LifespanShutdownFailed(_),
              ))
              | Ok(OutgoingAsgiMessage::Finished)
              | Ok(OutgoingAsgiMessage::Error(_))
              | Err(_) => break,
              _ => (),
            }
          }
        }
      }
      Ok(())
    })
  })
  .unwrap_or_default();
}

pub fn load_asgi_application(
  file_path: &Path,
  clear_sys_path: bool,
) -> Result<Py<PyAny>, Box<dyn Error + Send + Sync>> {
  let script_dirname = file_path
    .parent()
    .map(|path| path.to_string_lossy().to_string());
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
  let script_data = std::fs::read_to_string(file_path)?;
  let script_data_cstring = CString::from_str(&script_data)?;
  let asgi_application = Python::with_gil(move |py| -> PyResult<Py<PyAny>> {
    let mut sys_path_old = None;
    if let Some(script_dirname) = script_dirname {
      if let Ok(sys_module) = PyModule::import(py, "sys") {
        if let Ok(sys_path_any) = sys_module.getattr("path") {
          if let Ok(sys_path) = sys_path_any.downcast::<PyList>() {
            let sys_path = sys_path.clone();
            sys_path_old = sys_path.extract::<Vec<String>>().ok();
            sys_path.insert(0, script_dirname).unwrap_or_default();
          }
        }
      }
    }
    let asgi_application = PyModule::from_code(
      py,
      &script_data_cstring,
      &script_name_cstring,
      &module_name_cstring,
    )?
    .getattr("application")?
    .unbind();
    if clear_sys_path {
      if let Some(sys_path) = sys_path_old {
        if let Ok(sys_module) = PyModule::import(py, "sys") {
          sys_module.setattr("path", sys_path).unwrap_or_default();
        }
      }
    }
    Ok(asgi_application)
  })?;
  Ok(asgi_application)
}

pub fn server_module_init(
  config: &ServerConfig,
) -> Result<Box<dyn ServerModule + Send + Sync>, Box<dyn Error + Send + Sync>> {
  let mut asgi_applications = Vec::new();
  let mut global_asgi_application_id = None;
  let mut host_asgi_application_ids = Vec::new();
  let clear_sys_path = config["global"]["asgiClearModuleImportPath"]
    .as_bool()
    .unwrap_or(false);
  if let Some(asgi_application_path) = config["global"]["asgiApplicationPath"].as_str() {
    let asgi_application_id = asgi_applications.len();
    asgi_applications.push(Arc::new(load_asgi_application(
      PathBuf::from_str(asgi_application_path)?.as_path(),
      clear_sys_path,
    )?));
    global_asgi_application_id = Some(asgi_application_id);
  }
  let global_asgi_path = config["global"]["asgiPath"].as_str().map(|s| s.to_string());

  if let Some(hosts) = config["hosts"].as_vec() {
    for host_yaml in hosts.iter() {
      let domain = host_yaml["domain"].as_str().map(String::from);
      let ip = host_yaml["ip"].as_str().map(String::from);
      let mut locations = Vec::new();
      if let Some(locations_yaml) = host_yaml["locations"].as_vec() {
        for location_yaml in locations_yaml.iter() {
          if let Some(path_str) = location_yaml["path"].as_str() {
            let path = String::from(path_str);
            if let Some(asgi_application_path) = location_yaml["asgiApplicationPath"].as_str() {
              let asgi_application_id = asgi_applications.len();
              asgi_applications.push(Arc::new(load_asgi_application(
                PathBuf::from_str(asgi_application_path)?.as_path(),
                clear_sys_path,
              )?));

              locations.push(AsgiApplicationLocationWrap::new(
                path,
                asgi_application_id,
                location_yaml["asgiPath"].as_str().map(|s| s.to_string()),
              ));
            }
          }
        }
      }
      if let Some(asgi_application_path) = host_yaml["asgiApplicationPath"].as_str() {
        let asgi_application_id = asgi_applications.len();
        asgi_applications.push(Arc::new(load_asgi_application(
          PathBuf::from_str(asgi_application_path)?.as_path(),
          clear_sys_path,
        )?));
        host_asgi_application_ids.push(AsgiApplicationWrap::new(
          domain,
          ip,
          Some(asgi_application_id),
          host_yaml["asgiPath"].as_str().map(|s| s.to_string()),
          locations,
        ));
      } else if !locations.is_empty() {
        host_asgi_application_ids.push(AsgiApplicationWrap::new(
          domain,
          ip,
          None,
          host_yaml["asgiPath"].as_str().map(|s| s.to_string()),
          locations,
        ));
      }
    }
  }

  let cancel_token: CancellationToken = CancellationToken::new();
  let cancel_token_thread = cancel_token.clone();
  let mut asgi_event_loop_communication = Vec::new();
  let mut asgi_event_loop_communication_thread = Vec::new();

  for _ in 0..asgi_applications.len() {
    let (tx, rx_thread) = async_channel::unbounded::<()>();
    let (tx_thread, rx) = async_channel::unbounded::<AsgiChannelResult>();
    asgi_event_loop_communication.push((tx, rx));
    asgi_event_loop_communication_thread.push((tx_thread, rx_thread));
  }

  let available_parallelism = thread::available_parallelism()?.get();

  // Initialize a single-threaded (due to Python's GIL) Tokio runtime to be used as an intermediary event loop for asynchronous Python
  let mut runtime_builder = tokio::runtime::Builder::new_multi_thread();
  runtime_builder
    .worker_threads(1)
    .enable_all()
    .thread_name("python-async-pool");
  pyo3_async_runtimes::tokio::init(runtime_builder);

  // Create and spawn a task in the Tokio runtime for ASGI
  let runtime = tokio::runtime::Builder::new_multi_thread()
    .worker_threads(match available_parallelism / 2 {
      0 => 1,
      non_zero => non_zero,
    })
    .enable_all()
    .thread_name("asgi-pool")
    .build()?;

  runtime.spawn(asgi_init_event_loop_fn(
    cancel_token_thread,
    asgi_applications,
    asgi_event_loop_communication_thread,
  ));

  Ok(Box::new(AsgiModule::new(
    global_asgi_application_id,
    global_asgi_path,
    Arc::new(host_asgi_application_ids),
    cancel_token,
    asgi_event_loop_communication,
    runtime,
  )))
}

struct AsgiModule {
  global_asgi_application_id: Option<usize>,
  global_asgi_path: Option<String>,
  host_asgi_application_ids: Arc<Vec<AsgiApplicationWrap>>,
  cancel_token: CancellationToken,
  asgi_event_loop_communication: AsgiEventLoopCommunication,
  #[allow(dead_code)]
  runtime: Runtime,
}

impl AsgiModule {
  fn new(
    global_asgi_application_id: Option<usize>,
    global_asgi_path: Option<String>,
    host_asgi_application_ids: Arc<Vec<AsgiApplicationWrap>>,
    cancel_token: CancellationToken,
    asgi_event_loop_communication: AsgiEventLoopCommunication,
    runtime: Runtime,
  ) -> Self {
    AsgiModule {
      global_asgi_application_id,
      global_asgi_path,
      host_asgi_application_ids,
      cancel_token,
      asgi_event_loop_communication,
      runtime,
    }
  }
}

impl ServerModule for AsgiModule {
  fn get_handlers(&self, handle: Handle) -> Box<dyn ServerModuleHandlers + Send> {
    Box::new(AsgiModuleHandlers {
      global_asgi_application_id: self.global_asgi_application_id,
      global_asgi_path: self.global_asgi_path.clone(),
      host_asgi_application_ids: self.host_asgi_application_ids.clone(),
      asgi_event_loop_communication: self.asgi_event_loop_communication.clone(),
      handle,
    })
  }
}

impl Drop for AsgiModule {
  fn drop(&mut self) {
    self.cancel_token.cancel();
  }
}

struct AsgiModuleHandlers {
  global_asgi_application_id: Option<usize>,
  global_asgi_path: Option<String>,
  host_asgi_application_ids: Arc<Vec<AsgiApplicationWrap>>,
  asgi_event_loop_communication: AsgiEventLoopCommunication,
  handle: Handle,
}

#[async_trait]
impl ServerModuleHandlers for AsgiModuleHandlers {
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
      let mut asgi_application_id = self.global_asgi_application_id.take();
      let mut asgi_path = self.global_asgi_path.take();

      // Should have used a HashMap instead of iterating over an array for better performance...
      for host_asgi_application_wrap in self.host_asgi_application_ids.iter() {
        if match_hostname(
          match &host_asgi_application_wrap.domain {
            Some(value) => Some(value as &str),
            None => None,
          },
          match hyper_request.headers().get(header::HOST) {
            Some(value) => value.to_str().ok(),
            None => None,
          },
        ) && match &host_asgi_application_wrap.ip {
          Some(value) => ip_match(value as &str, socket_data.remote_addr.ip()),
          None => true,
        } {
          asgi_application_id = host_asgi_application_wrap.asgi_application_id;
          asgi_path = host_asgi_application_wrap.asgi_path.clone();
          if let Ok(path_decoded) = urlencoding::decode(
            request
              .get_original_url()
              .unwrap_or(request.get_hyper_request().uri())
              .path(),
          ) {
            for location_wrap in host_asgi_application_wrap.locations.iter() {
              if match_location(&location_wrap.path, &path_decoded) {
                asgi_application_id = Some(location_wrap.asgi_application_id);
                asgi_path = location_wrap.asgi_path.clone();
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

      if let Some(asgi_application_id) = asgi_application_id {
        let asgi_path = asgi_path.unwrap_or("/".to_string());
        let mut canonical_asgi_path: &str = &asgi_path;
        if canonical_asgi_path.bytes().last() == Some(b'/') {
          canonical_asgi_path = &canonical_asgi_path[..(canonical_asgi_path.len() - 1)];
        }

        let request_path_with_slashes = match request_path == canonical_asgi_path {
          true => format!("{}/", request_path),
          false => request_path.to_string(),
        };
        if let Some(stripped_request_path) =
          request_path_with_slashes.strip_prefix(canonical_asgi_path)
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

          let (tx, rx) = {
            let (tx, rx) = &self.asgi_event_loop_communication[asgi_application_id];
            tx.send(()).await?;
            rx.recv().await??
          };

          return execute_asgi(
            request,
            socket_data,
            error_logger,
            wwwroot,
            execute_pathbuf,
            execute_path_info,
            config["serverAdministratorEmail"].as_str(),
            tx,
            rx,
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

#[allow(clippy::too_many_arguments)]
async fn execute_asgi(
  request: RequestData,
  socket_data: &SocketData,
  error_logger: &ErrorLogger,
  wwwroot: &Path,
  execute_pathbuf: PathBuf,
  _path_info: Option<String>,
  _server_administrator_email: Option<&str>,
  asgi_tx: Sender<IncomingAsgiMessage>,
  asgi_rx: Receiver<OutgoingAsgiMessage>,
) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
  let (hyper_request, _, original_request_uri) = request.into_parts();
  let (hyper_request_parts, request_body) = hyper_request.into_parts();
  asgi_tx
    .send(IncomingAsgiMessage::Init(AsgiInitData::Http(
      AsgiHttpInitData {
        hyper_request_parts,
        original_request_uri,
        socket_data: SocketData {
          remote_addr: socket_data.remote_addr,
          local_addr: socket_data.local_addr,
          encrypted: socket_data.encrypted,
        },
        error_logger: error_logger.clone(),
        wwwroot: wwwroot.to_path_buf(),
        execute_pathbuf,
      },
    )))
    .await?;

  let mut request_body_stream = request_body.into_data_stream();
  let asgi_tx_clone = asgi_tx.clone();

  tokio::spawn(async move {
    loop {
      match request_body_stream.next().await {
        Some(Ok(data)) => asgi_tx_clone
          .send(IncomingAsgiMessage::Message(
            IncomingAsgiMessageInner::HttpRequest(AsgiHttpBody {
              body: data.to_vec(),
              more_body: true,
            }),
          ))
          .await
          .unwrap_or_default(),
        Some(Err(_)) => {
          asgi_tx_clone
            .send(IncomingAsgiMessage::Message(
              IncomingAsgiMessageInner::HttpDisconnect,
            ))
            .await
            .unwrap_or_default();
          asgi_tx_clone
            .send(IncomingAsgiMessage::ClientDisconnected)
            .await
            .unwrap_or_default();
        }
        None => {
          asgi_tx_clone
            .send(IncomingAsgiMessage::Message(
              IncomingAsgiMessageInner::HttpRequest(AsgiHttpBody {
                body: b"".to_vec(),
                more_body: false,
              }),
            ))
            .await
            .unwrap_or_default();
          break;
        }
      }
    }
  });

  let asgi_http_response_start;

  loop {
    match asgi_rx.recv().await? {
      OutgoingAsgiMessage::Finished => Err(anyhow::anyhow!(
        "ASGI application returned before sending the HTTP response start event"
      ))?,
      OutgoingAsgiMessage::Error(err) => Err(err)?,
      OutgoingAsgiMessage::Message(OutgoingAsgiMessageInner::HttpResponseStart(
        http_response_start,
      )) => {
        asgi_http_response_start = http_response_start;
        break;
      }
      _ => (),
    }
  }

  let response_body_stream = futures_util::stream::unfold(
    (asgi_tx, asgi_rx, false),
    move |(asgi_tx, asgi_rx, request_end)| {
      let has_trailers = asgi_http_response_start.trailers;
      async move {
        if request_end {
          asgi_tx
            .send(IncomingAsgiMessage::Message(
              IncomingAsgiMessageInner::HttpDisconnect,
            ))
            .await
            .unwrap_or_default();
          asgi_tx
            .send(IncomingAsgiMessage::ClientDisconnected)
            .await
            .unwrap_or_default();
          return None;
        }
        loop {
          match asgi_rx.recv().await {
            Err(err) => {
              return Some((
                Err(std::io::Error::other(err.to_string())),
                (asgi_tx, asgi_rx, false),
              ))
            }
            Ok(OutgoingAsgiMessage::Finished) => return None,
            Ok(OutgoingAsgiMessage::Error(err)) => {
              return Some((
                Err(std::io::Error::other(err.to_string())),
                (asgi_tx, asgi_rx, false),
              ))
            }
            Ok(OutgoingAsgiMessage::Message(OutgoingAsgiMessageInner::HttpResponseBody(
              http_response_body,
            ))) => {
              if !http_response_body.more_body {
                if http_response_body.body.is_empty() {
                  if !has_trailers {
                    asgi_tx
                      .send(IncomingAsgiMessage::Message(
                        IncomingAsgiMessageInner::HttpDisconnect,
                      ))
                      .await
                      .unwrap_or_default();
                    asgi_tx
                      .send(IncomingAsgiMessage::ClientDisconnected)
                      .await
                      .unwrap_or_default();
                    return None;
                  }
                } else {
                  return Some((
                    Ok(Frame::data(Bytes::from(http_response_body.body))),
                    (asgi_tx, asgi_rx, !has_trailers),
                  ));
                }
              } else if !http_response_body.body.is_empty() {
                return Some((
                  Ok(Frame::data(Bytes::from(http_response_body.body))),
                  (asgi_tx, asgi_rx, false),
                ));
              }
            }
            Ok(OutgoingAsgiMessage::Message(OutgoingAsgiMessageInner::HttpResponseTrailers(
              http_response_trailers,
            ))) => {
              if !http_response_trailers.more_trailers {
                if http_response_trailers.headers.is_empty() {
                  asgi_tx
                    .send(IncomingAsgiMessage::Message(
                      IncomingAsgiMessageInner::HttpDisconnect,
                    ))
                    .await
                    .unwrap_or_default();
                  asgi_tx
                    .send(IncomingAsgiMessage::ClientDisconnected)
                    .await
                    .unwrap_or_default();
                  return None;
                } else {
                  match async {
                    let mut headers = HeaderMap::new();
                    for (header_name, header_value) in http_response_trailers.headers {
                      if !header_name.is_empty() && header_name[0] != b':' {
                        headers.append(
                          HeaderName::from_bytes(&header_name)?,
                          HeaderValue::from_bytes(&header_value)?,
                        );
                      }
                    }
                    Ok::<_, Box<dyn Error + Send + Sync>>(headers)
                  }
                  .await
                  {
                    Ok(headers) => {
                      return Some((Ok(Frame::trailers(headers)), (asgi_tx, asgi_rx, true)))
                    }
                    Err(err) => {
                      return Some((
                        Err(std::io::Error::other(err.to_string())),
                        (asgi_tx, asgi_rx, false),
                      ))
                    }
                  }
                }
              } else if !http_response_trailers.headers.is_empty() {
                match async {
                  let mut headers = HeaderMap::new();
                  for (header_name, header_value) in http_response_trailers.headers {
                    if !header_name.is_empty() && header_name[0] != b':' {
                      headers.append(
                        HeaderName::from_bytes(&header_name)?,
                        HeaderValue::from_bytes(&header_value)?,
                      );
                    }
                  }
                  Ok::<_, Box<dyn Error + Send + Sync>>(headers)
                }
                .await
                {
                  Ok(headers) => {
                    return Some((Ok(Frame::trailers(headers)), (asgi_tx, asgi_rx, true)))
                  }
                  Err(err) => {
                    return Some((
                      Err(std::io::Error::other(err.to_string())),
                      (asgi_tx, asgi_rx, false),
                    ))
                  }
                }
              }
            }
            _ => (),
          }
        }
      }
    },
  );
  let response_body = BodyExt::boxed(StreamBody::new(response_body_stream));

  let mut hyper_response = Response::new(response_body);
  *hyper_response.status_mut() = StatusCode::from_u16(asgi_http_response_start.status)?;
  let headers = hyper_response.headers_mut();
  for (header_name, header_value) in asgi_http_response_start.headers {
    if !header_name.is_empty() && header_name[0] != b':' {
      headers.append(
        HeaderName::from_bytes(&header_name)?,
        HeaderValue::from_bytes(&header_value)?,
      );
    }
  }

  Ok(
    ResponseData::builder_without_request()
      .response(hyper_response)
      .build(),
  )
}
