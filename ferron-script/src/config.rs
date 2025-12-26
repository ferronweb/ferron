use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{anyhow, Result};
use ferron_common::config::{ServerConfiguration, ServerConfigurationEntries, ServerConfigurationEntry};
use ferron_common::get_entries;
use humantime::parse_duration;

const DEFAULT_MAX_OPERATIONS: u64 = 200_000;
const DEFAULT_MAX_CALL_DEPTH: u64 = 32;
const DEFAULT_MAX_EXEC_TIME: Duration = Duration::from_millis(50);
const DEFAULT_TICK_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone)]
pub struct ScriptModuleConfig {
  pub scripts: Vec<ScriptDefinition>,
}

impl ScriptModuleConfig {
  pub fn from_server_config(config: &ServerConfiguration) -> Result<Self> {
    let mut scripts = Vec::new();
    if let Some(modules) = get_entries!("module", config) {
      for module_entry in &modules.inner {
        let Some(name) = module_entry.values.first().and_then(|v| v.as_str()) else {
          continue;
        };
        if name != "script-exec" {
          continue;
        }
        if let Some(script_entries) = module_entry.children.get("script") {
          for script_entry in &script_entries.inner {
            scripts.push(parse_script(script_entry)?);
          }
        }
      }
    }
    Ok(Self { scripts })
  }

  #[allow(dead_code)]
  pub fn is_empty(&self) -> bool {
    self.scripts.is_empty()
  }
}

#[derive(Clone, Debug)]
pub struct ScriptDefinition {
  pub id: String,
  pub source: ScriptSource,
  pub triggers: Vec<ScriptTrigger>,
  pub env: HashMap<String, String>,
  pub permissions: ScriptPermissions,
  pub reload_on_change: bool,
  pub limits: ScriptLimits,
  pub failure_policy: FailurePolicy,
}

#[derive(Clone, Debug)]
pub enum ScriptSource {
  File(PathBuf),
}

#[derive(Clone, Debug)]
pub enum ScriptTrigger {
  RequestStart,
  RequestBody,
  ResponseReady,
  Tick(Duration),
}

#[derive(Clone, Debug)]
pub struct ScriptLimits {
  pub max_operations: u64,
  pub max_call_depth: u64,
  pub max_exec_time: Duration,
}

impl Default for ScriptLimits {
  fn default() -> Self {
    Self {
      max_operations: DEFAULT_MAX_OPERATIONS,
      max_call_depth: DEFAULT_MAX_CALL_DEPTH,
      max_exec_time: DEFAULT_MAX_EXEC_TIME,
    }
  }
}

#[derive(Clone, Debug)]
pub struct ScriptPermissions {
  pub allow_spawn_task: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FailurePolicy {
  Block,
  Skip,
}

fn parse_script(entry: &ServerConfigurationEntry) -> Result<ScriptDefinition> {
  let id = entry
    .values
    .first()
    .and_then(|v| v.as_str())
    .ok_or_else(|| anyhow!("script block is missing an identifier"))?;
  let source = parse_source(entry)?;
  let env = parse_env(entry);
  let permissions = parse_permissions(entry);
  let reload_on_change = match parse_flag(entry, "reload_on_change")? {
    Some(value) => value && matches!(source, ScriptSource::File(_)),
    None => matches!(source, ScriptSource::File(_)),
  };
  let limits = parse_limits(entry)?;
  let failure_policy = parse_failure_policy(entry)?;
  let tick_interval = parse_duration_field(entry, "tick_interval")?.unwrap_or(DEFAULT_TICK_INTERVAL);
  let triggers = parse_triggers(entry, tick_interval)?;
  if triggers.is_empty() {
    Err(anyhow!("script '{id}' must specify at least one trigger"))?
  }

  Ok(ScriptDefinition {
    id: id.to_string(),
    source,
    triggers,
    env,
    permissions,
    reload_on_change,
    limits,
    failure_policy,
  })
}

fn parse_source(entry: &ServerConfigurationEntry) -> Result<ScriptSource> {
  let file = entry
    .children
    .get("file")
    .and_then(|entries| entries.inner.first())
    .and_then(|e| e.values.first())
    .and_then(|v| v.as_str());

  match file {
    Some(path) => Ok(ScriptSource::File(PathBuf::from(path))),
    None => Err(anyhow!("script '{}' must specify a source", entry_name(entry))),
  }
}

fn entry_name(entry: &ServerConfigurationEntry) -> String {
  entry
    .values
    .first()
    .and_then(|v| v.as_str())
    .unwrap_or("<unknown>")
    .to_string()
}

fn parse_env(entry: &ServerConfigurationEntry) -> HashMap<String, String> {
  let mut env = HashMap::new();
  if let Some(env_blocks) = entry.children.get("env") {
    for env_block in &env_blocks.inner {
      for (key, values) in &env_block.children {
        for value_entry in &values.inner {
          if let Some(value) = value_entry.values.first().and_then(|v| v.as_str()) {
            env.insert(key.to_string(), value.to_string());
          }
        }
      }
    }
  }
  env
}

fn parse_permissions(entry: &ServerConfigurationEntry) -> ScriptPermissions {
  let mut allow_spawn_task = false;
  if let Some(allows) = entry.children.get("allow") {
    for allow_entry in &allows.inner {
      for value in &allow_entry.values {
        if value.as_str().is_some_and(|v| v.eq_ignore_ascii_case("spawn_task")) {
          allow_spawn_task = true;
        }
      }
    }
  }
  ScriptPermissions { allow_spawn_task }
}

fn parse_flag(entry: &ServerConfigurationEntry, name: &str) -> Result<Option<bool>> {
  Ok(
    entry
      .children
      .get(name)
      .and_then(|entries| entries.inner.first())
      .and_then(|e| e.values.first())
      .and_then(|v| v.as_bool()),
  )
}

fn parse_limits(entry: &ServerConfigurationEntry) -> Result<ScriptLimits> {
  let mut limits = ScriptLimits::default();
  if let Some(limit_blocks) = entry.children.get("limits") {
    for block in &limit_blocks.inner {
      if let Some(max_ops) = extract_integer(block.children.get("max_operations")) {
        limits.max_operations = validate_positive_i128(max_ops, "max_operations", entry)?;
      }
      if let Some(call_depth) = extract_integer(block.children.get("max_call_depth")) {
        limits.max_call_depth = validate_positive_i128(call_depth, "max_call_depth", entry)?;
      }
      if let Some(duration) = extract_duration(block.children.get("max_exec_time"))? {
        ensure_positive_duration(duration, "max_exec_time", entry)?;
        limits.max_exec_time = duration;
      }
    }
  }
  Ok(limits)
}

fn validate_positive_i128(value: i128, field: &str, entry: &ServerConfigurationEntry) -> Result<u64> {
  if value <= 0 {
    Err(anyhow!(
      "script '{}' has invalid {field} (must be > 0)",
      entry_name(entry)
    ))
  } else {
    Ok(value as u64)
  }
}

fn ensure_positive_duration(duration: Duration, field: &str, entry: &ServerConfigurationEntry) -> Result<()> {
  if duration.is_zero() {
    Err(anyhow!(
      "script '{}' has invalid {field} duration (must be > 0)",
      entry_name(entry)
    ))
  } else {
    Ok(())
  }
}

fn extract_integer(entries: Option<&ServerConfigurationEntries>) -> Option<i128> {
  entries
    .and_then(|entries| entries.inner.first())
    .and_then(|entry| entry.values.first())
    .and_then(|v| v.as_i128())
}

fn extract_duration(entries: Option<&ServerConfigurationEntries>) -> Result<Option<Duration>> {
  if let Some(value) = entries
    .and_then(|entries| entries.inner.first())
    .and_then(|entry| entry.values.first())
    .and_then(|v| v.as_str())
  {
    Ok(Some(parse_duration(value)?))
  } else {
    Ok(None)
  }
}

fn parse_failure_policy(entry: &ServerConfigurationEntry) -> Result<FailurePolicy> {
  let value = entry
    .children
    .get("failure_policy")
    .and_then(|entries| entries.inner.first())
    .and_then(|entry| entry.values.first())
    .and_then(|v| v.as_str())
    .unwrap_or("block");
  match value.to_ascii_lowercase().as_str() {
    "block" => Ok(FailurePolicy::Block),
    "skip" => Ok(FailurePolicy::Skip),
    other => Err(anyhow!("unsupported failure_policy '{other}'")),
  }
}

fn parse_duration_field(entry: &ServerConfigurationEntry, name: &str) -> Result<Option<Duration>> {
  let duration = entry
    .children
    .get(name)
    .and_then(|entries| entries.inner.first())
    .and_then(|entry| entry.values.first())
    .and_then(|v| v.as_str())
    .map(parse_duration)
    .transpose()?;
  Ok(duration)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScriptTriggerKind {
  RequestStart,
  RequestBody,
  ResponseReady,
  Tick,
}

impl ScriptTriggerKind {
  fn from_str(name: &str) -> Result<Self> {
    match name {
      "on_request_start" => Ok(Self::RequestStart),
      "on_request_body" => Ok(Self::RequestBody),
      "on_response_ready" => Ok(Self::ResponseReady),
      "on_tick" => Ok(Self::Tick),
      other => Err(anyhow!("unsupported trigger '{other}'")),
    }
  }
}

fn parse_triggers(entry: &ServerConfigurationEntry, tick_interval: Duration) -> Result<Vec<ScriptTrigger>> {
  let mut triggers = Vec::new();
  if let Some(trigger_entries) = entry.children.get("trigger") {
    for trigger_entry in &trigger_entries.inner {
      for value in &trigger_entry.values {
        let Some(name) = value.as_str() else {
          return Err(anyhow!(
            "script '{}' has an invalid trigger value; use `trigger \"on_request_start\"` style entries",
            entry_name(entry)
          ));
        };
        let trigger = match ScriptTriggerKind::from_str(name)? {
          ScriptTriggerKind::RequestStart => ScriptTrigger::RequestStart,
          ScriptTriggerKind::RequestBody => ScriptTrigger::RequestBody,
          ScriptTriggerKind::ResponseReady => ScriptTrigger::ResponseReady,
          ScriptTriggerKind::Tick => ScriptTrigger::Tick(tick_interval),
        };
        triggers.push(trigger);
      }
    }
  }
  Ok(triggers)
}
