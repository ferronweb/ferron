mod client;
mod config;
mod providers;

use std::collections::HashMap;
use std::error::Error;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Once};

use ferron_core::{
    config::ServerConfigurationBlock,
    loader::ModuleLoader,
    log_warn,
    providers::Provider,
    registry::{Registry, RegistryBuilder},
    Module,
};
use ferron_observability::{Event, EventSink, ObservabilityContext};

use crate::config::OtlpBackendConfig;
use crate::providers::{emit_access_log, emit_log, emit_metric, emit_trace, OtlpProviderCache};

static DROPPED_EVENT: Once = Once::new();

/// Wrapper that carries an event with its configuration through the channel
struct ConfiguredEvent {
    event: Event,
    log_config: Arc<ServerConfigurationBlock>,
}

/// The OTLP event sink that emits events to an OTLP collector
struct OtlpEventSink {
    inner: async_channel::Sender<ConfiguredEvent>,
    log_config: Arc<ServerConfigurationBlock>,
}

impl EventSink for OtlpEventSink {
    fn emit(&self, event: Event) {
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
                        "Observability event dropped (`otlp` observability backend). \
                        This may be caused by high server load."
                    );
                });
            }
        }
    }

    fn emit_arc(&self, event: std::sync::Arc<Event>) {
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
                        "Observability event dropped (`otlp` observability backend). \
                        This may be caused by high server load."
                    );
                });
            }
        }
    }

    fn processes_traces(&self) -> bool {
        true
    }

    fn processes_access(&self) -> bool {
        true
    }
}

struct OtlpObservabilityModule {
    inner: async_channel::Receiver<ConfiguredEvent>,
    cancel_token: tokio_util::sync::CancellationToken,
    registry: Arc<Registry>,
}

impl Module for OtlpObservabilityModule {
    fn name(&self) -> &str {
        "observability-otlp"
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
        let registry = self.registry.clone();

        runtime.spawn_secondary_task(async move {
            // Per-config exporter cache
            let mut providers: HashMap<String, OtlpProviderCache> = HashMap::new();

            while let Some(msg) = tokio::select! {
                result = rx.recv() => result.ok(),
                _ = cancel_token.cancelled() => None,
            } {
                ferron_core::admin::ADMIN_METRICS
                    .observability_event_queue_len
                    .fetch_sub(1, Ordering::Relaxed);

                let config = OtlpBackendConfig::parse_config(&msg.log_config);

                let cache_key = config_cache_key(&config);
                let entry = providers
                    .entry(cache_key)
                    .or_insert_with(|| OtlpProviderCache::init(&config));

                match &msg.event {
                    Event::Log(log_event) => {
                        if let Some(ref provider) = entry.logs_provider {
                            emit_log(provider, log_event);
                        }
                    }
                    Event::Metric(metric_event) => {
                        if let Some(ref provider) = entry.metrics_provider {
                            emit_metric(provider, metric_event, &mut entry.metrics_instruments);
                        }
                    }
                    Event::Trace(trace_event) => {
                        if let Some(ref provider) = entry.traces_provider {
                            emit_trace(provider, trace_event, &entry.correlation);
                        }
                    }
                    Event::Access(access_event) => {
                        if let Some(ref provider) = entry.logs_provider {
                            emit_access_log(provider, access_event, &msg.log_config, &registry);
                        }
                    }
                }
            }

            // Shutdown providers
            // `tokio::task::spawn_blocking` is needed, because without it, there can be a deadlock.
            // See https://docs.rs/opentelemetry_sdk/latest/opentelemetry_sdk/trace/struct.BatchSpanProcessor.html
            tokio::task::spawn_blocking(move || {
                for (_, cache) in providers {
                    if let Some(p) = cache.logs_provider {
                        let _ = p.shutdown();
                    }
                    if let Some(p) = cache.metrics_provider {
                        let _ = p.shutdown();
                    }
                    if let Some(p) = cache.traces_provider {
                        let _ = p.shutdown();
                    }
                }
            });
        });

        Ok(())
    }
}

impl Drop for OtlpObservabilityModule {
    fn drop(&mut self) {
        self.cancel_token.cancel();
    }
}

/// Create a cache key from the signal configs
fn config_cache_key(config: &OtlpBackendConfig) -> String {
    let logs_key = config
        .logs
        .as_ref()
        .map(|s| {
            format!(
                "{}|{}|{}",
                s.endpoint,
                s.protocol,
                s.authorization.as_deref().unwrap_or("")
            )
        })
        .unwrap_or_default();
    let metrics_key = config
        .metrics
        .as_ref()
        .map(|s| {
            format!(
                "{}|{}|{}",
                s.endpoint,
                s.protocol,
                s.authorization.as_deref().unwrap_or("")
            )
        })
        .unwrap_or_default();
    let traces_key = config
        .traces
        .as_ref()
        .map(|s| {
            format!(
                "{}|{}|{}",
                s.endpoint,
                s.protocol,
                s.authorization.as_deref().unwrap_or("")
            )
        })
        .unwrap_or_default();
    format!(
        "{}|{}|{}|{}",
        config.service_name, logs_key, metrics_key, traces_key
    )
}

struct OtlpObservabilityProvider {
    inner: async_channel::Sender<ConfiguredEvent>,
}

impl Provider<ObservabilityContext> for OtlpObservabilityProvider {
    fn name(&self) -> &str {
        "otlp"
    }

    fn execute(&self, ctx: &mut ObservabilityContext) -> Result<(), Box<dyn Error>> {
        ctx.sink = Some(Arc::new(OtlpEventSink {
            inner: self.inner.clone(),
            log_config: ctx.log_config.clone(),
        }));
        Ok(())
    }
}

pub struct OtlpObservabilityModuleLoader {
    cache: Option<Arc<OtlpObservabilityModule>>,
    channel: (
        async_channel::Sender<ConfiguredEvent>,
        async_channel::Receiver<ConfiguredEvent>,
    ),
}

impl Default for OtlpObservabilityModuleLoader {
    fn default() -> Self {
        Self {
            cache: None,
            channel: async_channel::bounded(131072),
        }
    }
}

impl ModuleLoader for OtlpObservabilityModuleLoader {
    fn register_providers(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        let channel = self.channel.0.clone();

        registry.with_provider::<ObservabilityContext, _>(move || {
            Arc::new(OtlpObservabilityProvider {
                inner: channel.clone(),
            })
        })
    }

    fn register_modules(
        &mut self,
        registry: Arc<Registry>,
        modules: &mut Vec<Arc<dyn Module>>,
        _config: Arc<ferron_core::config::ServerConfiguration>,
    ) -> Result<(), Box<dyn Error>> {
        if self.cache.is_none() {
            let module = Arc::new(OtlpObservabilityModule {
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
mod tests;
