use std::collections::HashMap;
use std::sync::Arc;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tokio::time::{interval, Duration};

use ferron_core::{
    config::ServerConfigurationBlock, loader::ModuleLoader, log_error, providers::Provider,
    registry::Registry, Module,
};
use ferron_observability::{
    AccessEvent, Event, EventSink, LogFormatterContext, LogLevel, ObservabilityContext,
};

/// Wrapper that carries an event with its configuration through the channel
struct ConfiguredEvent {
    event: Event,
    log_config: Arc<ServerConfigurationBlock>,
}

/// The initialized event sink that writes events to log files
struct LogFileEventSink {
    inner: kanal::AsyncSender<ConfiguredEvent>,
    log_config: Arc<ServerConfigurationBlock>,
}

impl EventSink for LogFileEventSink {
    fn emit(&self, event: Event) {
        let _ = self.inner.try_send(ConfiguredEvent {
            event,
            log_config: self.log_config.clone(),
        });
    }
}

/// File handle wrapper with path tracking
struct FileHandle {
    file: tokio::fs::File,
    buffer: Vec<u8>,
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

    async fn write_to_file(
        &mut self,
        path: String,
        content: &[u8],
    ) -> Result<(), Box<dyn std::error::Error>> {
        if !self.handles.contains_key(&path) {
            match OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .await
            {
                Ok(file) => {
                    self.handles.insert(
                        path.clone(),
                        FileHandle {
                            file,
                            buffer: Vec::with_capacity(4096),
                        },
                    );
                }
                Err(e) => {
                    log_error!("Failed to open log file {}: {}", path, e);
                    return Err(Box::new(e));
                }
            }
        }

        if let Some(handle) = self.handles.get_mut(&path) {
            handle.buffer.extend_from_slice(content);
        }

        Ok(())
    }

    async fn flush_all(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        for handle in self.handles.values_mut() {
            if !handle.buffer.is_empty() {
                handle.file.write_all(&handle.buffer).await?;
                handle.buffer.clear();
            }
        }
        Ok(())
    }
}

struct LogFileObservabilityModule {
    inner: kanal::AsyncReceiver<ConfiguredEvent>,
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
            let mut file_writer = FileWriter::new(1000);
            let mut flush_timer = interval(Duration::from_millis(file_writer.flush_interval_ms));

            loop {
                tokio::select! {
                    result = rx.recv() => {
                        if let Ok(msg) = result {
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
                                            let _ = file_writer
                                            .write_to_file(access_log_path.to_string(),
                                                line.as_bytes())
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
                                        let line = format!("[{} {}] {}\n",
                                            chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
                                            match le.level {
                                            LogLevel::Error => "ERROR",
                                            LogLevel::Warn => "WARN",
                                            LogLevel::Info => "INFO",
                                            LogLevel::Debug => "DEBUG",
                                        },  le.message.clone());
                                        let _ = file_writer
                                            .write_to_file(log_path.to_string(), line.as_bytes())
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
    inner: kanal::AsyncSender<ConfiguredEvent>,
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
        kanal::AsyncSender<ConfiguredEvent>,
        kanal::AsyncReceiver<ConfiguredEvent>,
    ),
}

impl Default for LogFileObservabilityModuleLoader {
    fn default() -> Self {
        Self {
            cache: None,
            channel: kanal::unbounded_async(),
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
