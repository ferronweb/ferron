use std::sync::Arc;

use ferron_core::{
    config::ServerConfigurationBlock, loader::ModuleLoader, log_debug, log_error, log_info,
    log_warn, providers::Provider, registry::Registry, Module,
};
use ferron_observability::{AccessEvent, Event, LogFormatterContext, ObservabilityContext};

/// Wrapper that carries an event with its configuration through the channel
struct ConfiguredEvent {
    event: Event,
    log_config: Arc<ServerConfigurationBlock>,
}

struct ConsoleObservabilityModule {
    inner: kanal::AsyncReceiver<ConfiguredEvent>,
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
                let registry = registry.clone();
                tokio::task::spawn_blocking(move || {
                    match msg.event {
                        ferron_observability::Event::Access(ae) => {
                            let message = format_access_event(&ae, &msg.log_config, &registry);
                            if let Some(message) = message {
                                log_info!("{}", message);
                            }
                        }
                        ferron_observability::Event::Log(le) => match le.level {
                            ferron_observability::LogLevel::Error => log_error!("{}", le.message),
                            ferron_observability::LogLevel::Warn => log_warn!("{}", le.message),
                            ferron_observability::LogLevel::Info => log_info!("{}", le.message),
                            ferron_observability::LogLevel::Debug => log_debug!("{}", le.message),
                        },
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
    // Check if a formatter is specified in the config
    if let Some(formatter_name) = log_config.get_value("format").and_then(|v| v.as_str()) {
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
    }

    None
}

struct ConsoleObservabilityProvider {
    inner: kanal::AsyncSender<ConfiguredEvent>,
}

impl Provider<ObservabilityContext> for ConsoleObservabilityProvider {
    fn name(&self) -> &str {
        "console"
    }

    fn execute(&self, ctx: &mut ObservabilityContext) -> Result<(), Box<dyn std::error::Error>> {
        let _ = self.inner.try_send(ConfiguredEvent {
            event: ctx.event.clone(),
            log_config: ctx.log_config.clone(),
        });
        Ok(())
    }
}

pub struct ConsoleObservabilityModuleLoader {
    cache: Option<Arc<ConsoleObservabilityModule>>,
    channel: (
        kanal::AsyncSender<ConfiguredEvent>,
        kanal::AsyncReceiver<ConfiguredEvent>,
    ),
}

impl Default for ConsoleObservabilityModuleLoader {
    fn default() -> Self {
        Self {
            cache: None,
            channel: kanal::unbounded_async(),
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
        _config: &mut ferron_core::config::ServerConfiguration,
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
}
