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

struct LogFile {
  filename: String,
  rotate_size: Option<usize>,
  rotate_keep: Option<usize>,
  writer: Arc<tokio::sync::Mutex<BufWriter<tokio::fs::File>>>,
  size: Option<Arc<tokio::sync::RwLock<u64>>>,
}

impl LogFile {
  async fn new(
    filename: String,
    rotate_size: Option<usize>,
    rotate_keep: Option<usize>,
  ) -> Result<Self, Box<dyn Error + Send + Sync>> {
    let file = tokio::fs::OpenOptions::new()
      .append(true)
      .create(true)
      .open(&filename)
      .await?;

    let size = if rotate_size.is_some() {
      Some(Arc::new(tokio::sync::RwLock::new(file.metadata().await?.len())))
    } else {
      None
    };

    Ok(Self {
      filename,
      rotate_size,
      rotate_keep,
      writer: Arc::new(tokio::sync::Mutex::new(BufWriter::with_capacity(131072, file))),
      size,
    })
  }

  async fn write(&mut self, mut message: String) {
    message.push('\n');

    let current_size = if let Some(size_lock) = &self.size {
      Some(*size_lock.read().await)
    } else {
      None
    };

    if let (Some(current_size), Some(rotate_size)) = (current_size, self.rotate_size) {
      if current_size > rotate_size as u64 {
        if let Err(e) = self.rotate().await {
          eprintln!("Failed to rotate log file: {e}");
        }
      }
    }

    let writer = self.writer.clone();
    let size_lock = self.size.clone();
    tokio::task::spawn(async move {
      let mut locked_file = writer.lock().await;
      if let Err(e) = locked_file.write(message.as_bytes()).await {
        eprintln!("Failed to write to log file: {e}");
      }
      if let Some(size_lock) = size_lock {
        size_lock.write().await.add_assign(message.len() as u64);
      }
    });
  }

  async fn flush(&self) {
    let writer = self.writer.clone();
    tokio::task::spawn(async move {
      let mut locked_file = writer.lock().await;
      locked_file.flush().await.unwrap_or_default();
    });
  }

  async fn rotate(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> {
    rotate_log_file(&self.filename, self.rotate_keep).await?;

    let file = tokio::fs::OpenOptions::new()
      .append(true)
      .create(true)
      .open(&self.filename)
      .await?;
    let new_writer = BufWriter::with_capacity(131072, file);

    {
      let mut old_writer = self.writer.lock().await;
      let _ = old_writer.flush().await;
    }

    self.writer = Arc::new(tokio::sync::Mutex::new(new_writer));
    if self.size.is_some() {
      self.size = Some(Arc::new(tokio::sync::RwLock::new(0)));
    }

    Ok(())
  }
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
          let error_log_rotate_keep = get_value!("error_log_rotate_keep", config)
            .and_then(|v| v.as_i128())
            .map(|v| v as usize);
          let (logging_tx, logging_rx) = async_channel::unbounded::<LogMessage>();
          secondary_runtime.spawn(async move {
            let mut access_log = if let Some(filename) = log_filename {
              match LogFile::new(filename, log_rotate_size, log_rotate_keep).await {
                Ok(l) => Some(l),
                Err(e) => {
                  eprintln!("Failed to open log file: {e}");
                  None
                }
              }
            } else {
              None
            };

            let mut error_log = if let Some(filename) = error_log_filename {
              match LogFile::new(filename, error_log_rotate_size, error_log_rotate_keep).await {
                Ok(l) => Some(l),
                Err(e) => {
                  eprintln!("Failed to open error log file: {e}");
                  None
                }
              }
            } else {
              None
            };

            let mut interval = tokio::time::interval(Duration::from_millis(100));

            // Logging loop
            loop {
              tokio::select! {
                message = logging_rx.recv() => {
                   match message {
                       Ok(message) => {
                          let (mut message, is_error) = message.get_message();
                          if is_error {
                              let now: DateTime<Local> = Local::now();
                              let formatted_time = now.format("%Y-%m-%d %H:%M:%S").to_string();
                              message = format!("[{formatted_time}]: {message}");
                              if let Some(log) = &mut error_log {
                                  log.write(message).await;
                              }
                          } else if let Some(log) = &mut access_log {
                              log.write(message).await;
                          }
                       }
                       Err(_) => break, // Channel closed
                   }
                }
                _ = interval.tick() => {
                    if let Some(log) = &access_log {
                        log.flush().await;
                    }
                    if let Some(log) = &error_log {
                        log.flush().await;
                    }
                }
                _ = cancel_token.cancelled() => return,
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
