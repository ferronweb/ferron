use ferron_core::{config_validator_scoped_key, log_warn};
use std::collections::HashMap;
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
    AccessEvent, ApplicationLogFormatterContext, Event, EventSink, LogEvent, LogFormatterContext,
    ObservabilityContext,
};

use crate::rotate::{rotate_log_file, RotationConfig};

mod rotate;
mod validator;

static DROPPED_EVENT: Once = Once::new();

/// Wrapper that carries an event with its configuration through the channel
struct ConfiguredEvent {
    event: Arc<Event>,
    log_config: Arc<ServerConfigurationBlock>,
}

/// The initialized event sink that writes events to log files
struct LogFileEventSink {
    inner: async_channel::Sender<ConfiguredEvent>,
    log_config: Arc<ServerConfigurationBlock>,
}

impl EventSink for LogFileEventSink {
    #[inline]
    fn emit(&self, event: Event) {
        if matches!(event, Event::Access(_) | Event::Log(_)) {
            match self.inner.try_send(ConfiguredEvent {
                event: Arc::new(event),
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

    #[inline]
    fn emit_arc(&self, event: std::sync::Arc<Event>) {
        if matches!(&*event, Event::Access(_) | Event::Log(_)) {
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

    #[inline]
    fn processes_access(&self) -> bool {
        true
    }
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

                            match &*msg.event {
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
                                            let rotation = RotationConfig::read_from_config(
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
                                        if let Some(message) =
                                          format_log_event(le, &msg.log_config, &registry) {
                                        let mut message = message.to_string().replace("\n", "\n  ");
                                        message.push('\n');

                                        // Read rotation config for error log
                                        let rotation = RotationConfig::read_from_config(
                                            &msg.log_config,
                                            "error_log_rotate_size",
                                            "error_log_rotate_keep",
                                        );

                                        let _ = file_writer
                                            .write_to_file(&log_path, message.as_bytes(), rotation)
                                            .await;
                                        }
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

fn format_log_event(
    log_event: &LogEvent,
    log_config: &Arc<ServerConfigurationBlock>,
    registry: &Registry,
) -> Option<String> {
    let formatter_name = log_config
        .get_value("error_format")
        .and_then(|v| v.as_str())
        .unwrap_or("text");

    // Try to resolve the formatter from the registry
    if let Some(formatter_registry) =
        registry.get_provider_registry::<ApplicationLogFormatterContext<'static>>()
    {
        if let Some(formatter) = formatter_registry.get(formatter_name) {
            // SAFETY: We know that the lifetime of the log event is longer
            //         than the lifetime of the resolver. but "'static"
            //         is the only lifetime we can use here. This
            //         constraint is enforced by the provider registry.
            let log_event =
                unsafe { std::mem::transmute::<&LogEvent, &'static LogEvent>(log_event) };
            let mut ctx = ApplicationLogFormatterContext {
                log_event,
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

    fn register_scoped_configuration_validators(
        &mut self,
        registry: &mut HashMap<
            ferron_core::config::validator::ConfigurationValidatorScopedKey,
            Box<dyn ferron_core::config::validator::ConfigurationValidator>,
        >,
    ) {
        registry.insert(
            config_validator_scoped_key!("observability", "file"),
            Box::new(validator::LogFileObservabilityConfigurationValidator),
        );
    }
}
