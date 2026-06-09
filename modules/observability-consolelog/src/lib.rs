use std::sync::atomic::Ordering;
use std::sync::{Arc, Once};

use ferron_core::config::validator::{validate_scoped_block_flat, ConfigurationValidator};
use ferron_core::config_validator_scoped_key;
use ferron_core::{
    config::ServerConfigurationBlock, loader::ModuleLoader, log_debug, log_error, log_info,
    log_warn, providers::Provider, registry::Registry, Module,
};
use ferron_observability::{
    AccessEvent, Event, EventSink, LogFormatterContext, ObservabilityContext,
};

static DROPPED_EVENT: Once = Once::new();

/// Wrapper that carries an event with its configuration through the channel
struct ConfiguredEvent {
    event: Event,
    log_config: Arc<ServerConfigurationBlock>,
}

/// The initialized event sink that emits events to the console
struct ConsoleEventSink {
    inner: async_channel::Sender<ConfiguredEvent>,
    log_config: Arc<ServerConfigurationBlock>,
}

impl EventSink for ConsoleEventSink {
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
                    // Increment dropped events metric and warn once
                    ferron_core::admin::ADMIN_METRICS
                        .observability_events_dropped
                        .fetch_add(1, Ordering::Relaxed);

                    DROPPED_EVENT.call_once(|| {
                        log_warn!(
                            "Observability event dropped (`console` observability backend). \
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
                    // Increment dropped events metric and warn once
                    ferron_core::admin::ADMIN_METRICS
                        .observability_events_dropped
                        .fetch_add(1, Ordering::Relaxed);

                    DROPPED_EVENT.call_once(|| {
                        log_warn!(
                            "Observability event dropped (`console` observability backend). \
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

struct ConsoleObservabilityModule {
    inner: async_channel::Receiver<ConfiguredEvent>,
    cancel_token: tokio_util::sync::CancellationToken,
    registry: Arc<Registry>,
}

impl Module for ConsoleObservabilityModule {
    fn name(&self) -> &str {
        "observability-consolelog"
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
            while let Some(msg) = tokio::select! {
                result = rx.recv() => {
                    result.ok()
                }
                _ = cancel_token.cancelled() => {
                    None
                }
            } {
                ferron_core::admin::ADMIN_METRICS
                    .observability_event_queue_len
                    .fetch_sub(1, Ordering::Relaxed);

                let registry = registry.clone();
                tokio::task::spawn_blocking(move || {
                    match msg.event {
                        ferron_observability::Event::Access(ae) => {
                            let message = format_access_event(&ae, &msg.log_config, &registry);
                            if let Some(message) = message {
                                log_info!("{}", message);
                            }
                        }
                        ferron_observability::Event::Log(le) => {
                            let trace_id_part = le
                                .trace_context
                                .as_ref()
                                .and_then(|t| str::from_utf8(&t.span_id).ok())
                                .map(|sid| format!("[trace={}] ", sid))
                                .unwrap_or_default();
                            match le.level {
                                ferron_observability::LogLevel::Error => {
                                    log_error!("{}{}", trace_id_part, le.message)
                                }
                                ferron_observability::LogLevel::Warn => {
                                    log_warn!("{}{}", trace_id_part, le.message)
                                }
                                ferron_observability::LogLevel::Info => {
                                    log_info!("{}{}", trace_id_part, le.message)
                                }
                                ferron_observability::LogLevel::Debug => {
                                    log_debug!("{}{}", trace_id_part, le.message)
                                }
                            }
                        }
                        _ => (), // Ignore unsupported event types
                    }
                });
            }
        });

        Ok(())
    }
}

impl Drop for ConsoleObservabilityModule {
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

struct ConsoleObservabilityProvider {
    inner: async_channel::Sender<ConfiguredEvent>,
}

impl Provider<ObservabilityContext> for ConsoleObservabilityProvider {
    fn name(&self) -> &str {
        "console"
    }

    fn execute(&self, ctx: &mut ObservabilityContext) -> Result<(), Box<dyn std::error::Error>> {
        ctx.sink = Some(Arc::new(ConsoleEventSink {
            inner: self.inner.clone(),
            log_config: ctx.log_config.clone(),
        }));
        Ok(())
    }
}

pub struct ConsoleObservabilityModuleLoader {
    cache: Option<Arc<ConsoleObservabilityModule>>,
    channel: (
        async_channel::Sender<ConfiguredEvent>,
        async_channel::Receiver<ConfiguredEvent>,
    ),
}

impl Default for ConsoleObservabilityModuleLoader {
    fn default() -> Self {
        Self {
            cache: None,
            channel: async_channel::bounded(131072),
        }
    }
}

impl ModuleLoader for ConsoleObservabilityModuleLoader {
    fn register_providers(
        &mut self,
        registry: ferron_core::registry::RegistryBuilder,
    ) -> ferron_core::registry::RegistryBuilder {
        let channel = self.channel.0.clone();

        registry.with_provider::<ObservabilityContext, _>(move || {
            Arc::new(ConsoleObservabilityProvider {
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
            let module = Arc::new(ConsoleObservabilityModule {
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
        registry: &mut std::collections::HashMap<
            ferron_core::config::validator::ConfigurationValidatorScopedKey,
            Box<dyn ferron_core::config::validator::ConfigurationValidator>,
        >,
    ) {
        registry.insert(
            config_validator_scoped_key!("observability", "console"),
            Box::new(ConsoleObservabilityConfigValidator),
        );
    }
}

struct ConsoleObservabilityConfigValidator;

impl ConfigurationValidator for ConsoleObservabilityConfigValidator {
    fn validate_block(
        &self,
        config: &ferron_core::config::ServerConfigurationBlock,
        validator_ctx: &mut ferron_core::config::validator::ConfigurationValidatorContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Log format
        validate_scoped_block_flat(config, validator_ctx, "format", "logformat", Some("text"))?;

        Ok(())
    }
}
