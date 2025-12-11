use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::env;
use std::error::Error;
use std::fs;
use std::panic::{self, AssertUnwindSafe};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime};

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use bytes::Bytes;
use ferron_common::config::ServerConfiguration;
use ferron_common::get_entries_for_validation;
use ferron_common::logging::ErrorLogger;
use ferron_common::modules::{Module, ModuleHandlers, ModuleLoader, ResponseData, SocketData};
use ferron_common::util::ModuleCache;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty, Full};
use hyper::{HeaderMap, Request, Response, StatusCode};
use parking_lot::Mutex;
use rhai::{FnPtr, Scope};
use sha2::{Digest, Sha256};
use tokio::runtime::Handle;
use tokio::sync::RwLock;
use tokio::task::{block_in_place, JoinHandle};
use tokio::time::{interval, MissedTickBehavior};

use crate::config::{FailurePolicy, ScriptDefinition, ScriptModuleConfig, ScriptSource, ScriptTrigger};
use crate::context::{
  apply_request_snapshot, apply_response_snapshot, env_to_dynamic, request_body_modified, ContextGuard, ScriptDecision,
  ScriptExecutionContext, ScriptPhase, ScriptRequestHandle, ScriptResponseHandle, ScriptStateHandle,
};

#[derive(Clone, Copy)]
struct SendableContextPtr(*mut ScriptExecutionContext<'static>);

impl SendableContextPtr {
  #[allow(clippy::unnecessary_cast)]
  fn new(ctx: &mut ScriptExecutionContext<'_>) -> Self {
    Self(ctx as *mut _ as *mut ScriptExecutionContext<'static>)
  }

  fn as_ptr(self) -> *mut ScriptExecutionContext<'static> {
    self.0
  }
}

unsafe impl Send for SendableContextPtr {}
use crate::engine::ScriptEngine;

const FAILURE_THRESHOLD: usize = 5;

fn script_debug_enabled() -> bool {
  static ENABLED: OnceLock<bool> = OnceLock::new();
  *ENABLED.get_or_init(|| env::var("FERRON_SCRIPT_DEBUG").is_ok())
}

macro_rules! script_debug {
  ($($arg:tt)*) => {
    if $crate::runtime::script_debug_enabled() {
      eprintln!($($arg)*);
    }
  };
}

pub struct ScriptExecModuleLoader {
  cache: ModuleCache<ScriptExecModule>,
}

impl Default for ScriptExecModuleLoader {
  fn default() -> Self {
    Self::new()
  }
}

impl ScriptExecModuleLoader {
  pub fn new() -> Self {
    Self {
      cache: ModuleCache::new(vec!["module"]),
    }
  }
}

impl ModuleLoader for ScriptExecModuleLoader {
  fn load_module(
    &mut self,
    config: &ServerConfiguration,
    _global_config: Option<&ServerConfiguration>,
    secondary_runtime: &tokio::runtime::Runtime,
  ) -> Result<Arc<dyn Module + Send + Sync>, Box<dyn std::error::Error + Send + Sync>> {
    Ok(
      self
        .cache
        .get_or_init::<_, Box<dyn std::error::Error + Send + Sync>>(config, |cfg| {
          let module_config = ScriptModuleConfig::from_server_config(cfg)?;
          let runtime = ScriptRuntime::new(module_config, secondary_runtime.handle());
          Ok(Arc::new(ScriptExecModule { runtime }))
        })?,
    )
  }

  fn get_requirements(&self) -> Vec<&'static str> {
    vec!["module"]
  }

  fn validate_configuration(
    &self,
    config: &ServerConfiguration,
    used_properties: &mut HashSet<String>,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    if get_entries_for_validation!("module", config, used_properties).is_some() {
      ScriptModuleConfig::from_server_config(config)?;
    }
    Ok(())
  }
}

struct ScriptExecModule {
  runtime: Arc<ScriptRuntime>,
}

impl Module for ScriptExecModule {
  fn get_module_handlers(&self) -> Box<dyn ModuleHandlers> {
    Box::new(ScriptModuleHandlers::new(self.runtime.clone()))
  }
}

struct ScriptRuntime {
  engine: Arc<ScriptEngine>,
  state: ScriptStateHandle,
  scripts: Vec<Arc<ManagedScript>>,
  request_start: Vec<Arc<ManagedScript>>,
  request_body: Vec<Arc<ManagedScript>>,
  response_ready: Vec<Arc<ManagedScript>>,
  requires_body: bool,
  tokio_handle: Handle,
  background_tasks: Mutex<Vec<JoinHandle<()>>>,
}

impl ScriptRuntime {
  fn new(config: ScriptModuleConfig, handle: &Handle) -> Arc<Self> {
    let engine = Arc::new(ScriptEngine::new());
    let state = ScriptStateHandle::new();
    let mut scripts = Vec::new();
    let mut request_start = Vec::new();
    let mut request_body = Vec::new();
    let mut response_ready = Vec::new();

    for definition in config.scripts {
      let script = Arc::new(ManagedScript::new(definition));
      script_debug!(
        "DEBUG: Loading script '{}' with {} triggers",
        script.id,
        script.triggers.len()
      );
      if script.has_trigger(|t| matches!(t, ScriptTrigger::RequestStart)) {
        script_debug!("DEBUG: Script '{}' added to request_start", script.id);
        request_start.push(script.clone());
      }
      if script.has_trigger(|t| matches!(t, ScriptTrigger::RequestBody)) {
        request_body.push(script.clone());
      }
      if script.has_trigger(|t| matches!(t, ScriptTrigger::ResponseReady)) {
        response_ready.push(script.clone());
      }
      scripts.push(script);
    }
    script_debug!(
      "DEBUG: ScriptRuntime initialized with {} request_start scripts",
      request_start.len()
    );

    let requires_body = !request_body.is_empty();
    let runtime = Arc::new(ScriptRuntime {
      engine,
      state,
      scripts,
      request_start,
      request_body,
      response_ready,
      requires_body,
      tokio_handle: handle.clone(),
      background_tasks: Mutex::new(Vec::new()),
    });

    runtime.spawn_tick_tasks();
    runtime
  }

  fn spawn_tick_tasks(self: &Arc<Self>) {
    for script in &self.scripts {
      for trigger in &script.triggers {
        if let ScriptTrigger::Tick(interval) = trigger {
          self.spawn_tick_worker(script.clone(), *interval);
        }
      }
    }
  }

  fn spawn_tick_worker(self: &Arc<Self>, script: Arc<ManagedScript>, interval_duration: Duration) {
    let runtime = self.clone();
    let mut ticker = interval(interval_duration);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

    let handle = self.tokio_handle.spawn(async move {
      let logger = ErrorLogger::without_logger();
      let response_handle = ScriptResponseHandle::new();
      let mut ticker = ticker;
      loop {
        ticker.tick().await;
        if let Err(err) = runtime
          .execute_script(script.clone(), ScriptPhase::Tick, None, &response_handle, &logger)
          .await
        {
          logger.log(&format!("Tick script failed: {err}")).await;
        }
      }
    });

    self.background_tasks.lock().push(handle);
  }

  fn is_empty(&self) -> bool {
    self.scripts.is_empty()
  }

  fn requires_request_body(&self) -> bool {
    self.requires_body
  }

  fn request_start_scripts(&self) -> &[Arc<ManagedScript>] {
    &self.request_start
  }

  fn request_body_scripts(&self) -> &[Arc<ManagedScript>] {
    &self.request_body
  }

  fn response_ready_scripts(&self) -> &[Arc<ManagedScript>] {
    &self.response_ready
  }

  async fn run_scripts(
    self: &Arc<Self>,
    scripts: &[Arc<ManagedScript>],
    phase: ScriptPhase,
    request_handle: Option<ScriptRequestHandle>,
    response_handle: &ScriptResponseHandle,
    logger: &ErrorLogger,
  ) -> Result<Option<Response<BoxBody<Bytes, std::io::Error>>>> {
    if scripts.is_empty() {
      script_debug!("DEBUG: run_scripts called with empty scripts array");
      return Ok(None);
    }

    script_debug!("DEBUG: run_scripts executing {} scripts", scripts.len());
    for script in scripts {
      script_debug!("DEBUG: Executing script: {}", script.id);
      match self
        .execute_script(script.clone(), phase, request_handle.clone(), response_handle, logger)
        .await
      {
        Ok(ScriptDecision::Continue) => {}
        Ok(ScriptDecision::Deny { status, body }) => {
          return Ok(Some(deny_response(status, body)));
        }
        Err(err) => Err(err)?,
      }
    }

    Ok(None)
  }

  async fn execute_script(
    self: &Arc<Self>,
    script: Arc<ManagedScript>,
    phase: ScriptPhase,
    request_handle: Option<ScriptRequestHandle>,
    response_handle: &ScriptResponseHandle,
    logger: &ErrorLogger,
  ) -> Result<ScriptDecision> {
    script_debug!(
      "DEBUG: execute_script called for script '{}' in phase {:?}",
      script.id,
      phase
    );
    if script.failure_state.is_tripped() {
      script_debug!("DEBUG: Script '{}' is tripped, skipping", script.id);
      logger
        .log(&format!(
          "Script '{}' skipped because it is temporarily disabled",
          script.id
        ))
        .await;
      return Ok(ScriptDecision::Continue);
    }

    let (ast, version_hash) = match script.ast_cache.ensure_ast(&self.engine, &self.tokio_handle).await {
      Ok(data) => data,
      Err(err) => {
        // Treat compilation errors the same way as runtime failures so `failure_policy`
        // is applied and we log the root cause.
        return self
          .handle_failure(&script, &None, logger, &format!("compile error: {err:#}"))
          .await;
      }
    };

    let mut scope = Scope::new();
    if let Some(request) = &request_handle {
      scope.push_constant("request", request.clone());
    }
    scope.push_constant("response", response_handle.clone());
    scope.push_constant("env", env_to_dynamic(&script.env));
    scope.push("state", self.state.clone());

    let logger_clone = logger.clone();
    let spawn_callback: Option<Arc<dyn Fn(String, FnPtr) + Send + Sync + 'static>> =
      if script.permissions.allow_spawn_task {
        let runtime = Arc::downgrade(self);
        let script_for_task = script.clone();
        Some(Arc::new(move |task_name: String, fn_ptr: FnPtr| {
          if let Some(runtime) = runtime.upgrade() {
            runtime.queue_spawn_task(script_for_task.clone(), task_name, fn_ptr, logger_clone.clone());
          }
        }) as Arc<dyn Fn(String, FnPtr) + Send + Sync + 'static>)
      } else {
        None
      };

    let mut execution_context = ScriptExecutionContext::new(
      &script.id,
      phase,
      request_handle,
      response_handle.clone(),
      script.env.clone(),
      self.state.clone(),
      logger,
      script.permissions.allow_spawn_task,
      spawn_callback,
    );

    let mut engine = self.engine.instantiate();
    let max_exec_time = script.limits.max_exec_time;
    engine.set_max_operations(script.limits.max_operations);
    engine.set_max_call_levels(script.limits.max_call_depth as usize);

    let ast_clone = ast.clone();
    let scope = scope;
    let ctx_ptr = SendableContextPtr::new(&mut execution_context);
    let timeout_handle = self.tokio_handle.clone();
    let result = timeout_handle
      .spawn(async move {
        tokio::time::timeout(max_exec_time, async move {
          block_in_place(move || {
            let mut scope = scope;
            let engine = engine;
            panic::catch_unwind(AssertUnwindSafe(|| {
              let guard = ContextGuard::activate(ctx_ptr.as_ptr());
              let run_result = engine.run_ast_with_scope(&mut scope, &ast_clone);
              drop(guard);
              run_result
            }))
          })
        })
        .await
      })
      .await;

    // Drain logs before handling result to ensure they are written even if script fails
    let pending_logs = execution_context.drain_logs();

    match result {
      Ok(Ok(Ok(Ok(_)))) => {
        script.failure_state.record_success();
        for log_line in pending_logs {
          logger.log(&log_line).await;
        }
        script_debug!(
          "DEBUG: Script '{}' executed successfully, decision: {:?}",
          script.id,
          execution_context.decision
        );
        Ok(execution_context.decision)
      }
      Ok(Ok(Ok(Err(err)))) => {
        for log_line in pending_logs {
          logger.log(&log_line).await;
        }
        self
          .handle_failure(&script, &version_hash, logger, &format!("execution error: {err}"))
          .await
      }
      Ok(Ok(Err(panic_payload))) => {
        for log_line in pending_logs {
          logger.log(&log_line).await;
        }
        let panic_msg = describe_panic(panic_payload);
        self
          .handle_failure(&script, &version_hash, logger, &format!("panic: {panic_msg}"))
          .await
      }
      Ok(Err(elapsed)) => {
        for log_line in pending_logs {
          logger.log(&log_line).await;
        }
        self
          .handle_failure(
            &script,
            &version_hash,
            logger,
            &format!("timed out after {:?}", elapsed),
          )
          .await
      }
      Err(join_err) => {
        for log_line in pending_logs {
          logger.log(&log_line).await;
        }
        self
          .handle_failure(&script, &version_hash, logger, &format!("tokio join error: {join_err}"))
          .await
      }
    }
  }

  async fn handle_failure(
    self: &Arc<Self>,
    script: &Arc<ManagedScript>,
    version_hash: &Option<String>,
    logger: &ErrorLogger,
    reason: &str,
  ) -> Result<ScriptDecision> {
    let hash_text = version_hash.as_deref().map(|h| format!(" @{}", h)).unwrap_or_default();
    logger
      .log(&format!("Script '{}'{hash_text} failed: {reason}", script.id))
      .await;

    let tripped = script.failure_state.record_failure();
    if tripped {
      logger
        .log(&format!(
          "Script '{}' has been disabled after {} consecutive failures",
          script.id, FAILURE_THRESHOLD
        ))
        .await;
    }

    match script.failure_policy {
      FailurePolicy::Block => Err(anyhow!("script '{}' failed", script.id)),
      FailurePolicy::Skip => Ok(ScriptDecision::Continue),
    }
  }

  fn queue_spawn_task(
    self: &Arc<Self>,
    script: Arc<ManagedScript>,
    task_name: String,
    fn_ptr: FnPtr,
    logger: ErrorLogger,
  ) {
    self.cleanup_finished_tasks();
    let runtime = self.clone();
    let handle = self.tokio_handle.spawn(async move {
      runtime.run_spawned_function(script, task_name, fn_ptr, logger).await;
    });
    self.background_tasks.lock().push(handle);
  }

  fn cleanup_finished_tasks(&self) {
    let mut handles = self.background_tasks.lock();
    handles.retain(|handle| !handle.is_finished());
  }

  async fn run_spawned_function(
    self: Arc<Self>,
    script: Arc<ManagedScript>,
    task_name: String,
    fn_ptr: FnPtr,
    logger: ErrorLogger,
  ) {
    let (ast, version_hash) = match script.ast_cache.ensure_ast(&self.engine, &self.tokio_handle).await {
      Ok(data) => data,
      Err(err) => {
        logger
          .log(&format!("Failed to compile background task '{}': {err}", task_name))
          .await;
        return;
      }
    };

    let mut scope = Scope::new();
    scope.push_constant("env", env_to_dynamic(&script.env));
    scope.push("state", self.state.clone());

    let mut engine = self.engine.instantiate();
    let max_exec_time = script.limits.max_exec_time;
    engine.set_max_operations(script.limits.max_operations);
    engine.set_max_call_levels(script.limits.max_call_depth as usize);

    let spawn_logger = logger.clone();
    let runtime = Arc::downgrade(&self);
    let script_for_task = script.clone();
    let callback: Option<Arc<dyn Fn(String, FnPtr) + Send + Sync + 'static>> = if script.permissions.allow_spawn_task {
      Some(Arc::new(move |name: String, pointer: FnPtr| {
        if let Some(runtime) = runtime.upgrade() {
          runtime.queue_spawn_task(script_for_task.clone(), name, pointer, spawn_logger.clone());
        }
      }) as Arc<dyn Fn(String, FnPtr) + Send + Sync + 'static>)
    } else {
      None
    };

    let mut exec_ctx = ScriptExecutionContext::new(
      &script.id,
      ScriptPhase::BackgroundTask,
      None,
      ScriptResponseHandle::new(),
      script.env.clone(),
      self.state.clone(),
      &logger,
      script.permissions.allow_spawn_task,
      callback,
    );

    let args: Vec<rhai::Dynamic> = fn_ptr.iter_curry().cloned().collect();
    let fn_name = fn_ptr.fn_name().to_string();
    let ast_clone = ast.clone();
    let scope = scope;
    let ctx_ptr = SendableContextPtr::new(&mut exec_ctx);
    let timeout_handle = self.tokio_handle.clone();
    let result = timeout_handle
      .spawn(async move {
        tokio::time::timeout(max_exec_time, async move {
          block_in_place(move || {
            let mut scope = scope;
            let engine = engine;
            let mut args = args;
            #[allow(deprecated)]
            panic::catch_unwind(AssertUnwindSafe(|| {
              let guard = ContextGuard::activate(ctx_ptr.as_ptr());
              let call_result = engine.call_fn_raw(
                &mut scope,
                &ast_clone,
                false,
                true,
                fn_name.as_str(),
                None,
                args.as_mut_slice(),
              );
              drop(guard);
              call_result
            }))
          })
        })
        .await
      })
      .await;

    match result {
      Ok(Ok(Ok(Ok(_)))) => {
        script.failure_state.record_success();
        for log_line in exec_ctx.drain_logs() {
          logger.log(&log_line).await;
        }
      }
      Ok(Ok(Ok(Err(err)))) => {
        for log_line in exec_ctx.drain_logs() {
          logger.log(&log_line).await;
        }
        self
          .handle_failure(&script, &version_hash, &logger, &format!("task error: {err}"))
          .await
          .ok();
      }
      Ok(Ok(Err(panic_payload))) => {
        for log_line in exec_ctx.drain_logs() {
          logger.log(&log_line).await;
        }
        let panic_msg = describe_panic(panic_payload);
        self
          .handle_failure(&script, &version_hash, &logger, &format!("task panic: {panic_msg}"))
          .await
          .ok();
      }
      Ok(Err(elapsed)) => {
        for log_line in exec_ctx.drain_logs() {
          logger.log(&log_line).await;
        }
        self
          .handle_failure(
            &script,
            &version_hash,
            &logger,
            &format!("task timed out after {:?}", elapsed),
          )
          .await
          .ok();
      }
      Err(join_err) => {
        for log_line in exec_ctx.drain_logs() {
          logger.log(&log_line).await;
        }
        self
          .handle_failure(
            &script,
            &version_hash,
            &logger,
            &format!("task tokio join error: {join_err}"),
          )
          .await
          .ok();
      }
    }
  }
}

impl Drop for ScriptRuntime {
  fn drop(&mut self) {
    for handle in self.background_tasks.lock().drain(..) {
      handle.abort();
    }
  }
}

#[derive(Clone)]
struct ManagedScript {
  id: String,
  env: Arc<HashMap<String, String>>,
  permissions: crate::config::ScriptPermissions,
  limits: crate::config::ScriptLimits,
  failure_policy: FailurePolicy,
  triggers: Vec<ScriptTrigger>,
  ast_cache: ScriptAstCache,
  failure_state: ScriptFailureState,
}

impl ManagedScript {
  fn new(definition: ScriptDefinition) -> Self {
    Self {
      id: definition.id.clone(),
      env: Arc::new(definition.env.clone()),
      permissions: definition.permissions.clone(),
      limits: definition.limits.clone(),
      failure_policy: definition.failure_policy,
      triggers: definition.triggers,
      ast_cache: ScriptAstCache::new(definition.source, definition.reload_on_change),
      failure_state: ScriptFailureState::new(),
    }
  }

  fn has_trigger<F>(&self, predicate: F) -> bool
  where
    F: Fn(&ScriptTrigger) -> bool,
  {
    self.triggers.iter().any(predicate)
  }
}

#[derive(Clone)]
struct ScriptFailureState {
  consecutive: Arc<AtomicUsize>,
  tripped: Arc<AtomicBool>,
}

impl ScriptFailureState {
  fn new() -> Self {
    Self {
      consecutive: Arc::new(AtomicUsize::new(0)),
      tripped: Arc::new(AtomicBool::new(false)),
    }
  }

  fn record_success(&self) {
    self.consecutive.store(0, Ordering::Relaxed);
    self.tripped.store(false, Ordering::Relaxed);
  }

  fn record_failure(&self) -> bool {
    let failures = self.consecutive.fetch_add(1, Ordering::Relaxed) + 1;
    if failures >= FAILURE_THRESHOLD {
      self.tripped.store(true, Ordering::Relaxed);
      true
    } else {
      false
    }
  }

  fn is_tripped(&self) -> bool {
    self.tripped.load(Ordering::Relaxed)
  }
}

#[derive(Clone)]
struct ScriptAstCache {
  source: ScriptSource,
  reload_on_change: bool,
  compiled: Arc<RwLock<Option<CompiledAst>>>,
}

#[derive(Clone)]
struct CompiledAst {
  ast: Arc<rhai::AST>,
  version_hash: String,
  modified: Option<SystemTime>,
}

impl ScriptAstCache {
  fn new(source: ScriptSource, reload_on_change: bool) -> Self {
    Self {
      source,
      reload_on_change,
      compiled: Arc::new(RwLock::new(None)),
    }
  }

  async fn ensure_ast(&self, engine: &ScriptEngine, handle: &Handle) -> Result<(Arc<rhai::AST>, Option<String>)> {
    loop {
      let cached = { self.compiled.read().await.clone() };
      let cached_version = cached.as_ref().map(|c| c.version_hash.clone());
      let should_reload = self.should_reload(&cached, handle).await;

      if !should_reload {
        if let Some(compiled) = cached {
          return Ok((compiled.ast.clone(), Some(compiled.version_hash.clone())));
        }
      }

      let mut guard = self.compiled.write().await;
      if guard.as_ref().map(|c| c.version_hash.as_str()) != cached_version.as_deref() {
        continue;
      }

      let code = match &self.source {
        ScriptSource::File(path) => read_to_string(handle, path).await?,
      };

      let ast = Arc::new(compile_script(engine, &code)?);
      let version_hash = compute_hash(&code);
      let modified = match &self.source {
        ScriptSource::File(path) => read_metadata(handle, path).await.ok().and_then(|m| m.modified().ok()),
      };
      let compiled = CompiledAst {
        ast: ast.clone(),
        version_hash,
        modified,
      };
      let version_hash = Some(compiled.version_hash.clone());
      *guard = Some(compiled);
      return Ok((ast, version_hash));
    }
  }

  async fn should_reload(&self, cached: &Option<CompiledAst>, handle: &Handle) -> bool {
    match (&self.source, self.reload_on_change, cached) {
      (_, _, None) => true,
      (ScriptSource::File(path), true, Some(compiled)) => {
        let metadata = read_metadata(handle, path).await.ok();
        metadata
          .and_then(|m| m.modified().ok())
          .map(|mtime| compiled.modified.is_none_or(|prev| mtime > prev))
          .unwrap_or(false)
      }
      _ => false,
    }
  }
}

fn compute_hash(code: &str) -> String {
  let mut hasher = Sha256::new();
  hasher.update(code.as_bytes());
  hex::encode(hasher.finalize())
}

fn compile_script(engine: &ScriptEngine, code: &str) -> Result<rhai::AST> {
  let engine = engine.instantiate();
  let scope = initial_compile_scope();
  engine
    .compile_with_scope(&scope, code)
    .with_context(|| "failed to compile script".to_string())
}

fn describe_panic(panic: Box<dyn Any + Send>) -> String {
  match panic.downcast::<String>() {
    Ok(message) => *message,
    Err(panic) => match panic.downcast::<&'static str>() {
      Ok(message) => message.to_string(),
      Err(_) => "unknown panic".to_string(),
    },
  }
}

fn initial_compile_scope() -> Scope<'static> {
  let mut scope = Scope::new();
  let headers = HeaderMap::new();
  scope.push_constant(
    "request",
    ScriptRequestHandle::from_parts("GET", "/", &headers, Vec::new()),
  );
  scope.push_constant("response", ScriptResponseHandle::new());
  scope.push_constant("env", env_to_dynamic(&HashMap::new()));
  scope.push("state", ScriptStateHandle::new());
  scope
}

async fn read_to_string(handle: &Handle, path: &Path) -> Result<String> {
  let owned = path.to_path_buf();
  let display = owned.display().to_string();
  handle
    .spawn_blocking(move || fs::read_to_string(&owned))
    .await
    .context("failed to join file read task")?
    .with_context(|| format!("unable to read script at {display}"))
}

async fn read_metadata(handle: &Handle, path: &Path) -> Result<fs::Metadata> {
  let owned = path.to_path_buf();
  let display = owned.display().to_string();
  handle
    .spawn_blocking(move || fs::metadata(&owned))
    .await
    .context("failed to join metadata read task")?
    .with_context(|| format!("unable to read metadata for {display}"))
}

enum RequestBodyState {
  Buffered(Vec<u8>),
  Stream(BoxBody<Bytes, std::io::Error>),
}

fn deny_response(status: StatusCode, body: Vec<u8>) -> Response<BoxBody<Bytes, std::io::Error>> {
  Response::builder()
    .status(status)
    .body(
      Full::new(Bytes::from(body))
        .map_err(|e: std::convert::Infallible| -> std::io::Error { match e {} })
        .boxed(),
    )
    .unwrap_or_else(|_| {
      Response::new(
        Empty::new()
          .map_err(|e: std::convert::Infallible| -> std::io::Error { match e {} })
          .boxed(),
      )
    })
}

#[derive(Debug)]
struct ResponseHandlerError(anyhow::Error);

impl std::fmt::Display for ResponseHandlerError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "{}", self.0)
  }
}

impl std::error::Error for ResponseHandlerError {}

pub struct ScriptModuleHandlers {
  runtime: Arc<ScriptRuntime>,
  request_handle: Option<ScriptRequestHandle>,
  response_handle: ScriptResponseHandle,
  request_parts: Option<hyper::http::request::Parts>,
  request_body: Option<RequestBodyState>,
  error_logger: Option<ErrorLogger>,
}

impl ScriptModuleHandlers {
  fn new(runtime: Arc<ScriptRuntime>) -> Self {
    Self {
      runtime,
      request_handle: None,
      response_handle: ScriptResponseHandle::new(),
      request_parts: None,
      request_body: None,
      error_logger: None,
    }
  }
}

#[async_trait(?Send)]
impl ModuleHandlers for ScriptModuleHandlers {
  async fn request_handler(
    &mut self,
    request: Request<BoxBody<Bytes, std::io::Error>>,
    _config: &ServerConfiguration,
    _socket_data: &SocketData,
    error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn std::error::Error + Send + Sync>> {
    if self.runtime.is_empty() {
      return Ok(ResponseData {
        request: Some(request),
        response: None,
        response_status: None,
        response_headers: None,
        new_remote_address: None,
      });
    }

    self.request_handle = None;
    self.response_handle = ScriptResponseHandle::new();
    self.request_parts = None;
    self.request_body = None;
    self.error_logger = Some(error_logger.clone());

    let (parts, body) = request.into_parts();
    self.request_parts = Some(parts);
    self.request_body = Some(if self.runtime.requires_request_body() {
      let collected = body.collect().await?;
      RequestBodyState::Buffered(collected.to_bytes().to_vec())
    } else {
      RequestBodyState::Stream(body)
    });

    if let Some(parts_ref) = self.request_parts.as_ref() {
      let body_clone = match self.request_body.as_ref().unwrap() {
        RequestBodyState::Buffered(bytes) => bytes.clone(),
        RequestBodyState::Stream(_) => Vec::new(),
      };
      let uri = parts_ref.uri.to_string();
      let handle =
        ScriptRequestHandle::from_parts(parts_ref.method.as_str(), uri.as_str(), &parts_ref.headers, body_clone);
      self.request_handle = Some(handle);
    }

    let request_start_scripts = self.runtime.request_start_scripts();
    script_debug!("DEBUG: request_start_scripts count: {}", request_start_scripts.len());
    if let Some(response) = self
      .runtime
      .run_scripts(
        request_start_scripts,
        ScriptPhase::RequestStart,
        self.request_handle.clone(),
        &self.response_handle,
        error_logger,
      )
      .await
      .map_err(|err| -> Box<dyn std::error::Error + Send + Sync> { err.into() })?
    {
      script_debug!("DEBUG: Script returned deny response");
      return Ok(ResponseData {
        request: None,
        response: Some(response),
        response_status: None,
        response_headers: None,
        new_remote_address: None,
      });
    }
    script_debug!("DEBUG: No deny response from scripts, continuing");

    if self.runtime.requires_request_body() {
      if let Some(response) = self
        .runtime
        .run_scripts(
          self.runtime.request_body_scripts(),
          ScriptPhase::RequestBody,
          self.request_handle.clone(),
          &self.response_handle,
          error_logger,
        )
        .await
        .map_err(|err| -> Box<dyn std::error::Error + Send + Sync> { err.into() })?
      {
        return Ok(ResponseData {
          request: None,
          response: Some(response),
          response_status: None,
          response_headers: None,
          new_remote_address: None,
        });
      }
    }

    if let (Some(handle), Some(parts)) = (&self.request_handle, &mut self.request_parts) {
      let snapshot = handle.snapshot();
      apply_request_snapshot(snapshot.clone(), parts)?;
      if request_body_modified(&snapshot) {
        let new_body = snapshot.body.clone();
        match self.request_body.as_mut() {
          Some(RequestBodyState::Buffered(buffer)) => {
            *buffer = new_body;
          }
          Some(RequestBodyState::Stream(_)) | None => {
            self.request_body = Some(RequestBodyState::Buffered(new_body));
          }
        }
      }
    }

    let parts = self.request_parts.take().unwrap();
    let body = match self.request_body.take().unwrap() {
      RequestBodyState::Buffered(bytes) => Full::new(Bytes::from(bytes)).map_err(|e| match e {}).boxed(),
      RequestBodyState::Stream(body) => body,
    };

    Ok(ResponseData {
      request: Some(Request::from_parts(parts, body)),
      response: None,
      response_status: None,
      response_headers: None,
      new_remote_address: None,
    })
  }

  async fn response_modifying_handler(
    &mut self,
    response: Response<BoxBody<Bytes, std::io::Error>>,
  ) -> Result<Response<BoxBody<Bytes, std::io::Error>>, Box<dyn std::error::Error>> {
    if self.runtime.response_ready_scripts().is_empty() {
      return Ok(response);
    }

    let (mut parts, body) = response.into_parts();
    let collected = body.collect().await?;
    let bytes = collected.to_bytes();
    self.response_handle.set_status(parts.status.as_u16());
    self.response_handle.set_body(bytes.to_vec());
    for (name, value) in parts.headers.iter() {
      if let Ok(value_str) = value.to_str() {
        self.response_handle.set_header(name.as_str(), value_str);
      }
    }

    let logger = match &self.error_logger {
      Some(logger) => {
        // Clone the logger to ensure it's available for script execution
        logger.clone()
      }
      None => {
        // If error_logger is None, create a dummy logger that does nothing
        // This shouldn't happen, but we handle it gracefully
        eprintln!("WARNING: error_logger is None in response_modifying_handler");
        ErrorLogger::without_logger()
      }
    };

    if let Some(response) = self
      .runtime
      .run_scripts(
        self.runtime.response_ready_scripts(),
        ScriptPhase::Response,
        self.request_handle.clone(),
        &self.response_handle,
        &logger,
      )
      .await
      .map_err(|err| -> Box<dyn std::error::Error> { Box::new(ResponseHandlerError(err)) })?
    {
      self.request_handle = None;
      self.error_logger = None;
      return Ok(response);
    }

    let mut snapshot = self.response_handle.snapshot();
    let body_len = snapshot.body.len();
    // 确保 Content-Length 与修改后的响应体一致，避免超/少报导致 hyper panic。
    snapshot
      .headers
      .retain(|(name, _)| !name.eq_ignore_ascii_case("content-length"));
    snapshot
      .headers
      .push(("content-length".to_string(), body_len.to_string()));

    apply_response_snapshot(&snapshot, &mut parts)?;
    let body = Full::new(Bytes::from(snapshot.body)).map_err(|e| match e {}).boxed();

    self.request_handle = None;
    self.error_logger = None;

    Ok(Response::from_parts(parts, body))
  }
}
