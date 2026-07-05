mod endpoint;
mod validator;

use std::borrow::Cow;
use std::collections::HashMap;
use std::error::Error;
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Once};

use ferron_core::config::ServerConfigurationBlock;
use ferron_core::loader::ModuleLoader;
use ferron_core::providers::Provider;
use ferron_core::registry::{Registry, RegistryBuilder};
use ferron_core::{config_validator_scoped_key, log_warn, Module};
use ferron_observability::baggage::{self, BaggageKeyPromotion, DistinctValueTracker, SignalSet};
use ferron_observability::{
    Event, EventSink, MetricAttributeValue, MetricEvent, MetricType, MetricValue,
    ObservabilityContext,
};
use tokio_util::sync::CancellationToken;

use crate::endpoint::endpoint_listener_fn;

type PrometheusInstrumentCache =
    HashMap<(&'static str, Vec<(Cow<'static, str>, String)>), CachedInstrument>;
static DROPPED_EVENT: Once = Once::new();

/// Shared configuration for an Prometheus backend instance
#[derive(Clone)]
struct PrometheusBackendConfig {
    listen: SocketAddr,
    format: String,
    auth_token: Option<String>,
    baggage_promotions: Vec<BaggageKeyPromotion>,
}

/// Wrapper that carries an event with its configuration through the channel
struct ConfiguredEvent {
    event: Option<Arc<Event>>,
    log_config: Arc<ServerConfigurationBlock>,
}

/// The Prometheus event sink that emits events to an Prometheus collector
struct PrometheusEventSink {
    inner: async_channel::Sender<ConfiguredEvent>,
    log_config: Arc<ServerConfigurationBlock>,
}

impl EventSink for PrometheusEventSink {
    #[inline]
    fn emit(&self, event: Event) {
        if matches!(event, Event::Metric(_)) {
            match self.inner.try_send(ConfiguredEvent {
                event: Some(Arc::new(event)),
                log_config: self.log_config.clone(),
            }) {
                Ok(_) => {
                    ferron_core::admin::ADMIN_METRICS
                        .observability_event_queue_len
                        .fetch_add(1, Ordering::Relaxed);
                }
                Err(_) => {
                    // Increment dropped-events metric
                    ferron_core::admin::ADMIN_METRICS
                        .observability_events_dropped
                        .fetch_add(1, Ordering::Relaxed);

                    DROPPED_EVENT.call_once(|| {
                        log_warn!(
                            "Observability event dropped (`prometheus` observability backend). \
                            This may be caused by high server load."
                        );
                    });
                }
            }
        }
    }

    #[inline]
    fn emit_arc(&self, event: std::sync::Arc<Event>) {
        if matches!(&*event, Event::Metric(_)) {
            match self.inner.try_send(ConfiguredEvent {
                event: Some(event),
                log_config: self.log_config.clone(),
            }) {
                Ok(_) => {
                    ferron_core::admin::ADMIN_METRICS
                        .observability_event_queue_len
                        .fetch_add(1, Ordering::Relaxed);
                }
                Err(_) => {
                    // Increment dropped-events metric
                    ferron_core::admin::ADMIN_METRICS
                        .observability_events_dropped
                        .fetch_add(1, Ordering::Relaxed);

                    DROPPED_EVENT.call_once(|| {
                        log_warn!(
                            "Observability event dropped (`prometheus` observability backend). \
                            This may be caused by high server load."
                        );
                    });
                }
            }
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

    let auth_token = config
        .get_value("endpoint_auth_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let baggage_promotions = parse_baggage_promotions(config);

    Ok(PrometheusBackendConfig {
        listen,
        format,
        auth_token,
        baggage_promotions,
    })
}

/// Parse the `baggage` directive from the Prometheus config block.
///
/// Expected format:
/// ```text
/// baggage {
///     key "tenant.id" {
///         attribute "tenant.id"
///         max_distinct 1000
///     }
/// }
/// ```
fn parse_baggage_promotions(config: &ServerConfigurationBlock) -> Vec<BaggageKeyPromotion> {
    let Some(baggage_entries) = config.directives.get("baggage") else {
        return Vec::new();
    };
    let Some(baggage_block) = baggage_entries.first().and_then(|e| e.children.as_ref()) else {
        return Vec::new();
    };

    let Some(key_entries) = baggage_block.directives.get("key") else {
        return Vec::new();
    };

    let mut promotions = Vec::new();
    for key_entry in key_entries {
        let Some(baggage_key) = key_entry.args.first().and_then(|v| v.as_str()) else {
            continue;
        };

        let children = key_entry.children.as_ref();

        let attribute_name = children
            .and_then(|c| c.get_value("attribute"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let max_distinct = children
            .and_then(|c| c.get_value("max_distinct"))
            .and_then(|v| {
                if v.as_boolean().is_some_and(|v| !v) {
                    None
                } else {
                    Some(v.as_number().unwrap_or(100))
                }
            })
            .map(|n| n as usize);

        promotions.push(BaggageKeyPromotion {
            baggage_key: baggage_key.to_string(),
            attribute_name,
            signals: Some(SignalSet::METRICS), // Prometheus only handles metrics.
            max_distinct,
        });
    }

    promotions
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
                ferron_core::admin::ADMIN_METRICS
                    .observability_event_queue_len
                    .fetch_sub(1, Ordering::Relaxed);

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

                if let Some(Event::Metric(metric_event)) = msg.event.as_deref() {
                    emit_metric(
                        &entry.registry,
                        metric_event,
                        &mut entry.metrics_instruments,
                        &entry.baggage_promotions,
                        &mut entry.baggage_tracker,
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
    baggage_promotions: Vec<BaggageKeyPromotion>,
    baggage_tracker: DistinctValueTracker,
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
    let config_clone = config.clone();
    let registry = prometheus::Registry::new();
    let registry_clone = registry.clone();
    let baggage_promotions = config.baggage_promotions.clone();
    // Note: Prometheus endpoint listener is spawned on demand when the first event
    // with a given config is received. This allows us to avoid starting unnecessary listeners
    // for configs that are never used, but also means that the first event may be delayed
    // while the listener is starting up.
    tokio::spawn(async move {
        let socket_addr = config_clone.listen;
        if let Err(err) = endpoint_listener_fn(config_clone, reload_token, registry_clone).await {
            ferron_core::log_warn!("Prometheus endpoint listener at {socket_addr} failed: {err}");
        }
    });
    PrometheusProviderCache {
        registry,
        metrics_instruments: HashMap::new(),
        baggage_promotions,
        baggage_tracker: DistinctValueTracker::new(),
    }
}

fn emit_metric(
    registry: &prometheus::Registry,
    event: &MetricEvent,
    instruments: &mut PrometheusInstrumentCache,
    promotions: &[BaggageKeyPromotion],
    tracker: &mut DistinctValueTracker,
) {
    // Sanitize label values to avoid high-cardinality or invalid label contents.
    fn sanitize_label_value(s: &str) -> String {
        let s = s.trim();
        if s.len() <= 128 {
            s.chars()
                .map(|c| if c.is_control() { '?' } else { c })
                .collect()
        } else {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut hasher = DefaultHasher::new();
            s.hash(&mut hasher);
            format!("hash_{:x}", hasher.finish())
        }
    }

    let mut attrs: Vec<(Cow<'static, str>, String)> = event
        .attributes
        .iter()
        .map(|(k, v)| {
            let raw = match v {
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
            };
            ((*k).into(), sanitize_label_value(&raw))
        })
        .collect();

    // Promote configured baggage keys into metric attributes
    if let Some(baggage_str) = event
        .trace_context
        .as_ref()
        .and_then(|c| c.baggage.as_deref())
    {
        let extracted = baggage::extract_promoted_keys(baggage_str, promotions, SignalSet::METRICS);
        for attr in extracted {
            let value = tracker.canonicalize(
                &attr.attribute_name,
                &attr.value,
                promotions
                    .iter()
                    .find(|p| p.effective_attribute_name() == attr.attribute_name)
                    .and_then(|p| p.max_distinct),
            );
            attrs.push((attr.attribute_name.into(), value));
        }
    }

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
                        buckets: buckets.as_deref().map(<[_]>::to_vec).unwrap_or_else(|| {
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
                        buckets: buckets.as_deref().map(<[_]>::to_vec).unwrap_or_else(|| {
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
        // Eagerly initialize the Prometheus endpoint
        let _ = self.inner.try_send(ConfiguredEvent {
            event: None,
            log_config: ctx.log_config.clone(),
        });
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

    fn register_scoped_configuration_validators(
        &mut self,
        registry: &mut HashMap<
            ferron_core::config::validator::ConfigurationValidatorScopedKey,
            Box<dyn ferron_core::config::validator::ConfigurationValidator>,
        >,
    ) {
        registry.insert(
            config_validator_scoped_key!("observability", "prometheus"),
            Box::new(validator::PrometheusObservabilityConfigurationValidator),
        );
    }
}
