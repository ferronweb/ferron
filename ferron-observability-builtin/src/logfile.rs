use std::{error::Error, ops::AddAssign, sync::Arc, time::Duration};

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

/// Rotates the log file if it is too large
#[inline]
async fn rotate_log_file(log_filename: &str, rotate_keep: Option<usize>) -> Result<(), Box<dyn Error + Send + Sync>> {
  // If we are not keeping any logs, just delete the current log file
  if rotate_keep == Some(0) {
    tokio::fs::remove_file(log_filename).await?;
    return Ok(());
  }

  // Find the oldest log file
  let mut oldest_log_file_suffix = 0;
  while rotate_keep.is_none_or(|k| oldest_log_file_suffix < k)
    && tokio::fs::try_exists(format!("{log_filename}.{}", oldest_log_file_suffix + 1)).await?
  {
    oldest_log_file_suffix += 1;
  }

  // Delete the oldest log file if we are keeping too many
  if rotate_keep.is_some_and(|k| oldest_log_file_suffix >= k) {
    tokio::fs::remove_file(format!("{log_filename}.{oldest_log_file_suffix}")).await?;
    oldest_log_file_suffix -= 1;
  }

  // Rotate the log files
  for i in (0..=oldest_log_file_suffix).rev() {
    tokio::fs::rename(
      format!(
        "{log_filename}{}",
        if i == 0 { "".to_string() } else { format!(".{i}") }
      ),
      format!("{log_filename}.{}", i + 1),
    )
    .await?;
  }

  Ok(())
}

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
      cache: ModuleCache::new(vec![
        "log",
        "error_log",
        "log_rotate_size",
        "log_rotate_keep",
        "error_log_rotate_size",
        "error_log_rotate_keep",
      ]),
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
          let log_rotate_size = get_value!("log_rotate_size", config)
            .and_then(|v| v.as_i128())
            .map(|v| v as usize);
          let log_rotate_keep = get_value!("log_rotate_keep", config)
            .and_then(|v| v.as_i128())
            .map(|v| v as usize);
          let error_log_rotate_size = get_value!("error_log_rotate_size", config)
            .and_then(|v| v.as_i128())
            .map(|v| v as usize);
          let _error_log_rotate_keep = get_value!("error_log_rotate_keep", config)
            .and_then(|v| v.as_i128())
            .map(|v| v as usize);
          let (logging_tx, logging_rx) = async_channel::unbounded::<LogMessage>();
          secondary_runtime.spawn(async move {
            let log_file = match &log_filename {
              Some(log_filename) => Some(
                tokio::fs::OpenOptions::new()
                  .append(true)
                  .create(true)
                  .open(log_filename)
                  .await,
              ),
              None => None,
            };

            let error_log_file = match &error_log_filename {
              Some(error_log_filename) => Some(
                tokio::fs::OpenOptions::new()
                  .append(true)
                  .create(true)
                  .open(error_log_filename)
                  .await,
              ),
              None => None,
            };

            let mut log_file_size = if log_rotate_size.is_some() {
              if let Some(file) = log_file.as_ref().and_then(|r| r.as_ref().ok()) {
                file
                  .metadata()
                  .await
                  .ok()
                  .map(|m| Arc::new(tokio::sync::RwLock::new(m.len())))
              } else {
                None
              }
            } else {
              None
            };

            let mut error_log_file_size = if error_log_rotate_size.is_some() {
              if let Some(file) = error_log_file.as_ref().and_then(|r| r.as_ref().ok()) {
                file
                  .metadata()
                  .await
                  .ok()
                  .map(|m| Arc::new(tokio::sync::RwLock::new(m.len())))
              } else {
                None
              }
            } else {
              None
            };

            let mut log_file_wrapped = match log_file {
              Some(Ok(file)) => Some(Arc::new(tokio::sync::Mutex::new(BufWriter::with_capacity(
                131072, file,
              )))),
              Some(Err(e)) => {
                eprintln!("Failed to open log file: {e}");
                None
              }
              None => None,
            };

            let mut error_log_file_wrapped = match error_log_file {
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
              let log_file_size_rwlock = if !is_error {
                log_file_size.clone()
              } else {
                error_log_file_size.clone()
              };
              let log_file_size_obtained = if let Some(size_rwlock) = &log_file_size_rwlock {
                Some(*size_rwlock.read().await)
              } else {
                None
              };
              let log_rotate_size_obtained = if !is_error {
                log_rotate_size
              } else {
                error_log_rotate_size
              };

              if let Some(mut log_file_wrapped_cloned) = log_file_wrapped_cloned {
                if is_error {
                  let now: DateTime<Local> = Local::now();
                  let formatted_time = now.format("%Y-%m-%d %H:%M:%S").to_string();
                  message = format!("[{formatted_time}]: {message}");
                }
                message.push('\n');
                if log_file_size_obtained.is_some()
                  && log_rotate_size_obtained.is_some()
                  && log_file_size_obtained > log_rotate_size_obtained.map(|x| x as u64)
                {
                  if let Some(filename) = if is_error { &error_log_filename } else { &log_filename } {
                    if let Err(e) = rotate_log_file(filename, log_rotate_keep).await {
                      eprintln!("Failed to rotate log file: {e}");
                    } else {
                      let logfile_new = match tokio::fs::OpenOptions::new()
                        .append(true)
                        .create(true)
                        .open(filename)
                        .await
                      {
                        Ok(file) => Some(BufWriter::with_capacity(131072, file)),
                        Err(err) => {
                          eprintln!("Failed to open log file: {err}");
                          None
                        }
                      };
                      if let Some(logfile_new) = logfile_new {
                        if is_error {
                          error_log_file_size = Some(Arc::new(tokio::sync::RwLock::new(0)));
                          if let Some(error_log_file_wrapped) = &mut error_log_file_wrapped {
                            let mut locked_file = error_log_file_wrapped.lock().await;
                            let _ = locked_file.flush().await;
                            *locked_file = logfile_new;
                          } else {
                            error_log_file_wrapped = Some(Arc::new(tokio::sync::Mutex::new(logfile_new)));
                          }
                          log_file_wrapped_cloned =
                            error_log_file_wrapped.clone().expect("error_log_file_wrapped is None");
                        } else {
                          log_file_size = Some(Arc::new(tokio::sync::RwLock::new(0)));
                          if let Some(log_file_wrapped) = &mut log_file_wrapped {
                            let mut locked_file = log_file_wrapped.lock().await;
                            let _ = locked_file.flush().await;
                            *locked_file = logfile_new;
                          } else {
                            log_file_wrapped = Some(Arc::new(tokio::sync::Mutex::new(logfile_new)));
                          }
                          log_file_wrapped_cloned = log_file_wrapped.clone().expect("log_file_wrapped is None");
                        }
                      }
                    }
                  }
                }
                tokio::task::spawn(async move {
                  let mut locked_file = log_file_wrapped_cloned.lock().await;
                  if let Err(e) = locked_file.write(message.as_bytes()).await {
                    eprintln!("Failed to write to log file: {e}");
                  }
                  if let Some(rwlock) = log_file_size_rwlock {
                    rwlock.write().await.add_assign(message.len() as u64);
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

    if let Some(entries) = get_entries_for_validation!("log_rotate_size", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `log_rotate_size` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_null() && entry.values[0].as_i128().is_none_or(|v| v < 0) {
          Err(anyhow::anyhow!("Invalid log rotation maximum size"))?
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("log_rotate_keep", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `log_rotate_keep` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_null() && entry.values[0].as_i128().is_none_or(|v| v < 0) {
          Err(anyhow::anyhow!("Invalid log rotation maximum number of files"))?
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("error_log_rotate_size", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `error_log_rotate_size` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_null() && entry.values[0].as_i128().is_none_or(|v| v < 0) {
          Err(anyhow::anyhow!("Invalid error log rotation maximum size"))?
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("error_log_rotate_keep", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `error_log_rotate_keep` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_null() && entry.values[0].as_i128().is_none_or(|v| v < 0) {
          Err(anyhow::anyhow!("Invalid error log rotation maximum number of files"))?
        }
      }
    }

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
