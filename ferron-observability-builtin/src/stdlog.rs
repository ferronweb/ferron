use std::{error::Error, sync::Arc, time::Duration};

use async_channel::Sender;
use chrono::{DateTime, Local};
use ferron_common::{
  config::ServerConfiguration,
  get_entries_for_validation, get_value,
  logging::LogMessage,
  observability::{ObservabilityBackend, ObservabilityBackendLoader},
  util::ModuleCache,
};
use tokio::io::{AsyncWriteExt, BufWriter};

/// Log file observability backend loader
pub struct StdioLogObservabilityBackendLoader {
  cache: ModuleCache<StdioLogObservabilityBackend>,
}

impl Default for StdioLogObservabilityBackendLoader {
  fn default() -> Self {
    Self::new()
  }
}

impl StdioLogObservabilityBackendLoader {
  /// Creates a new observability backend loader
  pub fn new() -> Self {
    Self {
      cache: ModuleCache::new(vec!["log_stdout", "log_stderr", "error_log_stdout", "error_log_stderr"]),
    }
  }
}

impl ObservabilityBackendLoader for StdioLogObservabilityBackendLoader {
  fn load_observability_backend(
    &mut self,
    config: &ServerConfiguration,
    _global_config: Option<&ServerConfiguration>,
    secondary_runtime: &tokio::runtime::Runtime,
  ) -> Result<Arc<dyn ObservabilityBackend + Send + Sync>, Box<dyn Error + Send + Sync>> {
    Ok(
      self
        .cache
        .get_or_init::<_, Box<dyn std::error::Error + Send + Sync>>(config, move |config| {
          let cancel_token = tokio_util::sync::CancellationToken::new();
          let cancel_token_clone = cancel_token.clone();
          let is_log_stdout = get_value!("log_stdout", config)
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
          let is_log_stderr = get_value!("log_stderr", config)
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
          let is_error_log_stdout = get_value!("error_log_stdout", config)
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
          let is_error_log_stderr = get_value!("error_log_stderr", config)
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
          let (logging_tx, logging_rx) = async_channel::unbounded::<LogMessage>();
          secondary_runtime.spawn(async move {
            let log_file: Option<Box<dyn tokio::io::AsyncWrite + Send + Sync + Unpin>> = if is_log_stdout {
              Some(Box::new(tokio::io::stdout()))
            } else if is_log_stderr {
              Some(Box::new(tokio::io::stderr()))
            } else {
              None
            };

            let error_log_file: Option<Box<dyn tokio::io::AsyncWrite + Send + Sync + Unpin>> = if is_error_log_stdout {
              Some(Box::new(tokio::io::stdout()))
            } else if is_error_log_stderr {
              Some(Box::new(tokio::io::stderr()))
            } else {
              None
            };

            let log_file_wrapped =
              log_file.map(|file| Arc::new(tokio::sync::Mutex::new(BufWriter::with_capacity(131072, file))));

            let error_log_file_wrapped =
              error_log_file.map(|file| Arc::new(tokio::sync::Mutex::new(BufWriter::with_capacity(131072, file))));

            // The logs are written when the log message is received by the log event loop, and flushed every 100 ms, improving the server performance.
            let log_file_wrapped_cloned_for_sleep = log_file_wrapped.clone();
            let error_log_file_wrapped_cloned_for_sleep = error_log_file_wrapped.clone();
            tokio::task::spawn(async move {
              let mut interval = tokio::time::interval(Duration::from_millis(100));
              loop {
                interval.tick().await;
                if let Some(log_file_wrapped_cloned) = log_file_wrapped_cloned_for_sleep.clone() {
                  let mut locked_file = log_file_wrapped_cloned.lock().await;
                  locked_file.flush().await.unwrap_or_default();
                }
                if let Some(error_log_file_wrapped_cloned) = error_log_file_wrapped_cloned_for_sleep.clone() {
                  let mut locked_file = error_log_file_wrapped_cloned.lock().await;
                  locked_file.flush().await.unwrap_or_default();
                }
              }
            });

            // Logging loop
            while let Ok(message) = tokio::select! {
              message = logging_rx.recv() => message,
              _ = cancel_token.cancelled() => return,
            } {
              let (mut message, is_error) = message.get_message();
              let log_file_wrapped_cloned = if !is_error {
                log_file_wrapped.clone()
              } else {
                error_log_file_wrapped.clone()
              };

              if let Some(log_file_wrapped_cloned) = log_file_wrapped_cloned {
                tokio::task::spawn(async move {
                  let mut locked_file = log_file_wrapped_cloned.lock().await;
                  if is_error {
                    let now: DateTime<Local> = Local::now();
                    let formatted_time = now.format("%Y-%m-%d %H:%M:%S").to_string();
                    message = format!("[{formatted_time}]: {message}");
                  }
                  message.push('\n');
                  if let Err(e) = locked_file.write(message.as_bytes()).await {
                    eprintln!("Failed to write to log file: {e}");
                  }
                });
              }
            }
          });
          Ok(Arc::new(StdioLogObservabilityBackend {
            cancel_token: cancel_token_clone,
            logging_tx,
          }))
        })?,
    )
  }

  fn get_requirements(&self) -> Vec<&'static str> {
    vec!["log_stdout", "log_stderr", "error_log_stdout", "error_log_stderr"]
  }

  fn validate_configuration(
    &self,
    config: &ferron_common::config::ServerConfiguration,
    used_properties: &mut std::collections::HashSet<String>,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if let Some(error_log_entries) = get_entries_for_validation!("log_stdout", config, used_properties) {
      for error_log_entry in &error_log_entries.inner {
        if error_log_entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `log_stdout` configuration property must have exactly one value"
          ))?
        } else if !error_log_entry.values[0].is_bool() {
          Err(anyhow::anyhow!(
            "Invalid standard output-based access log enabling option"
          ))?
        }
      }
    }

    if let Some(error_log_entries) = get_entries_for_validation!("log_stderr", config, used_properties) {
      for error_log_entry in &error_log_entries.inner {
        if error_log_entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `log_stderr` configuration property must have exactly one value"
          ))?
        } else if !error_log_entry.values[0].is_bool() {
          Err(anyhow::anyhow!(
            "Invalid standard error-based access log enabling option"
          ))?
        }
      }
    }

    if let Some(error_log_entries) = get_entries_for_validation!("error_log_stdout", config, used_properties) {
      for error_log_entry in &error_log_entries.inner {
        if error_log_entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `error_log_stdout` configuration property must have exactly one value"
          ))?
        } else if !error_log_entry.values[0].is_bool() {
          Err(anyhow::anyhow!(
            "Invalid standard output-based error log enabling option"
          ))?
        }
      }
    }

    if let Some(error_log_entries) = get_entries_for_validation!("error_log_stderr", config, used_properties) {
      for error_log_entry in &error_log_entries.inner {
        if error_log_entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `error_log_stderr` configuration property must have exactly one value"
          ))?
        } else if !error_log_entry.values[0].is_bool() {
          Err(anyhow::anyhow!(
            "Invalid standard error-based error log enabling option"
          ))?
        }
      }
    }

    Ok(())
  }
}

struct StdioLogObservabilityBackend {
  cancel_token: tokio_util::sync::CancellationToken,
  logging_tx: Sender<LogMessage>,
}

impl ObservabilityBackend for StdioLogObservabilityBackend {
  fn get_log_channel(&self) -> Option<Sender<LogMessage>> {
    Some(self.logging_tx.clone())
  }
}

impl Drop for StdioLogObservabilityBackend {
  fn drop(&mut self) {
    self.cancel_token.cancel();
  }
}
