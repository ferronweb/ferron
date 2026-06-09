mod client;
mod config;
mod providers;
mod validator;

use std::collections::HashMap;
use std::error::Error;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Once};

use ferron_core::{
    config::ServerConfigurationBlock,
    config_validator_scoped_key,
    loader::ModuleLoader,
    log_warn,
    providers::Provider,
    registry::{Registry, RegistryBuilder},
    Module,
};
use ferron_observability::{
    build_composite_sink, CompositeEventSink, Event, EventSink, ObservabilityContext,
};

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
    has_logs: bool,
    has_metrics: bool,
    has_traces: bool,
}

impl EventSink for OtlpEventSink {
    #[inline]
    fn emit(&self, event: Event) {
        let emit = match event {
            Event::Access(_) | Event::Log(_) => self.has_logs,
            Event::Metric(_) => self.has_metrics,
            Event::Trace(_) => self.has_traces,
        };
        if emit {
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
    }

    #[inline]
    fn emit_arc(&self, event: std::sync::Arc<Event>) {
        let emit = match &*event {
            Event::Access(_) | Event::Log(_) => self.has_logs,
            Event::Metric(_) => self.has_metrics,
            Event::Trace(_) => self.has_traces,
        };
        if emit {
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
    }

    #[inline]
    fn processes_traces(&self) -> bool {
        self.has_traces
    }

    #[inline]
    fn processes_access(&self) -> bool {
        self.has_logs
    }
}

struct OtlpObservabilityModule {
    inner: async_channel::Receiver<ConfiguredEvent>,
    cancel_token: tokio_util::sync::CancellationToken,
    registry: Arc<Registry>,
    event_sink: Option<Arc<CompositeEventSink>>,
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
        let event_sink = self.event_sink.clone();

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
                    .or_insert_with(|| OtlpProviderCache::init(&config, event_sink.as_deref()));

                match &msg.event {
                    Event::Log(log_event) => {
                        if let Some(ref provider) = entry.logs_provider {
                            emit_log(
                                provider,
                                log_event,
                                &entry.baggage_promotions,
                                config.log_style,
                            );
                        }
                    }
                    Event::Metric(metric_event) => {
                        if let Some(ref provider) = entry.metrics_provider {
                            emit_metric(
                                provider,
                                metric_event,
                                &mut entry.metrics_instruments,
                                &entry.baggage_promotions,
                                &mut entry.baggage_tracker,
                            );
                        }
                    }
                    Event::Trace(trace_event) => {
                        if let Some(ref provider) = entry.traces_provider {
                            emit_trace(
                                provider,
                                trace_event,
                                &entry.correlation,
                                &entry.baggage_promotions,
                            );
                        }
                    }
                    Event::Access(access_event) => {
                        if let Some(ref provider) = entry.logs_provider {
                            emit_access_log(
                                provider,
                                access_event,
                                &msg.log_config,
                                &registry,
                                &entry.baggage_promotions,
                                config.log_style,
                            );
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
            let sampling_key = format!("{:?}", s.sampling.mode);
            format!(
                "{}|{}|{}|{}",
                s.endpoint,
                s.protocol,
                s.authorization.as_deref().unwrap_or(""),
                sampling_key,
            )
        })
        .unwrap_or_default();
    format!(
        "{}|{}|{}|{}|{}",
        config.service_name,
        logs_key,
        metrics_key,
        traces_key,
        match config.log_style {
            crate::config::LogStyle::Legacy => "legacy",
            crate::config::LogStyle::Modern => "modern",
        }
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
        // Heuristics based on configuration directives
        let has_logs = ctx.log_config.has_directive("logs");
        let has_metrics = ctx.log_config.has_directive("metrics");
        let has_traces = ctx.log_config.has_directive("traces");

        ctx.sink = Some(Arc::new(OtlpEventSink {
            inner: self.inner.clone(),
            log_config: ctx.log_config.clone(),
            has_logs,
            has_metrics,
            has_traces,
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
        config: Arc<ferron_core::config::ServerConfiguration>,
    ) -> Result<(), Box<dyn Error>> {
        if self.cache.is_none() {
            let event_sink = build_composite_sink(&registry, &config.global_config).ok();

            let module = Arc::new(OtlpObservabilityModule {
                inner: self.channel.1.clone(),
                cancel_token: tokio_util::sync::CancellationToken::new(),
                registry: registry.clone(),
                event_sink,
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
            config_validator_scoped_key!("observability", "otlp"),
            Box::new(validator::OtlpObservabilityConfigurationValidator),
        );
    }
}

#[cfg(test)]
mod tests;
