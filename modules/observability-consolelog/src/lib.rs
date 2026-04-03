use std::sync::Arc;

use ferron_core::{
    loader::ModuleLoader, log_debug, log_error, log_info, log_warn, providers::Provider, Module,
};
use ferron_observability::{Event, ObservabilityContext};

struct ConsoleObservabilityModule {
    inner: kanal::AsyncReceiver<Event>,
    cancel_token: tokio_util::sync::CancellationToken,
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

        let rx = self.inner.clone();
        runtime.spawn_secondary_task(async move {
            while let Some(ev) = tokio::select! {
                result = rx.recv() => {
                    result.ok()
                }
                _ = cancel_token.cancelled() => {
                    None
                }
            } {
                tokio::task::spawn_blocking(move || {
                    match ev {
                        ferron_observability::Event::Access(ae) => log_info!("{}", ae.message),
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

struct ConsoleObservabilityProvider {
    inner: kanal::AsyncSender<Event>,
}

impl Provider<ObservabilityContext> for ConsoleObservabilityProvider {
    fn name(&self) -> &str {
        "console"
    }

    fn execute(&self, ctx: &mut ObservabilityContext) -> Result<(), Box<dyn std::error::Error>> {
        let _ = self.inner.try_send(ctx.event.clone());

        Ok(())
    }
}

pub struct ConsoleObservabilityModuleLoader {
    cache: Option<Arc<ConsoleObservabilityModule>>,
    channel: (kanal::AsyncSender<Event>, kanal::AsyncReceiver<Event>),
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
        let inner = self.channel.0.clone();
        registry.with_provider::<ObservabilityContext, _>(move || {
            Arc::new(ConsoleObservabilityProvider {
                inner: inner.clone(),
            })
        })
    }

    fn register_modules(
        &mut self,
        _registry: &ferron_core::registry::Registry,
        modules: &mut Vec<Arc<dyn ferron_core::Module>>,
        _config: &mut ferron_core::config::ServerConfiguration,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if self.cache.is_none() {
            let module = Arc::new(ConsoleObservabilityModule {
                inner: self.channel.1.clone(),
                cancel_token: tokio_util::sync::CancellationToken::new(),
            });

            self.cache = Some(module.clone());
            modules.push(module);
        }

        Ok(())
    }
}
