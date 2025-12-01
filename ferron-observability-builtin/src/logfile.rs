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
pub struct LogFileObservabilityBackendLoader {
  cache: ModuleCache<LogFileObservabilityBackend>,
}

impl Default for LogFileObservabilityBackendLoader {
  fn default() -> Self {
    Self::new()
  }
}

impl LogFileObservabilityBackendLoader {
  /// Creates a new observability backend loader
  pub fn new() -> Self {
    Self {
      cache: ModuleCache::new(vec!["log", "error_log"]),
    }
  }
}

impl ObservabilityBackendLoader for LogFileObservabilityBackendLoader {
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
          let log_filename = get_value!("log", config)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
          let error_log_filename = get_value!("error_log", config)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
          let (logging_tx, logging_rx) = async_channel::unbounded::<LogMessage>();
          secondary_runtime.spawn(async move {
            let log_file = match log_filename {
              Some(log_filename) => Some(
                tokio::fs::OpenOptions::new()
                  .append(true)
                  .create(true)
                  .open(log_filename)
                  .await,
              ),
              None => None,
            };

            let error_log_file = match error_log_filename {
              Some(error_log_filename) => Some(
                tokio::fs::OpenOptions::new()
                  .append(true)
                  .create(true)
                  .open(error_log_filename)
                  .await,
              ),
              None => None,
            };

            let log_file_wrapped = match log_file {
              Some(Ok(file)) => Some(Arc::new(tokio::sync::Mutex::new(BufWriter::with_capacity(
                131072, file,
              )))),
              Some(Err(e)) => {
                eprintln!("Failed to open log file: {e}");
                None
              }
              None => None,
            };

            let error_log_file_wrapped = match error_log_file {
              Some(Ok(file)) => Some(Arc::new(tokio::sync::Mutex::new(BufWriter::with_capacity(
                131072, file,
              )))),
              Some(Err(e)) => {
                eprintln!("Failed to open error log file: {e}");
                None
              }
              None => None,
            };

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
          Ok(Arc::new(LogFileObservabilityBackend {
            cancel_token: cancel_token_clone,
            logging_tx,
          }))
        })?,
    )
  }

  fn get_requirements(&self) -> Vec<&'static str> {
    vec!["log", "error_log"]
  }

  fn validate_configuration(
    &self,
    config: &ferron_common::config::ServerConfiguration,
    used_properties: &mut std::collections::HashSet<String>,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if let Some(error_log_entries) = get_entries_for_validation!("error_log", config, used_properties) {
      for error_log_entry in &error_log_entries.inner {
        if error_log_entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `error_log` configuration property must have exactly one value"
          ))?
        } else if !error_log_entry.values[0].is_string() {
          Err(anyhow::anyhow!("The path to the error log must be a string"))?
        }
      }
    };

    if let Some(log_entries) = get_entries_for_validation!("log", config, used_properties) {
      for log_entry in &log_entries.inner {
        if log_entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `log` configuration property must have exactly one value"
          ))?
        } else if !log_entry.values[0].is_string() {
          Err(anyhow::anyhow!("The path to the access log must be a string"))?
        }
      }
    };

    Ok(())
  }
}

struct LogFileObservabilityBackend {
  cancel_token: tokio_util::sync::CancellationToken,
  logging_tx: Sender<LogMessage>,
}

impl ObservabilityBackend for LogFileObservabilityBackend {
  fn get_log_channel(&self) -> Option<Sender<LogMessage>> {
    Some(self.logging_tx.clone())
  }
}

impl Drop for LogFileObservabilityBackend {
  fn drop(&mut self) {
    self.cancel_token.cancel();
  }
}
