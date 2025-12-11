use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use ferron_common::logging::ErrorLogger;
use hyper::header::{HeaderName, HeaderValue};
use hyper::{HeaderMap, StatusCode};
use parking_lot::RwLock;
use rhai::{Dynamic, FnPtr, ImmutableString};

/// Identifies the context in which a script executes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptPhase {
  RequestStart,
  RequestBody,
  Response,
  Tick,
  BackgroundTask,
}

/// Represents the result requested by the script.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScriptDecision {
  Continue,
  Deny { status: StatusCode, body: Vec<u8> },
}

impl ScriptDecision {
  #[allow(dead_code)]
  pub fn is_terminating(&self) -> bool {
    !matches!(self, ScriptDecision::Continue)
  }
}

/// A mutable state store shared between scripts.
#[derive(Clone, Default)]
pub struct ScriptStateHandle {
  inner: Arc<RwLock<HashMap<String, Dynamic>>>,
}

impl ScriptStateHandle {
  pub fn new() -> Self {
    Self::default()
  }

  pub fn get(&self, key: &str) -> Dynamic {
    self.inner.read().get(key).cloned().unwrap_or(Dynamic::UNIT)
  }

  pub fn set(&self, key: &str, value: Dynamic) {
    self.inner.write().insert(key.to_string(), value);
  }

  pub fn remove(&self, key: &str) {
    self.inner.write().remove(key);
  }

  pub fn clear(&self) {
    self.inner.write().clear();
  }

  pub fn keys(&self) -> Vec<String> {
    self.inner.read().keys().cloned().collect()
  }
}

fn header_map_to_vec(headers: &HeaderMap) -> Vec<(String, String)> {
  headers
    .iter()
    .map(|(name, value)| {
      (
        name.as_str().to_string(),
        value.to_str().map(ToString::to_string).unwrap_or_default(),
      )
    })
    .collect()
}

fn headers_vec_to_map(headers: &[(String, String)]) -> HeaderMap {
  let mut map = HeaderMap::new();
  for (name, value) in headers {
    if let (Ok(header_name), Ok(header_value)) = (HeaderName::try_from(name.as_str()), HeaderValue::from_str(value)) {
      map.append(header_name, header_value);
    }
  }
  map
}

fn names_equal(left: &str, right: &str) -> bool {
  left.eq_ignore_ascii_case(right)
}

#[derive(Clone)]
pub struct ScriptRequestHandle {
  inner: Arc<RwLock<ScriptRequestData>>,
}

#[derive(Clone, Debug)]
pub struct ScriptRequestSnapshot {
  pub method: String,
  pub uri: String,
  pub headers: Vec<(String, String)>,
  pub body: Vec<u8>,
  pub body_modified: bool,
}

#[derive(Clone, Debug)]
struct ScriptRequestData {
  method: String,
  uri: String,
  headers: Vec<(String, String)>,
  body: Vec<u8>,
  body_modified: bool,
}

impl ScriptRequestHandle {
  pub fn from_parts(method: &str, uri: &str, headers: &HeaderMap, body: Vec<u8>) -> Self {
    Self {
      inner: Arc::new(RwLock::new(ScriptRequestData {
        method: method.to_string(),
        uri: uri.to_string(),
        headers: header_map_to_vec(headers),
        body,
        body_modified: false,
      })),
    }
  }

  pub fn snapshot(&self) -> ScriptRequestSnapshot {
    let guard = self.inner.read();
    ScriptRequestSnapshot {
      method: guard.method.clone(),
      uri: guard.uri.clone(),
      headers: guard.headers.clone(),
      body: guard.body.clone(),
      body_modified: guard.body_modified,
    }
  }

  pub fn get_method(&self) -> String {
    self.inner.read().method.clone()
  }

  pub fn set_method(&self, method: &str) {
    self.inner.write().method = method.to_string();
  }

  pub fn get_uri(&self) -> String {
    self.inner.read().uri.clone()
  }

  pub fn set_uri(&self, uri: &str) {
    self.inner.write().uri = uri.to_string();
  }

  pub fn get_body(&self) -> Vec<u8> {
    self.inner.read().body.clone()
  }

  pub fn set_body(&self, body: Vec<u8>) {
    let mut guard = self.inner.write();
    guard.body = body;
    guard.body_modified = true;
  }

  pub fn get_header(&self, name: &str) -> Option<String> {
    self
      .inner
      .read()
      .headers
      .iter()
      .rfind(|(n, _)| names_equal(n, name))
      .map(|(_, v)| v.clone())
  }

  pub fn set_header(&self, name: &str, value: &str) {
    let mut guard = self.inner.write();
    guard.headers.retain(|(n, _)| !names_equal(n, name));
    guard.headers.push((name.to_ascii_lowercase(), value.to_string()));
  }

  pub fn remove_header(&self, name: &str) {
    self.inner.write().headers.retain(|(n, _)| !names_equal(n, name));
  }
}

#[derive(Clone)]
pub struct ScriptResponseHandle {
  inner: Arc<RwLock<ScriptResponseData>>,
}

#[derive(Clone, Debug, Default)]
pub struct ScriptResponseSnapshot {
  pub status: Option<u16>,
  pub headers: Vec<(String, String)>,
  pub body: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
struct ScriptResponseData {
  status: Option<u16>,
  headers: Vec<(String, String)>,
  body: Vec<u8>,
}

impl ScriptResponseHandle {
  pub fn new() -> Self {
    Self {
      inner: Arc::new(RwLock::new(ScriptResponseData::default())),
    }
  }

  pub fn snapshot(&self) -> ScriptResponseSnapshot {
    let guard = self.inner.read();
    ScriptResponseSnapshot {
      status: guard.status,
      headers: guard.headers.clone(),
      body: guard.body.clone(),
    }
  }

  pub fn set_status(&self, status: u16) {
    self.inner.write().status = Some(status);
  }

  pub fn get_status(&self) -> Option<i64> {
    self.inner.read().status.map(|s| s as i64)
  }

  pub fn get_body(&self) -> Vec<u8> {
    self.inner.read().body.clone()
  }

  pub fn set_body(&self, body: Vec<u8>) {
    self.inner.write().body = body;
  }

  pub fn get_header(&self, name: &str) -> Option<String> {
    self
      .inner
      .read()
      .headers
      .iter()
      .rfind(|(n, _)| names_equal(n, name))
      .map(|(_, v)| v.clone())
  }

  pub fn set_header(&self, name: &str, value: &str) {
    let mut guard = self.inner.write();
    guard.headers.retain(|(n, _)| !names_equal(n, name));
    guard.headers.push((name.to_ascii_lowercase(), value.to_string()));
  }

  pub fn remove_header(&self, name: &str) {
    self.inner.write().headers.retain(|(n, _)| !names_equal(n, name));
  }
}

/// Shared context for host APIs executed during script evaluation.
pub struct ScriptExecutionContext<'a> {
  pub script_id: &'a str,
  pub phase: ScriptPhase,
  pub request: Option<ScriptRequestHandle>,
  pub response: ScriptResponseHandle,
  #[allow(dead_code)]
  pub env: Arc<HashMap<String, String>>,
  #[allow(dead_code)]
  pub state: ScriptStateHandle,
  pub decision: ScriptDecision,
  #[allow(dead_code)]
  pub logger: &'a ErrorLogger,
  pub allow_spawn_task: bool,
  pub spawn_callback: Option<Arc<dyn Fn(String, FnPtr) + Send + Sync + 'static>>,
  pub pending_logs: Vec<String>,
}

impl<'a> ScriptExecutionContext<'a> {
  #[allow(clippy::too_many_arguments)]
  pub fn new(
    script_id: &'a str,
    phase: ScriptPhase,
    request: Option<ScriptRequestHandle>,
    response: ScriptResponseHandle,
    env: Arc<HashMap<String, String>>,
    state: ScriptStateHandle,
    logger: &'a ErrorLogger,
    allow_spawn_task: bool,
    spawn_callback: Option<Arc<dyn Fn(String, FnPtr) + Send + Sync + 'static>>,
  ) -> Self {
    Self {
      script_id,
      phase,
      request,
      response,
      env,
      state,
      decision: ScriptDecision::Continue,
      logger,
      allow_spawn_task,
      spawn_callback,
      pending_logs: Vec::new(),
    }
  }

  pub fn log(&mut self, level: &str, message: &str) {
    self
      .pending_logs
      .push(format!("[script:{}][{}] {}", self.script_id, level, message));
  }

  pub fn drain_logs(&mut self) -> Vec<String> {
    std::mem::take(&mut self.pending_logs)
  }
}

thread_local! {
  static ACTIVE_CONTEXT: std::cell::RefCell<Option<*mut ScriptExecutionContext<'static>>> =
    const { std::cell::RefCell::new(None) };
}

pub struct ContextGuard;

impl ContextGuard {
  pub fn activate(ctx: *mut ScriptExecutionContext<'_>) -> Self {
    ACTIVE_CONTEXT.with(|slot| {
      slot.replace(Some(ctx.cast::<ScriptExecutionContext<'static>>()));
    });
    Self
  }
}

impl Drop for ContextGuard {
  fn drop(&mut self) {
    ACTIVE_CONTEXT.with(|slot| {
      slot.replace(None);
    });
  }
}

fn with_context<F, R>(func: F) -> Option<R>
where
  F: FnOnce(&mut ScriptExecutionContext<'_>) -> R,
{
  ACTIVE_CONTEXT.with(|slot| {
    slot
      .borrow()
      .map(|ctx| unsafe { func(&mut *ctx.cast::<ScriptExecutionContext<'_>>()) })
  })
}

/// Converts the script environment into a `Dynamic` value for the Rhai scope.
pub fn env_to_dynamic(env: &HashMap<String, String>) -> Dynamic {
  let mut map = rhai::Map::new();
  for (key, value) in env {
    let owned_key = key.clone();
    let owned_value = value.clone();
    map.insert(owned_key.into(), Dynamic::from(owned_value));
  }
  Dynamic::from_map(map)
}

/// Registers script-visible types and helpers on the provided engine.
pub fn register_types(engine: &mut rhai::Engine) {
  engine.register_type_with_name::<ScriptRequestHandle>("Request");
  engine.register_type_with_name::<ScriptResponseHandle>("Response");
  engine.register_type_with_name::<ScriptStateHandle>("StateStore");

  engine.register_get("method", |req: &mut ScriptRequestHandle| req.get_method());
  engine.register_set("method", |req: &mut ScriptRequestHandle, value: ImmutableString| {
    req.set_method(&value);
  });
  engine.register_get("uri", |req: &mut ScriptRequestHandle| req.get_uri());
  engine.register_set("uri", |req: &mut ScriptRequestHandle, value: ImmutableString| {
    req.set_uri(&value);
  });
  engine.register_fn("get_header", |req: &mut ScriptRequestHandle, name: ImmutableString| {
    req.get_header(&name)
  });
  engine.register_fn("header", |req: &mut ScriptRequestHandle, name: ImmutableString| {
    req.get_header(&name).map(Dynamic::from).unwrap_or(Dynamic::UNIT)
  });
  engine.register_fn(
    "set_header",
    |req: &mut ScriptRequestHandle, name: ImmutableString, value: ImmutableString| {
      req.set_header(&name, &value);
    },
  );
  engine.register_fn(
    "remove_header",
    |req: &mut ScriptRequestHandle, name: ImmutableString| {
      req.remove_header(&name);
    },
  );
  engine.register_get("body", |req: &mut ScriptRequestHandle| req.get_body());
  engine.register_set("body", |req: &mut ScriptRequestHandle, value: Vec<u8>| {
    req.set_body(value);
  });

  engine.register_get("status", |resp: &mut ScriptResponseHandle| {
    resp.get_status().unwrap_or_default()
  });
  engine.register_set("status", |resp: &mut ScriptResponseHandle, value: rhai::INT| {
    resp.set_status(value as u16);
  });
  engine.register_fn("set_status", |resp: &mut ScriptResponseHandle, value: rhai::INT| {
    resp.set_status(value as u16);
  });
  engine.register_fn(
    "get_header",
    |resp: &mut ScriptResponseHandle, name: ImmutableString| resp.get_header(&name),
  );
  engine.register_fn(
    "set_header",
    |resp: &mut ScriptResponseHandle, name: ImmutableString, value: ImmutableString| {
      resp.set_header(&name, &value);
    },
  );
  engine.register_fn(
    "remove_header",
    |resp: &mut ScriptResponseHandle, name: ImmutableString| {
      resp.remove_header(&name);
    },
  );
  engine.register_get("body", |resp: &mut ScriptResponseHandle| resp.get_body());
  engine.register_set("body", |resp: &mut ScriptResponseHandle, value: Vec<u8>| {
    resp.set_body(value);
  });
  engine.register_fn("set_body", |resp: &mut ScriptResponseHandle, value: Vec<u8>| {
    resp.set_body(value);
  });
  engine.register_fn("set_body", |resp: &mut ScriptResponseHandle, value: ImmutableString| {
    resp.set_body(value.as_str().as_bytes().to_vec());
  });

  engine.register_fn("get", |state: &mut ScriptStateHandle, key: ImmutableString| {
    state.get(&key)
  });
  engine.register_fn(
    "set",
    |state: &mut ScriptStateHandle, key: ImmutableString, value: Dynamic| {
      state.set(&key, value);
    },
  );
  engine.register_fn("remove", |state: &mut ScriptStateHandle, key: ImmutableString| {
    state.remove(&key);
  });
  engine.register_fn("clear", |state: &mut ScriptStateHandle| state.clear());
  engine.register_fn("keys", |state: &mut ScriptStateHandle| state.keys());

  engine.register_fn("log", host_log);
  engine.register_fn("set_header", host_set_header);
  engine.register_fn("remove_header", host_remove_header);
  engine.register_fn("deny", host_deny);
  engine.register_fn("spawn_task", host_spawn_task);
}

fn host_log(level: ImmutableString, message: ImmutableString) {
  let level = level.to_string();
  let message = message.to_string();
  let _ = with_context(|ctx| {
    ctx.log(&level, &message);
  });
}

fn update_headers_for_phase(name: &str, value: Option<&str>) {
  with_context(|ctx| match ctx.phase {
    ScriptPhase::RequestStart | ScriptPhase::RequestBody => {
      if let Some(request) = &ctx.request {
        match value {
          Some(value) => request.set_header(name, value),
          None => request.remove_header(name),
        }
      }
    }
    ScriptPhase::Response | ScriptPhase::Tick | ScriptPhase::BackgroundTask => match value {
      Some(value) => ctx.response.set_header(name, value),
      None => ctx.response.remove_header(name),
    },
  });
}

fn host_set_header(name: ImmutableString, value: ImmutableString) {
  update_headers_for_phase(&name, Some(&value));
}

fn host_remove_header(name: ImmutableString) {
  update_headers_for_phase(&name, None);
}

fn host_deny(status: rhai::INT, body: ImmutableString) {
  with_context(|ctx| {
    let status_code = StatusCode::from_u16(status as u16).unwrap_or(StatusCode::FORBIDDEN);
    let bytes = body.as_str().as_bytes().to_vec();
    ctx.decision = ScriptDecision::Deny {
      status: status_code,
      body: bytes.clone(),
    };
    ctx.response.set_status(status_code.as_u16());
    ctx.response.set_body(bytes);
  });
}

fn host_spawn_task(name: ImmutableString, callback: FnPtr) -> Result<(), Dynamic> {
  with_context(|ctx| {
    if !ctx.allow_spawn_task {
      return Err(Dynamic::from("spawn_task not allowed"));
    }
    let spawner = ctx
      .spawn_callback
      .as_ref()
      .ok_or_else(|| Dynamic::from("task spawner unavailable"))?;
    spawner(name.to_string(), callback);
    Ok(())
  })
  .unwrap_or_else(|| Err(Dynamic::from("no active script context")))
}

/// Applies request updates back into the Hyper request parts.
pub fn apply_request_snapshot(snapshot: ScriptRequestSnapshot, parts: &mut hyper::http::request::Parts) -> Result<()> {
  parts.method = snapshot.method.parse().context("invalid HTTP method")?;
  parts.uri = snapshot.uri.parse().context("invalid URI")?;
  parts.headers = headers_vec_to_map(&snapshot.headers);
  Ok(())
}

/// Indicates whether the request body was modified by the script.
pub fn request_body_modified(snapshot: &ScriptRequestSnapshot) -> bool {
  snapshot.body_modified
}

/// Applies response snapshot changes onto response parts.
pub fn apply_response_snapshot(
  snapshot: &ScriptResponseSnapshot,
  parts: &mut hyper::http::response::Parts,
) -> Result<()> {
  if let Some(status) = snapshot.status {
    parts.status = StatusCode::from_u16(status)?;
  }
  if !snapshot.headers.is_empty() {
    parts.headers = headers_vec_to_map(&snapshot.headers);
  }
  Ok(())
}
