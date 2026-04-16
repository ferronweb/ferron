mod endpoint;

use std::collections::HashMap;
use std::error::Error;
use std::net::SocketAddr;
use std::sync::{Arc, Once};

use ferron_core::{
    config::ServerConfigurationBlock,
    loader::ModuleLoader,
    log_warn,
    providers::Provider,
    registry::{Registry, RegistryBuilder},
    Module,
};
use ferron_observability::{
    Event, EventSink, MetricAttributeValue, MetricEvent, MetricType, MetricValue,
    ObservabilityContext,
};
use tokio_util::sync::CancellationToken;

use crate::endpoint::endpoint_listener_fn;

type PrometheusInstrumentCache =
    HashMap<(&'static str, Vec<(&'static str, String)>), CachedInstrument>;
static DROPPED_EVENT: Once = Once::new();

/// Shared configuration for an Prometheus backend instance
#[allow(dead_code)]
#[derive(Clone)]
struct PrometheusBackendConfig {
    listen: SocketAddr,
    format: String,
}

/// Wrapper that carries an event with its configuration through the channel
struct ConfiguredEvent {
    event: Event,
    log_config: Arc<ServerConfigurationBlock>,
}

/// The Prometheus event sink that emits events to an Prometheus collector
struct PrometheusEventSink {
    inner: async_channel::Sender<ConfiguredEvent>,
    log_config: Arc<ServerConfigurationBlock>,
}

impl EventSink for PrometheusEventSink {
    fn emit(&self, event: Event) {
        if matches!(event, Event::Metric(_))
            && self
                .inner
                .try_send(ConfiguredEvent {
                    event,
                    log_config: self.log_config.clone(),
                })
                .is_err()
        {
            DROPPED_EVENT.call_once(|| {
                log_warn!(
                    "Observability event dropped (`prometheus` observability backend). \
                    This may be caused by high server load."
                )
            });
        }
    }

    fn emit_arc(&self, event: std::sync::Arc<Event>) {
        if matches!(&*event, Event::Metric(_))
            && self
                .inner
                .try_send(ConfiguredEvent {
                    event: Arc::unwrap_or_clone(event),
                    log_config: self.log_config.clone(),
                })
                .is_err()
        {
            DROPPED_EVENT.call_once(|| {
                log_warn!(
                    "Observability event dropped (`prometheus` observability backend). \
                    This may be caused by high server load."
                )
            });
        }
    }
}

/// Parse the Prometheus backend configuration from a ServerConfigurationBlock
fn parse_prometheus_config(
    config: &ServerConfigurationBlock,
) -> Result<PrometheusBackendConfig, Box<dyn Error>> {
    let listen = config
        .get_value("endpoint_listen")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<SocketAddr>().ok())
        .unwrap_or_else(|| "127.0.0.1:8889".parse().expect("default listen address"));

    let format = config
        .get_value("endpoint_format")
        .and_then(|v| v.as_str())
        .unwrap_or("text")
        .to_string();

    Ok(PrometheusBackendConfig { listen, format })
}

struct PrometheusObservabilityModule {
    inner: async_channel::Receiver<ConfiguredEvent>,
    cancel_token: tokio_util::sync::CancellationToken,
}

impl Module for PrometheusObservabilityModule {
    fn name(&self) -> &str {
        "observability-prometheus"
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
            // Per-config exporter cache
            let mut providers: HashMap<String, PrometheusProviderCache> = HashMap::new();

            while let Some(msg) = tokio::select! {
                result = rx.recv() => result.ok(),
                _ = cancel_token.cancelled() => None,
            } {
                let config = match parse_prometheus_config(&msg.log_config) {
                    Ok(c) => c,
                    Err(e) => {
                        ferron_core::log_error!("Failed to parse Prometheus config: {}", e);
                        continue;
                    }
                };

                let cache_key = config_cache_key(&config);
                let entry = providers
                    .entry(cache_key)
                    .or_insert_with(|| init_provider(&config, cancel_token.clone()));

                if let Event::Metric(metric_event) = &msg.event {
                    emit_metric(
                        &entry.registry,
                        metric_event,
                        &mut entry.metrics_instruments,
                    );
                }
            }
        });

        Ok(())
    }
}

/// Cached Prometheus providers for a given config
struct PrometheusProviderCache {
    registry: prometheus::Registry,
    metrics_instruments: PrometheusInstrumentCache,
}

enum CachedInstrument {
    F64Counter(prometheus::core::GenericCounter<prometheus::core::AtomicF64>),
    F64Gauge(prometheus::core::GenericGauge<prometheus::core::AtomicF64>),
    F64Histogram(prometheus::Histogram),
    // F64UpDownCounter would be gauge
    I64Gauge(prometheus::core::GenericGauge<prometheus::core::AtomicI64>),
    // I64UpDownCounter would be gauge
    U64Counter(prometheus::core::GenericCounter<prometheus::core::AtomicU64>),
    U64Gauge(prometheus::core::GenericGauge<prometheus::core::AtomicU64>),
    // U64Histogram would be F64Histogram...
}

/// Create a cache key from the signal configs
fn config_cache_key(config: &PrometheusBackendConfig) -> String {
    format!("{}|{}", config.listen, config.format)
}

fn init_provider(
    config: &PrometheusBackendConfig,
    reload_token: CancellationToken,
) -> PrometheusProviderCache {
    let config = config.clone();
    let registry = prometheus::Registry::new();
    let registry_clone = registry.clone();
    // Note: Prometheus endpoint listener is spawned on demand when the first event
    // with a given config is received. This allows us to avoid starting unnecessary listeners
    // for configs that are never used, but also means that the first event may be delayed
    // while the listener is starting up.
    tokio::spawn(async move {
        let socket_addr = config.listen;
        if let Err(err) = endpoint_listener_fn(config, reload_token, registry_clone).await {
            ferron_core::log_warn!("Prometheus endpoint listener at {socket_addr} failed: {err}");
        }
    });
    PrometheusProviderCache {
        registry,
        metrics_instruments: HashMap::new(),
    }
}

fn emit_metric(
    registry: &prometheus::Registry,
    event: &MetricEvent,
    instruments: &mut PrometheusInstrumentCache,
) {
    let attrs: Vec<(&'static str, String)> = event
        .attributes
        .iter()
        .map(|(k, v)| {
            (
                *k,
                match v {
                    MetricAttributeValue::F64(val) => val.to_string(),
                    MetricAttributeValue::I64(val) => val.to_string(),
                    MetricAttributeValue::String(val) => val.to_owned(),
                    MetricAttributeValue::StaticStr(val) => val.to_string(),
                    MetricAttributeValue::Bool(val) => {
                        if *val {
                            "1".to_string()
                        } else {
                            "0".to_string()
                        }
                    }
                },
            )
        })
        .collect();

    match (&event.ty, event.value) {
        (MetricType::Counter, MetricValue::F64(val)) => {
            let instrument_entry = instruments.entry((event.name, attrs.clone()));
            let instrument = match instrument_entry {
                std::collections::hash_map::Entry::Occupied(ref e) => Some(e.get()),
                std::collections::hash_map::Entry::Vacant(e) => {
                    let collector =
                        prometheus::core::GenericCounter::<prometheus::core::AtomicF64>::with_opts(
                            prometheus::Opts {
                                namespace: String::new(),
                                subsystem: String::new(),
                                name: event.name.to_string().replace(".", "_"),
                                help: event
                                    .description
                                    .unwrap_or("No description provided")
                                    .to_string(),
                                const_labels: attrs
                                    .iter()
                                    .map(|(k, v)| (k.replace(".", "_"), v.clone()))
                                    .collect(),
                                variable_labels: Vec::new(),
                            },
                        );
                    if let Ok(collector) = collector {
                        let _ = registry.register(Box::new(collector.clone()));
                        Some(e.insert(CachedInstrument::F64Counter(collector)) as &CachedInstrument)
                    } else {
                        None
                    }
                }
            };
            if let Some(CachedInstrument::F64Counter(i)) = instrument {
                if val >= 0.0 {
                    i.inc_by(val);
                }
            }
        }
        (MetricType::Counter, MetricValue::U64(val)) => {
            let instrument_entry = instruments.entry((event.name, attrs.clone()));
            let instrument = match instrument_entry {
                std::collections::hash_map::Entry::Occupied(ref e) => Some(e.get()),
                std::collections::hash_map::Entry::Vacant(e) => {
                    let collector =
                        prometheus::core::GenericCounter::<prometheus::core::AtomicU64>::with_opts(
                            prometheus::Opts {
                                namespace: String::new(),
                                subsystem: String::new(),
                                name: event.name.to_string().replace(".", "_"),
                                help: event
                                    .description
                                    .unwrap_or("No description provided")
                                    .to_string(),
                                const_labels: attrs
                                    .iter()
                                    .map(|(k, v)| (k.replace(".", "_"), v.clone()))
                                    .collect(),
                                variable_labels: Vec::new(),
                            },
                        );
                    if let Ok(collector) = collector {
                        let _ = registry.register(Box::new(collector.clone()));
                        Some(e.insert(CachedInstrument::U64Counter(collector)) as &CachedInstrument)
                    } else {
                        None
                    }
                }
            };
            if let Some(CachedInstrument::U64Counter(i)) = instrument {
                i.inc_by(val);
            }
        }
        (MetricType::UpDownCounter, MetricValue::F64(val)) => {
            let instrument_entry = instruments.entry((event.name, attrs.clone()));
            let instrument = match instrument_entry {
                std::collections::hash_map::Entry::Occupied(ref e) => Some(e.get()),
                std::collections::hash_map::Entry::Vacant(e) => {
                    let collector =
                        prometheus::core::GenericCounter::<prometheus::core::AtomicU64>::with_opts(
                            prometheus::Opts {
                                namespace: String::new(),
                                subsystem: String::new(),
                                name: event.name.to_string().replace(".", "_"),
                                help: event
                                    .description
                                    .unwrap_or("No description provided")
                                    .to_string(),
                                const_labels: attrs
                                    .iter()
                                    .map(|(k, v)| (k.replace(".", "_"), v.clone()))
                                    .collect(),
                                variable_labels: Vec::new(),
                            },
                        );
                    if let Ok(collector) = collector {
                        let _ = registry.register(Box::new(collector.clone()));
                        Some(e.insert(CachedInstrument::U64Counter(collector)) as &CachedInstrument)
                    } else {
                        None
                    }
                }
            };
            if let Some(CachedInstrument::F64Gauge(i)) = instrument {
                i.add(val);
            }
        }
        (MetricType::UpDownCounter, MetricValue::I64(val)) => {
            let instrument_entry = instruments.entry((event.name, attrs.clone()));
            let instrument = match instrument_entry {
                std::collections::hash_map::Entry::Occupied(ref e) => Some(e.get()),
                std::collections::hash_map::Entry::Vacant(e) => {
                    let collector =
                        prometheus::core::GenericGauge::<prometheus::core::AtomicI64>::with_opts(
                            prometheus::Opts {
                                namespace: String::new(),
                                subsystem: String::new(),
                                name: event.name.to_string().replace(".", "_"),
                                help: event
                                    .description
                                    .unwrap_or("No description provided")
                                    .to_string(),
                                const_labels: attrs
                                    .iter()
                                    .map(|(k, v)| (k.replace(".", "_"), v.clone()))
                                    .collect(),
                                variable_labels: Vec::new(),
                            },
                        );
                    if let Ok(collector) = collector {
                        let _ = registry.register(Box::new(collector.clone()));
                        Some(e.insert(CachedInstrument::I64Gauge(collector)) as &CachedInstrument)
                    } else {
                        None
                    }
                }
            };
            if let Some(CachedInstrument::I64Gauge(i)) = instrument {
                i.add(val);
            }
        }
        (MetricType::Gauge, MetricValue::F64(val)) => {
            let instrument_entry = instruments.entry((event.name, attrs.clone()));
            let instrument = match instrument_entry {
                std::collections::hash_map::Entry::Occupied(ref e) => Some(e.get()),
                std::collections::hash_map::Entry::Vacant(e) => {
                    let collector =
                        prometheus::core::GenericGauge::<prometheus::core::AtomicF64>::with_opts(
                            prometheus::Opts {
                                namespace: String::new(),
                                subsystem: String::new(),
                                name: event.name.to_string().replace(".", "_"),
                                help: event
                                    .description
                                    .unwrap_or("No description provided")
                                    .to_string(),
                                const_labels: attrs
                                    .iter()
                                    .map(|(k, v)| (k.replace(".", "_"), v.clone()))
                                    .collect(),
                                variable_labels: Vec::new(),
                            },
                        );
                    if let Ok(collector) = collector {
                        let _ = registry.register(Box::new(collector.clone()));
                        Some(e.insert(CachedInstrument::F64Gauge(collector)) as &CachedInstrument)
                    } else {
                        None
                    }
                }
            };
            if let Some(CachedInstrument::F64Gauge(i)) = instrument {
                i.set(val);
            }
        }
        (MetricType::Gauge, MetricValue::I64(val)) => {
            let instrument_entry = instruments.entry((event.name, attrs.clone()));
            let instrument = match instrument_entry {
                std::collections::hash_map::Entry::Occupied(ref e) => Some(e.get()),
                std::collections::hash_map::Entry::Vacant(e) => {
                    let collector =
                        prometheus::core::GenericGauge::<prometheus::core::AtomicI64>::with_opts(
                            prometheus::Opts {
                                namespace: String::new(),
                                subsystem: String::new(),
                                name: event.name.to_string().replace(".", "_"),
                                help: event
                                    .description
                                    .unwrap_or("No description provided")
                                    .to_string(),
                                const_labels: attrs
                                    .iter()
                                    .map(|(k, v)| (k.replace(".", "_"), v.clone()))
                                    .collect(),
                                variable_labels: Vec::new(),
                            },
                        );
                    if let Ok(collector) = collector {
                        let _ = registry.register(Box::new(collector.clone()));
                        Some(e.insert(CachedInstrument::I64Gauge(collector)) as &CachedInstrument)
                    } else {
                        None
                    }
                }
            };
            if let Some(CachedInstrument::I64Gauge(i)) = instrument {
                i.set(val);
            }
        }
        (MetricType::Gauge, MetricValue::U64(val)) => {
            let instrument_entry = instruments.entry((event.name, attrs.clone()));
            let instrument = match instrument_entry {
                std::collections::hash_map::Entry::Occupied(ref e) => Some(e.get()),
                std::collections::hash_map::Entry::Vacant(e) => {
                    let collector =
                        prometheus::core::GenericGauge::<prometheus::core::AtomicU64>::with_opts(
                            prometheus::Opts {
                                namespace: String::new(),
                                subsystem: String::new(),
                                name: event.name.to_string().replace(".", "_"),
                                help: event
                                    .description
                                    .unwrap_or("No description provided")
                                    .to_string(),
                                const_labels: attrs
                                    .iter()
                                    .map(|(k, v)| (k.replace(".", "_"), v.clone()))
                                    .collect(),
                                variable_labels: Vec::new(),
                            },
                        );
                    if let Ok(collector) = collector {
                        let _ = registry.register(Box::new(collector.clone()));
                        Some(e.insert(CachedInstrument::U64Gauge(collector)) as &CachedInstrument)
                    } else {
                        None
                    }
                }
            };
            if let Some(CachedInstrument::U64Gauge(i)) = instrument {
                i.set(val);
            }
        }
        (MetricType::Histogram(buckets), MetricValue::F64(val)) => {
            let instrument_entry = instruments.entry((event.name, attrs.clone()));
            let instrument = match instrument_entry {
                std::collections::hash_map::Entry::Occupied(ref e) => Some(e.get()),
                std::collections::hash_map::Entry::Vacant(e) => {
                    let mut histogram_opts = prometheus::HistogramOpts {
                        common_opts: prometheus::Opts {
                            namespace: String::new(),
                            subsystem: String::new(),
                            name: event.name.to_string().replace(".", "_"),
                            help: event
                                .description
                                .unwrap_or("No description provided")
                                .to_string(),
                            const_labels: attrs
                                .iter()
                                .map(|(k, v)| (k.replace(".", "_"), v.clone()))
                                .collect(),
                            variable_labels: Vec::new(),
                        },
                        buckets: buckets.clone().unwrap_or_else(|| {
                            vec![
                                0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
                            ]
                        }),
                    };
                    if let Some(u) = event.unit {
                        histogram_opts.common_opts.help += &format!(" (unit: {})", u);
                    }
                    let collector = prometheus::Histogram::with_opts(histogram_opts);
                    if let Ok(collector) = collector {
                        let _ = registry.register(Box::new(collector.clone()));
                        Some(e.insert(CachedInstrument::F64Histogram(collector))
                            as &CachedInstrument)
                    } else {
                        None
                    }
                }
            };
            if let Some(CachedInstrument::F64Histogram(i)) = instrument {
                i.observe(val);
            }
        }
        (MetricType::Histogram(buckets), MetricValue::U64(val)) => {
            let instrument_entry = instruments.entry((event.name, attrs.clone()));
            let instrument = match instrument_entry {
                std::collections::hash_map::Entry::Occupied(ref e) => Some(e.get()),
                std::collections::hash_map::Entry::Vacant(e) => {
                    let mut histogram_opts = prometheus::HistogramOpts {
                        common_opts: prometheus::Opts {
                            namespace: String::new(),
                            subsystem: String::new(),
                            name: event.name.to_string().replace(".", "_"),
                            help: event
                                .description
                                .unwrap_or("No description provided")
                                .to_string(),
                            const_labels: attrs
                                .iter()
                                .map(|(k, v)| (k.replace(".", "_"), v.clone()))
                                .collect(),
                            variable_labels: Vec::new(),
                        },
                        buckets: buckets.clone().unwrap_or_else(|| {
                            vec![
                                0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
                            ]
                        }),
                    };
                    if let Some(u) = event.unit {
                        histogram_opts.common_opts.help += &format!(" (unit: {})", u);
                    }
                    let collector = prometheus::Histogram::with_opts(histogram_opts);
                    if let Ok(collector) = collector {
                        let _ = registry.register(Box::new(collector.clone()));
                        Some(e.insert(CachedInstrument::F64Histogram(collector))
                            as &CachedInstrument)
                    } else {
                        None
                    }
                }
            };
            if let Some(CachedInstrument::F64Histogram(i)) = instrument {
                i.observe(val as f64);
            }
        }
        _ => {}
    }
}

struct PrometheusObservabilityProvider {
    inner: async_channel::Sender<ConfiguredEvent>,
}

impl Provider<ObservabilityContext> for PrometheusObservabilityProvider {
    fn name(&self) -> &str {
        "prometheus"
    }

    fn execute(&self, ctx: &mut ObservabilityContext) -> Result<(), Box<dyn Error>> {
        ctx.sink = Some(Arc::new(PrometheusEventSink {
            inner: self.inner.clone(),
            log_config: ctx.log_config.clone(),
        }));
        Ok(())
    }
}

pub struct PrometheusObservabilityModuleLoader {
    channel: (
        async_channel::Sender<ConfiguredEvent>,
        async_channel::Receiver<ConfiguredEvent>,
    ),
    cancel_token: tokio_util::sync::CancellationToken,
}

impl Default for PrometheusObservabilityModuleLoader {
    fn default() -> Self {
        Self {
            channel: async_channel::bounded(131072),
            cancel_token: tokio_util::sync::CancellationToken::new(),
        }
    }
}

impl ModuleLoader for PrometheusObservabilityModuleLoader {
    fn register_providers(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        let channel = self.channel.0.clone();

        registry.with_provider::<ObservabilityContext, _>(move || {
            Arc::new(PrometheusObservabilityProvider {
                inner: channel.clone(),
            })
        })
    }

    fn register_modules(
        &mut self,
        _registry: Arc<Registry>,
        modules: &mut Vec<Arc<dyn Module>>,
        _config: Arc<ferron_core::config::ServerConfiguration>,
    ) -> Result<(), Box<dyn Error>> {
        self.cancel_token.cancel();
        self.cancel_token = tokio_util::sync::CancellationToken::new();

        modules.push(Arc::new(PrometheusObservabilityModule {
            inner: self.channel.1.clone(),
            cancel_token: self.cancel_token.clone(),
        }));

        Ok(())
    }
}
