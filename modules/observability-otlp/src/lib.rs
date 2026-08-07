mod config;
mod validator;

// New OTLP exporter logic
mod pipeline;
pub mod proto;

// TODO: wire the remaining transports into the module event loop
// (traces and logs pipelines are wired; metrics is wired in pipeline step 5)
#[allow(dead_code)]
mod transport;

mod convert;

use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Once};
use std::time::SystemTime;

use ferron_core::config::ServerConfigurationBlock;
use ferron_core::loader::ModuleLoader;
use ferron_core::providers::Provider;
use ferron_core::registry::{Registry, RegistryBuilder};
use ferron_core::{config_validator_scoped_key, log_warn, Module};
use ferron_observability::baggage::{BaggageKeyPromotion, DistinctValueTracker};
use ferron_observability::{
    build_composite_sink, CompositeEventSink, Event, EventSink, LogAttributeValue, LogEvent,
    LogLevel, ObservabilityContext, TraceEvent,
};

use crate::config::{OtlpBackendConfig, SignalConfig};
use crate::convert::{
    build_access_log_record, build_log_record, end_span, start_span, CorrelationContext,
};
use crate::pipeline::logs::{LogExporter, LogPipeline};
use crate::pipeline::metrics::{MetricExporter, MetricPipeline};
use crate::pipeline::traces::{TraceExporter, TracePipeline};
use crate::pipeline::{
    BatchConfig, DEFAULT_BATCH_SIZE, DEFAULT_EXPORT_TIMEOUT, DEFAULT_FLUSH_INTERVAL,
    DEFAULT_READ_INTERVAL,
};
use crate::transport::client::OtlpTransport;

static DROPPED_EVENT: Once = Once::new();

/// Wrapper that carries an event with its configuration through the channel
struct ConfiguredEvent {
    event: Arc<Event>,
    log_config: Arc<ServerConfigurationBlock>,
    control_plane_metadata: Option<Arc<BTreeMap<String, String>>>,
}

/// The OTLP event sink that emits events to an OTLP collector
struct OtlpEventSink {
    inner: async_channel::Sender<ConfiguredEvent>,
    log_config: Arc<ServerConfigurationBlock>,
    has_logs: bool,
    has_metrics: bool,
    has_traces: bool,
    control_plane_metadata: Option<Arc<BTreeMap<String, String>>>,
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
                event: Arc::new(event),
                log_config: self.log_config.clone(),
                control_plane_metadata: self.control_plane_metadata.clone(),
            }) {
                Ok(_) => {
                    ferron_core::admin::ADMIN_METRICS
                        .observability_event_queue_len
                        .fetch_add(1, Ordering::Relaxed);
                }
                Err(_) => {
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
                event,
                log_config: self.log_config.clone(),
                control_plane_metadata: self.control_plane_metadata.clone(),
            }) {
                Ok(_) => {
                    ferron_core::admin::ADMIN_METRICS
                        .observability_event_queue_len
                        .fetch_add(1, Ordering::Relaxed);
                }
                Err(_) => {
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

/// Per-config state for the trace pipeline: the exporter handle, the
/// correlation context for parent resolution, and the config-derived
/// attributes the conversion needs.
struct TracePipelineEntry {
    pipeline: Option<TracePipeline>,
    correlation: CorrelationContext,
    baggage_promotions: Vec<BaggageKeyPromotion>,
    control_plane_metadata: Option<Arc<BTreeMap<String, String>>>,
}

impl TracePipelineEntry {
    fn init(
        config: &OtlpBackendConfig,
        control_plane_metadata: Option<Arc<BTreeMap<String, String>>>,
        cancel_token: tokio_util::sync::CancellationToken,
        event_sink: Option<&CompositeEventSink>,
    ) -> Self {
        let pipeline = if let Some(signal) = &config.traces {
            match OtlpTransport::from_config(config) {
                Ok(transport) => {
                    let exporter: Arc<dyn TraceExporter> = Arc::new(transport);
                    Some(TracePipeline::spawn_with_config(
                        exporter,
                        config.service_name.clone(),
                        cancel_token,
                        batch_config(signal),
                    ))
                }
                Err(err) => {
                    if let Some(sink) = event_sink {
                        sink.emit(Event::Log(LogEvent {
                            level: LogLevel::Warn,
                            message: format!("Error with traces pipeline: {err}"),
                            summary: "Error with traces pipeline".into(),
                            target: "ferron-observability-otlp",
                            attributes: vec![(
                                "error.message",
                                LogAttributeValue::String(err.to_string()),
                            )],
                            trace_context: None,
                        }));
                    }
                    None
                }
            }
        } else {
            None
        };
        Self {
            pipeline,
            correlation: CorrelationContext::new(),
            baggage_promotions: config.baggage_promotions.clone(),
            control_plane_metadata,
        }
    }
}

/// Per-config state for the log pipeline: the exporter handle and the
/// config-derived values the conversion needs.
struct LogPipelineEntry {
    pipeline: Option<LogPipeline>,
    baggage_promotions: Vec<BaggageKeyPromotion>,
    control_plane_metadata: Option<Arc<BTreeMap<String, String>>>,
}

impl LogPipelineEntry {
    fn init(
        config: &OtlpBackendConfig,
        control_plane_metadata: Option<Arc<BTreeMap<String, String>>>,
        cancel_token: tokio_util::sync::CancellationToken,
        event_sink: Option<&CompositeEventSink>,
    ) -> Self {
        let pipeline = if let Some(signal) = &config.logs {
            match OtlpTransport::from_config(config) {
                Ok(transport) => {
                    let exporter: Arc<dyn LogExporter> = Arc::new(transport);
                    Some(LogPipeline::spawn_with_config(
                        exporter,
                        config.service_name.clone(),
                        cancel_token,
                        batch_config(signal),
                    ))
                }
                Err(err) => {
                    if let Some(sink) = event_sink {
                        sink.emit(Event::Log(LogEvent {
                            level: LogLevel::Warn,
                            message: format!("Error with logs pipeline: {err}"),
                            summary: "Error with logs pipeline".into(),
                            target: "ferron-observability-otlp",
                            attributes: vec![(
                                "error.message",
                                LogAttributeValue::String(err.to_string()),
                            )],
                            trace_context: None,
                        }));
                    }
                    None
                }
            }
        } else {
            None
        };
        Self {
            pipeline,
            baggage_promotions: config.baggage_promotions.clone(),
            control_plane_metadata,
        }
    }
}

/// Per-config state for the metric pipeline: the reader handle, the baggage
/// promotions, and the cardinality-limiting tracker shared by all series.
struct MetricPipelineEntry {
    pipeline: Option<MetricPipeline>,
    baggage_promotions: Vec<BaggageKeyPromotion>,
    baggage_tracker: DistinctValueTracker,
}

impl MetricPipelineEntry {
    fn init(
        config: &OtlpBackendConfig,
        cancel_token: tokio_util::sync::CancellationToken,
        event_sink: Option<&CompositeEventSink>,
    ) -> Self {
        let pipeline = if let Some(signal) = &config.metrics {
            match OtlpTransport::from_config(config) {
                Ok(transport) => {
                    let exporter: Arc<dyn MetricExporter> = Arc::new(transport);
                    Some(MetricPipeline::spawn_with_config(
                        exporter,
                        config.service_name.clone(),
                        cancel_token,
                        signal.read_interval.unwrap_or(DEFAULT_READ_INTERVAL),
                        DEFAULT_EXPORT_TIMEOUT,
                    ))
                }
                Err(err) => {
                    if let Some(sink) = event_sink {
                        sink.emit(Event::Log(LogEvent {
                            level: LogLevel::Warn,
                            message: format!("Error with metrics pipeline: {err}"),
                            summary: "Error with metrics pipeline".into(),
                            target: "ferron-observability-otlp",
                            attributes: vec![(
                                "error.message",
                                LogAttributeValue::String(err.to_string()),
                            )],
                            trace_context: None,
                        }));
                    }
                    None
                }
            }
        } else {
            None
        };
        Self {
            pipeline,
            baggage_promotions: config.baggage_promotions.clone(),
            baggage_tracker: DistinctValueTracker::new(),
        }
    }
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
            // Per-config trace pipelines
            let mut trace_pipelines: HashMap<String, TracePipelineEntry> = HashMap::new();
            // Per-config log pipelines
            let mut log_pipelines: HashMap<String, LogPipelineEntry> = HashMap::new();
            // Per-config metric pipelines
            let mut metric_pipelines: HashMap<String, MetricPipelineEntry> = HashMap::new();

            while let Some(msg) = tokio::select! {
                result = rx.recv() => result.ok(),
                _ = cancel_token.cancelled() => None,
            } {
                ferron_core::admin::ADMIN_METRICS
                    .observability_event_queue_len
                    .fetch_sub(1, Ordering::Relaxed);

                let config = OtlpBackendConfig::parse_config(&msg.log_config);

                let cache_key = config_cache_key(&config);
                let trace = trace_pipelines.entry(cache_key.clone()).or_insert_with(|| {
                    TracePipelineEntry::init(
                        &config,
                        msg.control_plane_metadata.clone(),
                        cancel_token.clone(),
                        event_sink.as_deref(),
                    )
                });
                let log = log_pipelines.entry(cache_key.clone()).or_insert_with(|| {
                    LogPipelineEntry::init(
                        &config,
                        msg.control_plane_metadata.clone(),
                        cancel_token.clone(),
                        event_sink.as_deref(),
                    )
                });
                let metrics = metric_pipelines
                    .entry(cache_key.clone())
                    .or_insert_with(|| {
                        MetricPipelineEntry::init(
                            &config,
                            cancel_token.clone(),
                            event_sink.as_deref(),
                        )
                    });

                match &*msg.event {
                    Event::Log(log_event) => {
                        if let Some(pipeline) = &log.pipeline {
                            let record = build_log_record(
                                log_event,
                                &log.baggage_promotions,
                                config.log_style,
                                SystemTime::now(),
                            );
                            pipeline.buffer.push("ferron", record);
                        }
                    }
                    Event::Metric(metric_event) => {
                        if let Some(metric) = &metrics.pipeline {
                            metric.store.record(
                                metric_event,
                                &metrics.baggage_promotions,
                                &mut metrics.baggage_tracker,
                            );
                        }
                    }
                    Event::Trace(trace_event) => {
                        let now = SystemTime::now();
                        let finished = match trace_event {
                            TraceEvent::StartSpan { .. } => start_span(
                                trace_event,
                                &mut trace.correlation,
                                &trace.baggage_promotions,
                                &trace.control_plane_metadata,
                                now,
                            ),
                            TraceEvent::EndSpan { .. } => {
                                end_span(trace_event, &mut trace.correlation, now)
                            }
                        };
                        if let Some(span) = finished {
                            if let Some(pipeline) = &trace.pipeline {
                                pipeline.buffer.push(span);
                            }
                        }
                    }
                    Event::Access(access_event) => {
                        if let Some(pipeline) = &log.pipeline {
                            let record = build_access_log_record(
                                access_event,
                                &msg.log_config,
                                &registry,
                                &log.baggage_promotions,
                                config.log_style,
                                &log.control_plane_metadata,
                                SystemTime::now(),
                            );
                            pipeline.buffer.push("ferron.access", record);
                        }
                    }
                }
            }

            // Shutdown trace pipelines: flush remaining spans.
            cancel_token.cancel();
            let trace_pipelines: Vec<TracePipeline> = trace_pipelines
                .into_values()
                .filter_map(|entry| entry.pipeline)
                .collect();
            for pipeline in trace_pipelines {
                pipeline.wait_done().await;
            }

            // Shutdown log pipelines: flush remaining records.
            let log_pipelines: Vec<LogPipeline> = log_pipelines
                .into_values()
                .filter_map(|entry| entry.pipeline)
                .collect();
            for pipeline in log_pipelines {
                pipeline.wait_done().await;
            }

            // Shutdown metric pipelines: one final collection.
            let metric_pipelines: Vec<MetricPipeline> = metric_pipelines
                .into_values()
                .filter_map(|entry| entry.pipeline)
                .collect();
            for pipeline in metric_pipelines {
                pipeline.wait_done().await;
            }
        });

        Ok(())
    }
}

impl Drop for OtlpObservabilityModule {
    fn drop(&mut self) {
        self.cancel_token.cancel();
    }
}

/// Batch tuning for a logs/traces signal, falling back to the SDK-default
/// values for anything the configuration does not override.
fn batch_config(signal: &SignalConfig) -> BatchConfig {
    BatchConfig {
        batch_size: signal.export_batch_size.unwrap_or(DEFAULT_BATCH_SIZE),
        interval: signal.export_interval.unwrap_or(DEFAULT_FLUSH_INTERVAL),
        ..BatchConfig::default()
    }
}

/// Create a cache key from the signal configs
fn config_cache_key(config: &OtlpBackendConfig) -> String {
    let logs_key = config
        .logs
        .as_ref()
        .map(|s| {
            format!(
                "{}|{}|{}|{:?}|{:?}",
                s.endpoint,
                s.protocol,
                s.authorization.as_deref().unwrap_or(""),
                s.export_interval,
                s.export_batch_size
            )
        })
        .unwrap_or_default();
    let metrics_key = config
        .metrics
        .as_ref()
        .map(|s| {
            format!(
                "{}|{}|{}|{:?}",
                s.endpoint,
                s.protocol,
                s.authorization.as_deref().unwrap_or(""),
                s.read_interval
            )
        })
        .unwrap_or_default();
    let traces_key = config
        .traces
        .as_ref()
        .map(|s| {
            format!(
                "{}|{}|{}|{:?}|{:?}",
                s.endpoint,
                s.protocol,
                s.authorization.as_deref().unwrap_or(""),
                s.export_interval,
                s.export_batch_size
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
            control_plane_metadata: ctx.control_plane_metadata.clone(),
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
            let event_sink = build_composite_sink(&registry, &config.global_config, None).ok();

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

    fn register_directives(&mut self, registry: &mut ferron_core::directives::DirectiveRegistry) {
        use ferron_core::directives::{Directive, DirectiveSubblock};
        registry
            .register(
                Directive {
                    name: "format",
                    usage: "format { ... }",
                    description: "This directive configures the log format for the OTLP provider. Delegates to a log format sub-block (text or json).",
                    applicable_protocols: None,
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("observability"),
            )
            .register(
                Directive {
                    name: "logs",
                    usage: "logs <endpoint>",
                    description: "This directive specifies the OTLP logs endpoint. Contains optional protocol, authorization, export_interval, export_batch_size, and gzip sub-directives.",
                    applicable_protocols: None,
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("observability"),
            )
            .register(
                Directive {
                    name: "metrics",
                    usage: "metrics <endpoint>",
                    description: "This directive specifies the OTLP metrics endpoint. Contains optional protocol, authorization, read_interval, gzip, native_histograms, and exemplars sub-directives.",
                    applicable_protocols: None,
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("observability"),
            )
            .register(
                Directive {
                    name: "traces",
                    usage: "traces <endpoint>",
                    description: "This directive specifies the OTLP traces endpoint. Contains optional protocol, authorization, export_interval, export_batch_size, and gzip sub-directives.",
                    applicable_protocols: None,
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("observability"),
            )
            .register(
                Directive {
                    name: "service_name",
                    usage: "service_name <name>",
                    description: "This directive specifies the service name for OTLP telemetry data.",
                    applicable_protocols: None,
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("observability"),
            )
            .register(
                Directive {
                    name: "no_verification",
                    usage: "no_verification",
                    description: "This directive disables TLS certificate verification for OTLP endpoints.",
                    applicable_protocols: None,
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("observability"),
            )
            .register(
                Directive {
                    name: "log_style",
                    usage: "log_style <style>",
                    description: "This directive specifies the log style for OTLP. Supported: legacy, modern.",
                    applicable_protocols: None,
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("observability"),
            )
            .register(
                Directive {
                    name: "baggage",
                    usage: "baggage { ... }",
                    description: "This directive configures baggage key promotion for OTLP. Contains key blocks with attribute, signals, and max_distinct.",
                    applicable_protocols: None,
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("observability"),
            );
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
