use std::sync::Arc;

use parking_lot::Mutex;
use rhai::Engine;

use crate::context;

/// Wraps the configured Rhai engine used by the script runtime.
pub struct ScriptEngine {
  pool: Arc<Mutex<Vec<Engine>>>,
}

impl ScriptEngine {
  pub fn new() -> Self {
    Self {
      pool: Arc::new(Mutex::new(Vec::new())),
    }
  }

  fn build_engine() -> Engine {
    let mut engine = Engine::new();
    engine.set_strict_variables(true);
    engine.disable_symbol("eval");
    engine.disable_symbol("import");
    context::register_types(&mut engine);
    engine
  }

  /// Creates a new engine instance for script execution.
  pub fn instantiate(&self) -> EngineHandle {
    let engine = self.pool.lock().pop().unwrap_or_else(Self::build_engine);
    EngineHandle {
      engine: Some(engine),
      pool: Arc::clone(&self.pool),
    }
  }
}

pub struct EngineHandle {
  engine: Option<Engine>,
  pool: Arc<Mutex<Vec<Engine>>>,
}

impl std::ops::Deref for EngineHandle {
  type Target = Engine;

  fn deref(&self) -> &Self::Target {
    self.engine.as_ref().expect("engine available")
  }
}

impl std::ops::DerefMut for EngineHandle {
  fn deref_mut(&mut self) -> &mut Self::Target {
    self.engine.as_mut().expect("engine available")
  }
}

impl Drop for EngineHandle {
  fn drop(&mut self) {
    if let Some(engine) = self.engine.take() {
      let mut pool = self.pool.lock();
      pool.push(engine);
    }
  }
}
