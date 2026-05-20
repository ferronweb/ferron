use ferron_core::log_warn;
use std::collections::HashMap;
use std::error::Error;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Once};
use tokio::fs::OpenOptions;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::time::{interval, Duration, MissedTickBehavior};

use ferron_core::{
    config::ServerConfigurationBlock, loader::ModuleLoader, log_error, providers::Provider,
    registry::Registry, Module,
};
use ferron_observability::{
    AccessEvent, Event, EventSink, LogFormatterContext, LogLevel, ObservabilityContext,
};

static DROPPED_EVENT: Once = Once::new();

/// Wrapper that carries an event with its configuration through the channel
struct ConfiguredEvent {
    event: Event,
    log_config: Arc<ServerConfigurationBlock>,
}

/// The initialized event sink that writes events to log files
struct LogFileEventSink {
    inner: async_channel::Sender<ConfiguredEvent>,
    log_config: Arc<ServerConfigurationBlock>,
}

impl EventSink for LogFileEventSink {
    fn emit(&self, event: Event) {
        if matches!(event, Event::Access(_) | Event::Log(_)) {
            match self.inner.try_send(ConfiguredEvent {
                event,
                log_config: self.log_config.clone(),
            }) {
                Ok(_) => {
                    ferron_core::admin::ADMIN_METRICS
                        .observability_event_queue_len
                        .fetch_add(1, Ordering::Relaxed);
                }
                Err(_) => {
                    // Increment global dropped-events metric
                    ferron_core::admin::ADMIN_METRICS
                        .observability_events_dropped
                        .fetch_add(1, Ordering::Relaxed);

                    DROPPED_EVENT.call_once(|| {
                        log_warn!(
                            "Observability event dropped (`file` observability backend). \
                            This may be caused by high server load."
                        );
                    });
                }
            }
        }
    }

    fn emit_arc(&self, event: std::sync::Arc<Event>) {
        if matches!(&*event, Event::Access(_) | Event::Log(_)) {
            match self.inner.try_send(ConfiguredEvent {
                event: Arc::unwrap_or_clone(event),
                log_config: self.log_config.clone(),
            }) {
                Ok(_) => {
                    ferron_core::admin::ADMIN_METRICS
                        .observability_event_queue_len
                        .fetch_add(1, Ordering::Relaxed);
                }
                Err(_) => {
                    // Increment global dropped-events metric
                    ferron_core::admin::ADMIN_METRICS
                        .observability_events_dropped
                        .fetch_add(1, Ordering::Relaxed);

                    DROPPED_EVENT.call_once(|| {
                        log_warn!(
                            "Observability event dropped (`file` observability backend). \
                            This may be caused by high server load."
                        );
                    });
                }
            }
        }
    }

    fn processes_access(&self) -> bool {
        true
    }
}

/// Rotates the log file if it is too large
async fn rotate_log_file(
    log_filename: &str,
    rotate_keep: Option<usize>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
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
                if i == 0 {
                    String::new()
                } else {
                    format!(".{i}")
                }
            ),
            format!("{log_filename}.{}", i + 1),
        )
        .await?;
    }

    Ok(())
}

/// Rotation configuration for a log file
#[derive(Clone, Copy)]
struct RotationConfig {
    /// Rotate when file size exceeds this value (in bytes)
    rotate_size: Option<u64>,
    /// Number of rotated log files to keep
    rotate_keep: Option<usize>,
}

/// File handle wrapper with BufWriter and rotation support
struct FileHandle {
    writer: BufWriter<tokio::fs::File>,
    current_size: u64,
    rotation: Option<RotationConfig>,
}

/// Manages buffered file handles and flushing
struct FileWriter {
    handles: HashMap<String, FileHandle>,
    flush_interval_ms: u64,
}

impl FileWriter {
    fn new(flush_interval_ms: u64) -> Self {
        Self {
            handles: HashMap::new(),
            flush_interval_ms,
        }
    }

    /// Ensure a file handle exists for the given path
    async fn ensure_handle(
        &mut self,
        path: &str,
        rotation: Option<RotationConfig>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if !self.handles.contains_key(path) {
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .await;

            match file {
                Ok(file) => {
                    let current_size = file.metadata().await?.len();
                    self.handles.insert(
                        path.to_string(),
                        FileHandle {
                            writer: BufWriter::with_capacity(131072, file),
                            current_size,
                            rotation,
                        },
                    );
                }
                Err(e) => {
                    log_error!("Failed to open log file {}: {}", path, e);
                    return Err(Box::new(e));
                }
            }
        }

        // Update rotation config if it changed
        if let Some(handle) = self.handles.get_mut(path) {
            handle.rotation = rotation;
        }

        Ok(())
    }

    /// Write content to a log file, rotating if necessary
    async fn write_to_file(
        &mut self,
        path: &str,
        content: &[u8],
        rotation: Option<RotationConfig>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.ensure_handle(path, rotation).await?;

        // Check if rotation is needed
        let needs_rotation = rotation.and_then(|r| r.rotate_size).is_some_and(|rot| {
            self.handles
                .get(path)
                .is_some_and(|h| h.current_size >= rot)
        });

        if needs_rotation {
            // Flush and remove the old handle
            if let Some(mut handle) = self.handles.remove(path) {
                if let Err(e) = handle.writer.flush().await {
                    log_error!("Failed to flush log file before rotation {}: {}", path, e);
                }
            }

            let rotate_keep = rotation.and_then(|r| r.rotate_keep);
            if let Err(e) = rotate_log_file(path, rotate_keep).await {
                log_error!("Failed to rotate log file {}: {}", path, e);
            }

            // Re-open the file
            match OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .await
            {
                Ok(file) => {
                    self.handles.insert(
                        path.to_string(),
                        FileHandle {
                            writer: BufWriter::with_capacity(131072, file),
                            current_size: 0,
                            rotation,
                        },
                    );
                }
                Err(e) => {
                    log_error!("Failed to re-open log file after rotation {}: {}", path, e);
                    return Err(Box::new(e));
                }
            }
        }

        if let Some(handle) = self.handles.get_mut(path) {
            handle.writer.write_all(content).await?;
            handle.current_size += content.len() as u64;
        }

        Ok(())
    }

    async fn flush_all(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        for handle in self.handles.values_mut() {
            handle.writer.flush().await?;
        }
        Ok(())
    }
}

struct LogFileObservabilityModule {
    inner: async_channel::Receiver<ConfiguredEvent>,
    cancel_token: tokio_util::sync::CancellationToken,
    registry: Arc<Registry>,
}

impl Module for LogFileObservabilityModule {
    fn name(&self) -> &str {
        "observability-logfile"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn start(
        &self,
        runtime: &mut ferron_core::runtime::Runtime,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let cancel_token = self.cancel_token.clone();
        let registry = self.registry.clone();

        let rx = self.inner.clone();
        runtime.spawn_secondary_task(async move {
            let mut file_writer = FileWriter::new(100);
            let mut flush_timer = interval(Duration::from_millis(file_writer.flush_interval_ms));
            flush_timer.set_missed_tick_behavior(MissedTickBehavior::Skip);

            loop {
                tokio::select! {
                    biased;

                    result = rx.recv() => {
                        if let Ok(msg) = result {
                            ferron_core::admin::ADMIN_METRICS
                                .observability_event_queue_len
                                .fetch_sub(1, Ordering::Relaxed);

                            match &msg.event {
                                Event::Access(ae) => {
                                    if let Some(access_log_path) =
                                      msg.log_config.get_value("access_log")
                                          .and_then(|v|
                                           v.as_string_with_interpolations(&HashMap::new())) {
                                        if let Some(message) =
                                          format_access_event(ae, &msg.log_config, &registry) {
                                            let mut line = message;
                                            line.push('\n');

                                            // Read rotation config
                                            let rotation = read_rotation_config(
                                                &msg.log_config,
                                                "access_log_rotate_size",
                                                "access_log_rotate_keep",
                                            );

                                            let _ = file_writer
                                            .write_to_file(&access_log_path,
                                                line.as_bytes(),
                                                rotation)
                                            .await;
                                        }
                                    }
                                }
                                Event::Log(le) => {
                                    let log_path = msg.log_config
                                        .get_value("error_log")
                                        .and_then(|v| v
                                            .as_string_with_interpolations(&HashMap::new()));

                                    if let Some(log_path) = log_path {
                                        let message = le.message.to_string().replace("\n", "\n  ");
                                        let line = format!("[{} {}] {}\n",
                                            chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
                                            match le.level {
                                            LogLevel::Error => "ERROR",
                                            LogLevel::Warn => "WARN",
                                            LogLevel::Info => "INFO",
                                            LogLevel::Debug => "DEBUG",
                                        },  message.trim());

                                        // Read rotation config for error log
                                        let rotation = read_rotation_config(
                                            &msg.log_config,
                                            "error_log_rotate_size",
                                            "error_log_rotate_keep",
                                        );

                                        let _ = file_writer
                                            .write_to_file(&log_path, line.as_bytes(), rotation)
                                            .await;
                                    }
                                }
                                _ => {
                                    // Ignore other event types
                                }
                            }
                        } else {
                            break;
                        }
                    }
                    _ = flush_timer.tick() => {
                        if let Err(e) = file_writer.flush_all().await {
                            log_error!("Failed to flush log files: {}", e);
                        }
                    }
                    _ = cancel_token.cancelled() => {
                        let _ = file_writer.flush_all().await;
                        break;
                    }
                }
            }
        });

        Ok(())
    }
}

impl Drop for LogFileObservabilityModule {
    fn drop(&mut self) {
        self.cancel_token.cancel();
    }
}

/// Read rotation configuration from the log config block
fn read_rotation_config(
    log_config: &ServerConfigurationBlock,
    rotate_size_directive: &str,
    rotate_keep_directive: &str,
) -> Option<RotationConfig> {
    let rotate_size = log_config
        .get_value(rotate_size_directive)
        .and_then(|v| v.as_number())
        .filter(|&v| v > 0)
        .map(|v| v as u64);

    let rotate_keep = log_config
        .get_value(rotate_keep_directive)
        .and_then(|v| v.as_number())
        .filter(|&v| v >= 0)
        .map(|v| v as usize);

    // Only return Some if at least one rotation setting is configured
    if rotate_size.is_some() || rotate_keep.is_some() {
        Some(RotationConfig {
            rotate_size,
            rotate_keep,
        })
    } else {
        None
    }
}

fn format_access_event(
    access_event: &Arc<dyn AccessEvent>,
    log_config: &Arc<ServerConfigurationBlock>,
    registry: &Registry,
) -> Option<String> {
    let formatter_name = log_config
        .get_value("format")
        .and_then(|v| v.as_str())
        .unwrap_or("text");

    // Try to resolve the formatter from the registry
    if let Some(formatter_registry) = registry.get_provider_registry::<LogFormatterContext>() {
        if let Some(formatter) = formatter_registry.get(formatter_name) {
            let mut ctx = LogFormatterContext {
                access_event: access_event.clone(),
                log_config: log_config.clone(),
                output: None,
            };
            if formatter.execute(&mut ctx).is_ok() {
                if let Some(output) = ctx.output {
                    return Some(output);
                }
            }
        }
    }

    None
}

struct LogFileObservabilityProvider {
    inner: async_channel::Sender<ConfiguredEvent>,
}

impl Provider<ObservabilityContext> for LogFileObservabilityProvider {
    fn name(&self) -> &str {
        "file"
    }

    fn execute(&self, ctx: &mut ObservabilityContext) -> Result<(), Box<dyn std::error::Error>> {
        ctx.sink = Some(Arc::new(LogFileEventSink {
            inner: self.inner.clone(),
            log_config: ctx.log_config.clone(),
        }));
        Ok(())
    }
}

pub struct LogFileObservabilityModuleLoader {
    cache: Option<Arc<LogFileObservabilityModule>>,
    channel: (
        async_channel::Sender<ConfiguredEvent>,
        async_channel::Receiver<ConfiguredEvent>,
    ),
}

impl Default for LogFileObservabilityModuleLoader {
    fn default() -> Self {
        Self {
            cache: None,
            channel: async_channel::bounded(131072),
        }
    }
}

impl ModuleLoader for LogFileObservabilityModuleLoader {
    fn register_providers(
        &mut self,
        registry: ferron_core::registry::RegistryBuilder,
    ) -> ferron_core::registry::RegistryBuilder {
        let channel = self.channel.0.clone();

        registry.with_provider::<ObservabilityContext, _>(move || {
            Arc::new(LogFileObservabilityProvider {
                inner: channel.clone(),
            })
        })
    }

    fn register_modules(
        &mut self,
        registry: Arc<ferron_core::registry::Registry>,
        modules: &mut Vec<Arc<dyn ferron_core::Module>>,
        _config: Arc<ferron_core::config::ServerConfiguration>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if self.cache.is_none() {
            let module = Arc::new(LogFileObservabilityModule {
                inner: self.channel.1.clone(),
                cancel_token: tokio_util::sync::CancellationToken::new(),
                registry: registry.clone(),
            });

            self.cache = Some(module.clone());
            modules.push(module);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferron_core::config::{
        ServerConfigurationBlockBuilder, ServerConfigurationDirectiveEntry,
        ServerConfigurationValue,
    };

    fn make_log_config(
        access_log: Option<&str>,
        error_log: Option<&str>,
        rotate_size: Option<u64>,
        rotate_keep: Option<usize>,
    ) -> Arc<ServerConfigurationBlock> {
        let mut builder = ServerConfigurationBlockBuilder::new();

        if let Some(path) = access_log {
            builder = builder.directive_str("access_log", vec![path]);
        }
        if let Some(path) = error_log {
            builder = builder.directive_str("error_log", vec![path]);
        }
        if let Some(size) = rotate_size {
            let entry = ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::Number(size as i64, None)],
                children: None,
                span: None,
            };
            builder = builder.directive("access_log_rotate_size", entry);
        }
        if let Some(keep) = rotate_keep {
            let entry = ServerConfigurationDirectiveEntry {
                args: vec![ServerConfigurationValue::Number(keep as i64, None)],
                children: None,
                span: None,
            };
            builder = builder.directive("access_log_rotate_keep", entry);
        }

        Arc::new(builder.build())
    }

    #[tokio::test]
    async fn test_rotate_log_file_basic() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("access.log");
        tokio::fs::write(&log_path, b"original content")
            .await
            .unwrap();

        let path_str = log_path.to_string_lossy().to_string();
        rotate_log_file(&path_str, Some(3)).await.unwrap();

        assert!(!tokio::fs::try_exists(&log_path).await.unwrap());
        assert!(tokio::fs::try_exists(format!("{}.1", path_str))
            .await
            .unwrap());
        let rotated = tokio::fs::read(format!("{}.1", path_str)).await.unwrap();
        assert_eq!(rotated, b"original content");
    }

    #[tokio::test]
    async fn test_rotate_log_file_multiple_rotations() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("access.log");

        tokio::fs::write(&log_path, b"first").await.unwrap();
        let path_str = log_path.to_string_lossy().to_string();
        rotate_log_file(&path_str, Some(5)).await.unwrap();

        tokio::fs::write(&log_path, b"second").await.unwrap();
        rotate_log_file(&path_str, Some(5)).await.unwrap();

        let r1 = tokio::fs::read(format!("{}.1", path_str)).await.unwrap();
        let r2 = tokio::fs::read(format!("{}.2", path_str)).await.unwrap();
        assert_eq!(r1, b"second");
        assert_eq!(r2, b"first");
    }

    #[tokio::test]
    async fn test_rotate_log_file_keep_limit() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("access.log");
        let path_str = log_path.to_string_lossy().to_string();

        for content in [b"v1", b"v2", b"v3", b"v4"] {
            tokio::fs::write(&log_path, content).await.unwrap();
            rotate_log_file(&path_str, Some(2)).await.unwrap();
        }

        assert!(!tokio::fs::try_exists(format!("{}.3", path_str))
            .await
            .unwrap());
        assert!(tokio::fs::try_exists(format!("{}.1", path_str))
            .await
            .unwrap());
        assert!(tokio::fs::try_exists(format!("{}.2", path_str))
            .await
            .unwrap());

        let r1 = tokio::fs::read(format!("{}.1", path_str)).await.unwrap();
        let r2 = tokio::fs::read(format!("{}.2", path_str)).await.unwrap();
        assert_eq!(r1, b"v4");
        assert_eq!(r2, b"v3");
    }

    #[tokio::test]
    async fn test_rotate_log_file_keep_zero() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("access.log");
        tokio::fs::write(&log_path, b"should be deleted")
            .await
            .unwrap();

        let path_str = log_path.to_string_lossy().to_string();
        rotate_log_file(&path_str, Some(0)).await.unwrap();

        assert!(!tokio::fs::try_exists(&log_path).await.unwrap());
        assert!(!tokio::fs::try_exists(format!("{}.1", path_str))
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn test_rotate_log_file_no_limit() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("access.log");
        let path_str = log_path.to_string_lossy().to_string();

        for content in [b"a", b"b", b"c", b"d", b"e"] {
            tokio::fs::write(&log_path, content).await.unwrap();
            rotate_log_file(&path_str, None).await.unwrap();
        }

        for i in 1..=5 {
            assert!(
                tokio::fs::try_exists(format!("{}.{}", path_str, i))
                    .await
                    .unwrap(),
                "Expected {}.{} to exist",
                path_str,
                i
            );
        }
    }

    #[tokio::test]
    async fn test_rotate_log_file_nonexistent() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("nonexistent.log");
        let path_str = log_path.to_string_lossy().to_string();

        let result = rotate_log_file(&path_str, Some(3)).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_read_rotation_config_both_set() {
        let config = make_log_config(Some("/tmp/access.log"), None, Some(100), Some(5));
        let rotation =
            read_rotation_config(&config, "access_log_rotate_size", "access_log_rotate_keep")
                .unwrap();
        assert_eq!(rotation.rotate_size, Some(100));
        assert_eq!(rotation.rotate_keep, Some(5));
    }

    #[tokio::test]
    async fn test_read_rotation_config_only_size() {
        let config = make_log_config(Some("/tmp/access.log"), None, Some(200), None);
        let rotation =
            read_rotation_config(&config, "access_log_rotate_size", "access_log_rotate_keep")
                .unwrap();
        assert_eq!(rotation.rotate_size, Some(200));
        assert_eq!(rotation.rotate_keep, None);
    }

    #[tokio::test]
    async fn test_read_rotation_config_only_keep() {
        let config = make_log_config(Some("/tmp/access.log"), None, None, Some(3));
        let rotation =
            read_rotation_config(&config, "access_log_rotate_size", "access_log_rotate_keep")
                .unwrap();
        assert_eq!(rotation.rotate_size, None);
        assert_eq!(rotation.rotate_keep, Some(3));
    }

    #[tokio::test]
    async fn test_read_rotation_config_neither_set() {
        let config = make_log_config(Some("/tmp/access.log"), None, None, None);
        let rotation =
            read_rotation_config(&config, "access_log_rotate_size", "access_log_rotate_keep");
        assert!(rotation.is_none());
    }

    #[tokio::test]
    async fn test_read_rotation_config_zero_size() {
        let config = make_log_config(Some("/tmp/access.log"), None, Some(0), Some(3));
        let rotation =
            read_rotation_config(&config, "access_log_rotate_size", "access_log_rotate_keep")
                .unwrap();
        assert_eq!(rotation.rotate_size, None);
        assert_eq!(rotation.rotate_keep, Some(3));
    }

    #[tokio::test]
    async fn test_read_rotation_config_zero_keep() {
        let config = make_log_config(Some("/tmp/access.log"), None, Some(100), Some(0));
        let rotation =
            read_rotation_config(&config, "access_log_rotate_size", "access_log_rotate_keep")
                .unwrap();
        assert_eq!(rotation.rotate_size, Some(100));
        assert_eq!(rotation.rotate_keep, Some(0));
    }

    #[tokio::test]
    async fn test_file_writer_write_without_rotation() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("access.log");
        let path_str = log_path.to_string_lossy().to_string();

        let mut writer = FileWriter::new(100);
        writer
            .write_to_file(&path_str, b"hello\n", None)
            .await
            .unwrap();
        writer.flush_all().await.unwrap();

        let content = tokio::fs::read(&log_path).await.unwrap();
        assert_eq!(content, b"hello\n");
    }

    #[tokio::test]
    async fn test_file_writer_triggers_rotation() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("access.log");
        let path_str = log_path.to_string_lossy().to_string();

        let rotation = Some(RotationConfig {
            rotate_size: Some(5),
            rotate_keep: Some(3),
        });

        let mut writer = FileWriter::new(100);

        writer
            .write_to_file(&path_str, b"12345", rotation)
            .await
            .unwrap();
        writer.flush_all().await.unwrap();

        let content = tokio::fs::read(&log_path).await.unwrap();
        assert_eq!(content, b"12345");

        writer
            .write_to_file(&path_str, b"next", rotation)
            .await
            .unwrap();
        writer.flush_all().await.unwrap();

        assert!(tokio::fs::try_exists(format!("{}.1", path_str))
            .await
            .unwrap());
        let rotated = tokio::fs::read(format!("{}.1", path_str)).await.unwrap();
        assert_eq!(rotated, b"12345");

        let current = tokio::fs::read(&log_path).await.unwrap();
        assert_eq!(current, b"next");
    }

    #[tokio::test]
    async fn test_file_writer_multiple_rotations() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("access.log");
        let path_str = log_path.to_string_lossy().to_string();

        let rotation = Some(RotationConfig {
            rotate_size: Some(3),
            rotate_keep: Some(5),
        });

        let mut writer = FileWriter::new(100);

        for chunk in [b"aaa", b"bbb", b"ccc"] {
            writer
                .write_to_file(&path_str, chunk, rotation)
                .await
                .unwrap();
        }
        writer.flush_all().await.unwrap();

        assert!(tokio::fs::try_exists(format!("{}.1", path_str))
            .await
            .unwrap());
        assert!(tokio::fs::try_exists(format!("{}.2", path_str))
            .await
            .unwrap());

        let r1 = tokio::fs::read(format!("{}.1", path_str)).await.unwrap();
        let r2 = tokio::fs::read(format!("{}.2", path_str)).await.unwrap();
        assert_eq!(r1, b"bbb");
        assert_eq!(r2, b"aaa");
    }

    #[tokio::test]
    async fn test_file_writer_current_size_reset_after_rotation() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("access.log");
        let path_str = log_path.to_string_lossy().to_string();

        let rotation = Some(RotationConfig {
            rotate_size: Some(4),
            rotate_keep: Some(3),
        });

        let mut writer = FileWriter::new(100);

        writer
            .write_to_file(&path_str, b"1234", rotation)
            .await
            .unwrap();
        writer.flush_all().await.unwrap();

        let handle = writer.handles.get(&path_str).unwrap();
        assert_eq!(handle.current_size, 4);

        writer
            .write_to_file(&path_str, b"next", rotation)
            .await
            .unwrap();
        writer.flush_all().await.unwrap();

        let handle = writer.handles.get(&path_str).unwrap();
        assert_eq!(handle.current_size, 4);
    }

    #[tokio::test]
    async fn test_file_writer_existing_file_size_tracked() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("access.log");
        let path_str = log_path.to_string_lossy().to_string();

        tokio::fs::write(&log_path, b"pre-existing content")
            .await
            .unwrap();

        let rotation = Some(RotationConfig {
            rotate_size: Some(25),
            rotate_keep: Some(3),
        });

        let mut writer = FileWriter::new(100);
        writer
            .write_to_file(&path_str, b"more", rotation)
            .await
            .unwrap();
        writer.flush_all().await.unwrap();

        let handle = writer.handles.get(&path_str).unwrap();
        assert_eq!(handle.current_size, 24);
    }

    #[tokio::test]
    async fn test_file_writer_rotation_respects_keep_limit() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("access.log");
        let path_str = log_path.to_string_lossy().to_string();

        let rotation = Some(RotationConfig {
            rotate_size: Some(3),
            rotate_keep: Some(2),
        });

        let mut writer = FileWriter::new(100);

        for chunk in [b"aaa", b"bbb", b"ccc", b"ddd"] {
            writer
                .write_to_file(&path_str, chunk, rotation)
                .await
                .unwrap();
        }
        writer.flush_all().await.unwrap();

        assert!(!tokio::fs::try_exists(format!("{}.3", path_str))
            .await
            .unwrap());
        assert!(tokio::fs::try_exists(format!("{}.1", path_str))
            .await
            .unwrap());
        assert!(tokio::fs::try_exists(format!("{}.2", path_str))
            .await
            .unwrap());
    }
}
