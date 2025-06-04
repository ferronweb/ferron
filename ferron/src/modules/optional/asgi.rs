// WARNING: We have measured this module on our computers, and found it to be slower than Uvicorn (with 1 worker),
//          with FastAPI application, vanilla ASGI application is found out to be faster than Uvicorn (with 1 worker).
//          It might be more performant to just use Ferron as a reverse proxy for Uvicorn (or any other ASGI server).

use std::error::Error;
use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

use async_channel::{Receiver, Sender};
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, StreamBody};
use hyper::body::Frame;
use hyper::header::{HeaderName, HeaderValue};
use hyper::{HeaderMap, Request, Response, StatusCode, Version};
use pyo3::exceptions::{PyIOError, PyOSError, PyRuntimeError, PyTypeError};
use pyo3::prelude::*;
use pyo3::types::{PyCFunction, PyDict, PyList, PyTuple, PyType};
use tokio::runtime::Runtime;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

use crate::config::ServerConfiguration;
use crate::logging::ErrorLogger;
use crate::modules::{Module, ModuleHandlers, ModuleLoader, RequestData, ResponseData, SocketData};
use crate::util::asgi::{
  asgi_event_to_outgoing_struct, incoming_struct_to_asgi_event, AsgiHttpBody, AsgiHttpInitData,
  AsgiInitData, AsgiWebsocketClose, AsgiWebsocketInitData, AsgiWebsocketMessage,
  IncomingAsgiMessage, IncomingAsgiMessageInner, OutgoingAsgiMessage, OutgoingAsgiMessageInner,
};
use crate::util::{get_entries_for_validation, ModuleCache};
use crate::{get_entry, get_value};

/// The ASGI channel `Result`
type AsgiChannelResult =
  Result<(Sender<IncomingAsgiMessage>, Receiver<OutgoingAsgiMessage>), anyhow::Error>;

/// Channels used to communicate with the ASGI event loop
type AsgiEventLoopCommunication = (Sender<()>, Receiver<AsgiChannelResult>);

/// The function executing the ASGI application
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
        let path = http_init_data.request_parts.uri.path().to_owned();
        let query_string = http_init_data
          .request_parts
          .uri
          .query()
          .unwrap_or("")
          .to_owned();
        let original_request_uri = http_init_data
          .request_parts
          .extensions
          .get::<RequestData>()
          .and_then(|d| d.original_url.to_owned())
          .unwrap_or(http_init_data.request_parts.uri);
        scope.set_item("type", "http")?;
        scope_asgi.set_item("version", "2.5")?;
        scope.set_item(
          "http_version",
          match http_init_data.request_parts.version {
            Version::HTTP_09 => "1.0", // ASGI doesn't support HTTP/0.9
            Version::HTTP_10 => "1.0",
            Version::HTTP_11 => "1.1",
            Version::HTTP_2 => "2",
            Version::HTTP_3 => "2", // ASGI doesn't support HTTP/3
            _ => "1.1",             // Some other HTTP versions, of course...
          },
        )?;
        scope.set_item("method", http_init_data.request_parts.method.to_string())?;
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
        for (header_name, header_value) in http_init_data.request_parts.headers.iter() {
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
      AsgiInitData::Websocket(websocket_init_data) => {
        let path = websocket_init_data.request_parts.uri.path().to_owned();
        let query_string = websocket_init_data
          .request_parts
          .uri
          .query()
          .unwrap_or("")
          .to_owned();
        let original_request_uri = websocket_init_data
          .request_parts
          .extensions
          .get::<RequestData>()
          .and_then(|d| d.original_url.to_owned())
          .unwrap_or(websocket_init_data.request_parts.uri);
        scope.set_item("type", "websocket")?;
        scope_asgi.set_item("version", "2.5")?;
        scope.set_item(
          "http_version",
          "1.1", // WebSocket is supported only on HTTP/1.1 in Ferron
        )?;
        scope.set_item(
          "scheme",
          if websocket_init_data.socket_data.encrypted {
            "wss"
          } else {
            "ws"
          },
        )?;
        scope.set_item("path", urlencoding::decode(&path)?)?;
        scope.set_item("raw_path", original_request_uri.to_string().as_bytes())?;
        scope.set_item("query_string", query_string.as_bytes())?;
        if let Ok(script_path) = websocket_init_data
          .execute_pathbuf
          .as_path()
          .strip_prefix(websocket_init_data.wwwroot)
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
        for (header_name, header_value) in websocket_init_data.request_parts.headers.iter() {
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
            websocket_init_data
              .socket_data
              .remote_addr
              .ip()
              .to_canonical()
              .to_string(),
            websocket_init_data.socket_data.remote_addr.port(),
          ),
        )?;
        scope.set_item(
          "server",
          (
            websocket_init_data
              .socket_data
              .local_addr
              .ip()
              .to_canonical()
              .to_string(),
            websocket_init_data.socket_data.local_addr.port(),
          ),
        )?;
        scope.set_item("subprotocols", PyList::empty(py))?;
      }
    };

    scope_asgi.set_item("spec_version", "1.0")?;
    scope.set_item("asgi", scope_asgi)?;
    let scope_extensions = PyDict::new(py);
    scope_extensions.set_item("http.response.trailers", PyDict::new(py))?;
    scope.set_item("extensions", scope_extensions)?;

    let client_disconnected = Arc::new(AtomicBool::new(false));
    let client_disconnected_clone = client_disconnected.clone();

    let receive = PyCFunction::new_closure(
      py,
      None,
      None,
      move |args: &Bound<'_, PyTuple>, _: Option<&Bound<'_, PyDict>>| -> PyResult<_> {
        let rx = rx_clone.clone();
        let client_disconnected = client_disconnected.clone();
        Ok(
          pyo3_async_runtimes::tokio::future_into_py(args.py(), async move {
            if client_disconnected.load(Ordering::Relaxed) {
              Err(PyErr::new::<PyOSError, _>("Client disconnected"))
            } else {
              let message = rx
                .recv()
                .await
                .map_err(|e| PyErr::new::<PyOSError, _>(e.to_string()))?;
              match message {
                IncomingAsgiMessage::Init(_) => Err(PyErr::new::<PyOSError, _>(
                  "Unexpected ASGI initialization message",
                )),
                IncomingAsgiMessage::Message(message) => {
                  if let IncomingAsgiMessageInner::HttpDisconnect = &message {
                    client_disconnected.store(true, Ordering::Relaxed);
                  }
                  incoming_struct_to_asgi_event(message)
                }
              }
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
        let client_disconnected = client_disconnected_clone.clone();
        Ok(
          pyo3_async_runtimes::tokio::future_into_py(args.py(), async move {
            if client_disconnected.load(Ordering::Relaxed) {
              Err(PyErr::new::<PyOSError, _>("Client disconnected"))
            } else {
              tx.send(OutgoingAsgiMessage::Message(message))
                .await
                .map_err(|e| PyErr::new::<PyOSError, _>(e.to_string()))?;
              Ok(())
            }
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

/// The ASGI lifetime protocol initialization function
async fn asgi_lifetime_init_fn(asgi_application: Arc<Py<PyAny>>) -> AsgiChannelResult {
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

/// The ASGI event loop function
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

/// The function for initializing ASGI event loop
async fn asgi_init_event_loop_fn(
  cancel_token: CancellationToken,
  asgi_application: Arc<Py<PyAny>>,
  channel: (Sender<AsgiChannelResult>, Receiver<()>),
) {
  Python::with_gil(|py| {
    // Try installing `uvloop`, when it fails, use `asyncio` fallback instead.
    if let Ok(uvloop) = py.import("uvloop") {
      let _ = uvloop.call_method0("install");
    }

    pyo3_async_runtimes::tokio::run::<_, ()>(py, async move {
      let asgi_lifetime_channel_result = asgi_lifetime_init_fn(asgi_application.clone()).await;
      if let Ok((tx, rx)) = asgi_lifetime_channel_result.as_ref() {
        tx.send(IncomingAsgiMessage::Message(
          IncomingAsgiMessageInner::LifespanStartup,
        ))
        .await
        .unwrap_or_default();
        loop {
          match rx.recv().await {
            Ok(OutgoingAsgiMessage::Message(OutgoingAsgiMessageInner::LifespanStartupComplete))
            | Ok(OutgoingAsgiMessage::Message(OutgoingAsgiMessageInner::LifespanStartupFailed(
              _,
            )))
            | Ok(OutgoingAsgiMessage::Finished)
            | Ok(OutgoingAsgiMessage::Error(_))
            | Err(_) => break,
            _ => (),
          }
        }
      }
      let init_closure = async move {
        let (tx, rx) = channel;

        if let Ok(locals) = Python::with_gil(pyo3_async_runtimes::tokio::get_current_locals) {
          tokio::spawn(pyo3_async_runtimes::tokio::scope(
            locals,
            asgi_event_loop_fn(asgi_application.clone(), tx, rx),
          ))
          .await
          .unwrap_or_default();
        }
      };
      tokio::select! {
        _ = cancel_token.cancelled() => {}
        _ = init_closure => {}
      }
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
            | Ok(OutgoingAsgiMessage::Message(OutgoingAsgiMessageInner::LifespanShutdownFailed(
              _,
            )))
            | Ok(OutgoingAsgiMessage::Finished)
            | Ok(OutgoingAsgiMessage::Error(_))
            | Err(_) => break,
            _ => (),
          }
        }
      }
      Ok(())
    })
  })
  .unwrap_or_default();
}

/// Loads an ASGI application
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

/// An ASGI module loader
pub struct AsgiModuleLoader {
  cache: ModuleCache<AsgiModule>,
  tokio_runtime: Option<Runtime>,
}

impl AsgiModuleLoader {
  /// Creates a new module loader
  pub fn new() -> Self {
    Self {
      cache: ModuleCache::new(vec!["asgi"]),
      tokio_runtime: None,
    }
  }
}

impl ModuleLoader for AsgiModuleLoader {
  fn load_module(
    &mut self,
    config: &ServerConfiguration,
    global_config: Option<&ServerConfiguration>,
  ) -> Result<Arc<dyn Module + Send + Sync>, Box<dyn Error + Send + Sync>> {
    Ok(
      self
        .cache
        .get_or_init::<_, Box<dyn std::error::Error + Send + Sync>>(config, |config| {
          let clear_sys_path = global_config
            .and_then(|c| get_value!("asgi_clear_imports", c))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
          if let Some(asgi_application_path) = get_value!("asgi", config).and_then(|v| v.as_str()) {
            let runtime: &Runtime = if let Some(runtime) = self.tokio_runtime.as_ref() {
              runtime
            } else {
              let available_parallelism = thread::available_parallelism()?.get();

              // Initialize a single-threaded (due to Python's GIL) Tokio runtime to be used as an intermediary event loop for asynchronous Python
              let mut runtime_builder = tokio::runtime::Builder::new_multi_thread();
              runtime_builder
                .worker_threads(1)
                .enable_all()
                .thread_name("python-async-pool");
              pyo3_async_runtimes::tokio::init(runtime_builder);

              // Create a Tokio runtime for ASGI
              let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(match available_parallelism / 2 {
                  0 => 1,
                  non_zero => non_zero,
                })
                .enable_all()
                .thread_name("asgi-pool")
                .build()?;

              self.tokio_runtime = Some(runtime);
              self
                .tokio_runtime
                .as_ref()
                .ok_or(anyhow::anyhow!("Tokio runtime initialization failed"))?
            };

            let asgi_application = Arc::new(load_asgi_application(
              PathBuf::from_str(asgi_application_path)?.as_path(),
              clear_sys_path,
            )?);

            let (tx, rx_thread) = async_channel::unbounded::<()>();
            let (tx_thread, rx) = async_channel::unbounded::<AsgiChannelResult>();

            let cancel_token: CancellationToken = CancellationToken::new();
            let cancel_token_thread = cancel_token.clone();
            runtime.spawn(asgi_init_event_loop_fn(
              cancel_token_thread,
              asgi_application,
              (tx_thread, rx_thread),
            ));

            Ok(Arc::new(AsgiModule {
              asgi_application_communication: Some(Arc::new((tx, rx))),
              cancel_token: Some(cancel_token),
            }))
          } else {
            Ok(Arc::new(AsgiModule {
              asgi_application_communication: None,
              cancel_token: None,
            }))
          }
        })?,
    )
  }

  fn get_requirements(&self) -> Vec<&'static str> {
    vec!["asgi"]
  }

  fn validate_configuration(
    &self,
    config: &ServerConfiguration,
    used_properties: &mut std::collections::HashSet<String>,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    if let Some(entries) = get_entries_for_validation!("asgi", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `asgi` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_string() && !entry.values[0].is_null() {
          Err(anyhow::anyhow!(
            "The ASGI application path must be a string"
          ))?
        }
      }
    };

    if let Some(entries) =
      get_entries_for_validation!("asgi_clear_imports", config, used_properties)
    {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `asgi_clear_imports` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_string() && !entry.values[0].is_null() {
          Err(anyhow::anyhow!(
            "Invalid ASGI Python import clearing option"
          ))?
        }
      }
    };

    Ok(())
  }
}

/// An ASGI module
struct AsgiModule {
  asgi_application_communication: Option<Arc<AsgiEventLoopCommunication>>,
  cancel_token: Option<CancellationToken>,
}

impl Module for AsgiModule {
  fn get_module_handlers(&self) -> Box<dyn ModuleHandlers> {
    Box::new(AsgiModuleHandlers {
      asgi_application_communication: self.asgi_application_communication.clone(),
    })
  }
}

impl Drop for AsgiModule {
  fn drop(&mut self) {
    if let Some(cancel_token) = self.cancel_token.take() {
      cancel_token.cancel();
    }
  }
}

/// Handlers for the ASGI module
struct AsgiModuleHandlers {
  asgi_application_communication: Option<Arc<AsgiEventLoopCommunication>>,
}

#[async_trait(?Send)]
impl ModuleHandlers for AsgiModuleHandlers {
  async fn request_handler(
    &mut self,
    request: Request<BoxBody<Bytes, std::io::Error>>,
    config: &ServerConfiguration,
    socket_data: &SocketData,
    error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    if let Some(asgi_application_communication) = self.asgi_application_communication.clone() {
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

      let (tx, rx) = {
        let (tx, rx) = &*asgi_application_communication;
        tx.send(()).await?;
        rx.recv().await??
      };

      if hyper_tungstenite::is_upgrade_request(&request) {
        return execute_asgi_websocket(
          request,
          socket_data,
          error_logger,
          wwwroot,
          execute_pathbuf,
          tx,
          rx,
        )
        .await;
      } else {
        return execute_asgi(
          request,
          socket_data,
          error_logger,
          wwwroot,
          execute_pathbuf,
          tx,
          rx,
        )
        .await;
      }
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

/// The ASGI handler for HTTP requests
#[allow(clippy::too_many_arguments)]
async fn execute_asgi(
  request: Request<BoxBody<Bytes, std::io::Error>>,
  socket_data: &SocketData,
  error_logger: &ErrorLogger,
  wwwroot: &Path,
  execute_pathbuf: PathBuf,
  asgi_tx: Sender<IncomingAsgiMessage>,
  asgi_rx: Receiver<OutgoingAsgiMessage>,
) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
  let (request_parts, request_body) = request.into_parts();
  asgi_tx
    .send(IncomingAsgiMessage::Init(AsgiInitData::Http(
      AsgiHttpInitData {
        request_parts,
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

  crate::runtime::spawn(async move {
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

  let mut response = Response::new(response_body);
  *response.status_mut() = StatusCode::from_u16(asgi_http_response_start.status)?;
  let headers = response.headers_mut();
  for (header_name, header_value) in asgi_http_response_start.headers {
    if !header_name.is_empty() && header_name[0] != b':' {
      headers.append(
        HeaderName::from_bytes(&header_name)?,
        HeaderValue::from_bytes(&header_value)?,
      );
    }
  }

  Ok(ResponseData {
    request: None,
    response: Some(response),
    response_status: None,
    response_headers: None,
    new_remote_address: None,
  })
}

/// The ASGI handler for WebSocket requests
#[allow(clippy::too_many_arguments)]
async fn execute_asgi_websocket(
  request: Request<BoxBody<Bytes, std::io::Error>>,
  socket_data: &SocketData,
  error_logger: &ErrorLogger,
  wwwroot: &Path,
  execute_pathbuf: PathBuf,
  asgi_tx: Sender<IncomingAsgiMessage>,
  asgi_rx: Receiver<OutgoingAsgiMessage>,
) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
  let (request_parts, _) = request.into_parts();
  asgi_tx
    .send(IncomingAsgiMessage::Init(AsgiInitData::Websocket(
      AsgiWebsocketInitData {
        request_parts: request_parts.clone(),
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

  asgi_tx
    .send(IncomingAsgiMessage::Message(
      IncomingAsgiMessageInner::WebsocketConnect,
    ))
    .await?;

  loop {
    match asgi_rx.recv().await? {
      OutgoingAsgiMessage::Finished => Err(anyhow::anyhow!(
        "ASGI application returned before sending the WebSocket accept event"
      ))?,
      OutgoingAsgiMessage::Error(err) => Err(err)?,
      OutgoingAsgiMessage::Message(OutgoingAsgiMessageInner::WebsocketAccept(_)) => {
        let (response, websocket) =
          hyper_tungstenite::upgrade(Request::from_parts(request_parts, ()), None)?;

        let error_logger = error_logger.clone();
        crate::runtime::spawn(async move {
          let client_bi_stream = match websocket.await {
            Ok(stream) => stream,
            Err(err) => {
              error_logger
                .log(&format!(
                  "There was an error when upgrading the HTTP request to WebSocket: {}",
                  err
                ))
                .await;
              return;
            }
          };

          let (client_sink, mut client_stream) = client_bi_stream.split();

          let client_disconnected_mutex = Arc::new(Mutex::new(AtomicBool::new(false)));
          let client_disconnected_mutex_clone = client_disconnected_mutex.clone();

          let asgi_tx_clone = asgi_tx.clone();
          let (ping, pong) = async_channel::unbounded();

          crate::runtime::spawn(async move {
            while let Some(websocket_frame) = client_stream.next().await {
              match websocket_frame {
                Err(_) => {
                  let client_disconnected = client_disconnected_mutex_clone.lock().await;
                  if !client_disconnected.load(Ordering::Relaxed) {
                    client_disconnected.store(true, Ordering::Relaxed);
                    asgi_tx_clone
                      .send(IncomingAsgiMessage::Message(
                        IncomingAsgiMessageInner::WebsocketDisconnect(AsgiWebsocketClose {
                          code: 1005,
                          reason: "Error while receiving WebSocket data".to_string(),
                        }),
                      ))
                      .await
                      .unwrap_or_default();
                  }
                }
                Ok(Message::Ping(message)) => {
                  ping.send(message).await.unwrap_or_default();
                }
                Ok(Message::Binary(message)) => {
                  asgi_tx_clone
                    .send(IncomingAsgiMessage::Message(
                      IncomingAsgiMessageInner::WebsocketReceive(AsgiWebsocketMessage {
                        bytes: Some(message.to_vec()),
                        text: None,
                      }),
                    ))
                    .await
                    .unwrap_or_default();
                }
                Ok(Message::Text(message)) => {
                  asgi_tx_clone
                    .send(IncomingAsgiMessage::Message(
                      IncomingAsgiMessageInner::WebsocketReceive(AsgiWebsocketMessage {
                        bytes: None,
                        text: Some(message.to_string()),
                      }),
                    ))
                    .await
                    .unwrap_or_default();
                }
                Ok(Message::Close(close_frame)) => {
                  let client_disconnected = client_disconnected_mutex_clone.lock().await;
                  if !client_disconnected.load(Ordering::Relaxed) {
                    client_disconnected.store(true, Ordering::Relaxed);
                    client_disconnected_mutex_clone
                      .lock()
                      .await
                      .store(true, Ordering::Relaxed);
                    let (status_code, message) = if let Some(close_frame) = close_frame {
                      (close_frame.code.into(), close_frame.reason.to_string())
                    } else {
                      (
                        1005,
                        "Websocket connection closed for unknown reason".to_string(),
                      )
                    };
                    asgi_tx_clone
                      .send(IncomingAsgiMessage::Message(
                        IncomingAsgiMessageInner::WebsocketDisconnect(AsgiWebsocketClose {
                          code: status_code,
                          reason: message,
                        }),
                      ))
                      .await
                      .unwrap_or_default();
                  }
                }
                _ => (),
              }
            }
          });

          let client_sink_mutex = Arc::new(Mutex::new(client_sink));
          let client_sink_mutex_cloned = client_sink_mutex.clone();

          crate::runtime::spawn(async move {
            while let Ok(message) = pong.recv().await {
              if client_sink_mutex_cloned
                .lock()
                .await
                .send(Message::Pong(message))
                .await
                .is_err()
              {
                break;
              }
            }
          });

          let websocket_future = async move {
            loop {
              match asgi_rx.recv().await? {
                OutgoingAsgiMessage::Finished => Err(anyhow::anyhow!(
                  "ASGI application returned before sending the WebSocket accept event"
                ))?,
                OutgoingAsgiMessage::Error(err) => Err(err)?,
                OutgoingAsgiMessage::Message(OutgoingAsgiMessageInner::WebsocketSend(
                  websocket_message,
                )) => {
                  let frame_option = if let Some(bytes) = websocket_message.bytes {
                    Some(Message::binary(bytes))
                  } else {
                    websocket_message.text.map(Message::text)
                  };
                  if let Some(frame) = frame_option {
                    let mut client_sink = client_sink_mutex.lock().await;
                    if let Err(err) = client_sink.send(frame).await {
                      drop(client_sink);
                      let client_disconnected = client_disconnected_mutex.lock().await;
                      if !client_disconnected.load(Ordering::Relaxed) {
                        client_disconnected.store(true, Ordering::Relaxed);
                        asgi_tx
                          .send(IncomingAsgiMessage::Message(
                            IncomingAsgiMessageInner::WebsocketDisconnect(AsgiWebsocketClose {
                              code: 1005,
                              reason: "Error while sending WebSocket data".to_string(),
                            }),
                          ))
                          .await
                          .unwrap_or_default();
                      }
                      Err(err)?;
                    }
                  }
                }
                OutgoingAsgiMessage::Message(OutgoingAsgiMessageInner::WebsocketClose(
                  websocket_close,
                )) => {
                  let client_disconnected = client_disconnected_mutex.lock().await;
                  if !client_disconnected.load(Ordering::Relaxed) {
                    client_disconnected.store(true, Ordering::Relaxed);
                    asgi_tx
                      .send(IncomingAsgiMessage::Message(
                        IncomingAsgiMessageInner::WebsocketDisconnect(AsgiWebsocketClose {
                          code: websocket_close.code,
                          reason: websocket_close.reason.clone(),
                        }),
                      ))
                      .await
                      .unwrap_or_default();
                  }
                  let mut client_sink = client_sink_mutex.lock().await;
                  client_sink
                    .send(Message::Close(Some(CloseFrame {
                      code: websocket_close.code.into(),
                      reason: websocket_close.reason.into(),
                    })))
                    .await?;
                  client_sink.close().await.unwrap_or_default();
                  break;
                }
                _ => (),
              }
            }

            Ok::<_, Box<dyn Error + Send + Sync>>(())
          };

          crate::runtime::spawn(async move {
            if let Err(err) = websocket_future.await {
              error_logger
                .log(&format!(
                  "There was an error when processing an ASGI WebSocket request: {}",
                  err
                ))
                .await;
            }
          });
        });

        return Ok(ResponseData {
          request: None,
          response: Some(response.map(|b| b.map_err(|e| match e {}).boxed())),
          response_status: None,
          response_headers: None,
          new_remote_address: None,
        });
      }
      OutgoingAsgiMessage::Message(OutgoingAsgiMessageInner::WebsocketClose(_)) => {
        asgi_tx
          .send(IncomingAsgiMessage::Message(
            IncomingAsgiMessageInner::WebsocketDisconnect(AsgiWebsocketClose {
              code: 1005,
              reason: "ASGI application closed the WebSocket connection before accepting it"
                .to_string(),
            }),
          ))
          .await
          .unwrap_or_default();
        return Ok(ResponseData {
          request: None,
          response: None,
          response_status: Some(StatusCode::FORBIDDEN),
          response_headers: None,
          new_remote_address: None,
        });
      }
      _ => (),
    }
  }
}
