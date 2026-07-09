mod endpoint;
mod validator;

use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::fmt;
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Once};

use ferron_core::config::ServerConfigurationBlock;
use ferron_core::loader::ModuleLoader;
use ferron_core::providers::Provider;
use ferron_core::registry::{Registry, RegistryBuilder};
use ferron_core::shutdown::RELOAD_TOKEN;
use ferron_core::{config_validator_scoped_key, log_warn, Module};
use ferron_observability::baggage::{self, BaggageKeyPromotion, DistinctValueTracker, SignalSet};
use ferron_observability::{
    Event, EventSink, MetricAttributeValue, MetricEvent, MetricType, MetricValue,
    ObservabilityContext,
};
use prometheus_client::encoding::{EncodeLabelKey, EncodeLabelSet, EncodeLabelValue};
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::metrics::histogram::{self, Histogram, NativeHistogramConfig};
use tokio_util::sync::CancellationToken;

use crate::endpoint::endpoint_listener_fn;

static DROPPED_EVENT: Once = Once::new();

const DEFAULT_BUCKETS: [f64; 11] = [
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

/// A dynamic label set for metrics with arbitrary key-value attributes.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct DynamicLabels(Vec<(String, String)>);

impl EncodeLabelSet for DynamicLabels {
    fn encode(
        &self,
        encoder: &mut prometheus_client::encoding::LabelSetEncoder,
    ) -> Result<(), fmt::Error> {
        for (key, value) in &self.0 {
            let mut label_encoder = encoder.encode_label();
            let mut label_key_encoder = label_encoder.encode_label_key()?;
            EncodeLabelKey::encode(key, &mut label_key_encoder)?;
            let mut label_value_encoder = label_key_encoder.encode_label_value()?;
            EncodeLabelValue::encode(value, &mut label_value_encoder)?;
            label_value_encoder.finish()?;
        }
        Ok(())
    }
}

/// Shared configuration for a Prometheus backend instance
#[derive(Clone)]
struct PrometheusBackendConfig {
    listen: SocketAddr,
    format: String,
    auth_token: Option<String>,
    native_histograms: bool,
    baggage_promotions: Vec<BaggageKeyPromotion>,
}

/// Wrapper that carries an event with its configuration through the channel
struct ConfiguredEvent {
    event: Option<Arc<Event>>,
    log_config: Arc<ServerConfigurationBlock>,
    control_plane_metadata: Option<Arc<BTreeMap<String, String>>>,
}

/// The Prometheus event sink that emits events to a Prometheus collector
struct PrometheusEventSink {
    inner: async_channel::Sender<ConfiguredEvent>,
    log_config: Arc<ServerConfigurationBlock>,
    control_plane_metadata: Option<Arc<BTreeMap<String, String>>>,
}

impl EventSink for PrometheusEventSink {
    #[inline]
    fn emit(&self, event: Event) {
        if matches!(event, Event::Metric(_)) {
            match self.inner.try_send(ConfiguredEvent {
                event: Some(Arc::new(event)),
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

    let native_histograms = config
        .get_value("endpoint_native_histograms")
        .and_then(|v| v.as_boolean())
        .unwrap_or(false);

    let baggage_promotions = parse_baggage_promotions(config);

    Ok(PrometheusBackendConfig {
        listen,
        format,
        auth_token,
        native_histograms,
        baggage_promotions,
    })
}

/// Parse the `baggage` directive from the Prometheus config block.
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
            signals: Some(SignalSet::METRICS),
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
            let mut providers: HashMap<String, PrometheusProviderCache> = HashMap::new();
            let reload_token = RELOAD_TOKEN.load_full();

            while let Some(msg) = tokio::select! {
                result = async {
                    let Some(result) = reload_token.run_until_cancelled(rx.recv()).await else {
                        return std::future::pending().await;
                    };
                    result
                } => result.ok(),
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
                let entry = providers.entry(cache_key).or_insert_with(|| {
                    init_provider(
                        &config,
                        cancel_token.clone(),
                        msg.control_plane_metadata.clone(),
                    )
                });

                if let Some(Event::Metric(metric_event)) = msg.event.as_deref() {
                    emit_metric(
                        &entry.registry,
                        metric_event,
                        &mut entry.metrics_cache,
                        &entry.baggage_promotions,
                        &mut entry.baggage_tracker,
                        &entry.control_plane_metadata,
                        entry.native_histograms,
                    )
                    .await;
                }
            }
        });

        Ok(())
    }
}

/// Cached Prometheus providers for a given config
struct PrometheusProviderCache {
    registry: Arc<tokio::sync::RwLock<prometheus_client::registry::Registry>>,
    metrics_cache: MetricCache,
    baggage_promotions: Vec<BaggageKeyPromotion>,
    baggage_tracker: DistinctValueTracker,
    control_plane_metadata: Option<Arc<BTreeMap<String, String>>>,
    native_histograms: bool,
}

type MetricCache = HashMap<&'static str, CachedMetric>;

enum CachedMetric {
    BareCounter(Counter),
    BareGauge(Gauge),
    BareHistogram(Histogram),
    FamilyCounter(Family<DynamicLabels, Counter>),
    FamilyGauge(Family<DynamicLabels, Gauge>),
    FamilyHistogram(Family<DynamicLabels, Histogram>),
}

fn config_cache_key(config: &PrometheusBackendConfig) -> String {
    format!("{}|{}", config.listen, config.format)
}

fn init_provider(
    config: &PrometheusBackendConfig,
    reload_token: CancellationToken,
    control_plane_metadata: Option<Arc<BTreeMap<String, String>>>,
) -> PrometheusProviderCache {
    let config_clone = config.clone();
    let mut registry = prometheus_client::registry::Registry::default();
    let baggage_promotions = config.baggage_promotions.clone();
    let native_histograms = config.native_histograms;

    // Register self-referential scrape metrics
    let scrape_duration = Histogram::new(histogram::exponential_buckets(0.001, 2.0, 14));
    registry.register(
        "ferron_prometheus_scrape_duration_seconds",
        "Duration of Prometheus scrape requests in seconds",
        scrape_duration.clone(),
    );

    let scrape_total = Counter::default();
    registry.register(
        "ferron_prometheus_scrape_total",
        "Total number of Prometheus scrape requests",
        scrape_total.clone(),
    );

    let scrape_errors = Counter::default();
    registry.register(
        "ferron_prometheus_scrape_errors_total",
        "Total number of failed Prometheus scrape requests",
        scrape_errors.clone(),
    );

    let registry = Arc::new(tokio::sync::RwLock::new(registry));

    let registry2 = registry.clone();
    tokio::spawn(async move {
        let socket_addr = config_clone.listen;
        if let Err(err) = endpoint_listener_fn(
            config_clone,
            reload_token,
            registry2,
            scrape_duration,
            scrape_total,
            scrape_errors,
        )
        .await
        {
            ferron_core::log_warn!("Prometheus endpoint listener at {socket_addr} failed: {err}");
        }
    });

    PrometheusProviderCache {
        registry: registry,
        metrics_cache: HashMap::new(),
        baggage_promotions,
        baggage_tracker: DistinctValueTracker::new(),
        control_plane_metadata,
        native_histograms,
    }
}

fn make_histogram_constructor(native_histograms: bool) -> fn() -> Histogram {
    if native_histograms {
        fn constructor_native() -> Histogram {
            let native = NativeHistogramConfig::new(1.1);
            Histogram::new_classic_and_native(DEFAULT_BUCKETS, native)
        }
        constructor_native
    } else {
        fn constructor_classic() -> Histogram {
            Histogram::new(DEFAULT_BUCKETS)
        }
        constructor_classic
    }
}

async fn emit_metric(
    registry: &tokio::sync::RwLock<prometheus_client::registry::Registry>,
    event: &MetricEvent,
    cache: &mut MetricCache,
    promotions: &[BaggageKeyPromotion],
    tracker: &mut DistinctValueTracker,
    control_plane_metadata: &Option<Arc<BTreeMap<String, String>>>,
    native_histograms: bool,
) {
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

    let mut attrs: Vec<(String, String)> = event
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
            (k.replace('.', "_"), sanitize_label_value(&raw))
        })
        .collect();

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
            attrs.push((attr.attribute_name.replace('.', "_"), value));
        }
    }

    if let Some(metadata) = control_plane_metadata {
        for (key, value) in metadata.iter() {
            let attr_key = format!("ferron_control_plane_{}", key);
            attrs.push((attr_key, value.clone()));
        }
    }

    let labels = DynamicLabels(attrs);

    match (&event.ty, event.value) {
        (MetricType::Counter, MetricValue::F64(val)) => {
            if val < 0.0 {
                return;
            }
            let cached = match cache.entry(event.name) {
                std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::hash_map::Entry::Vacant(e) => e.insert(if labels.0.is_empty() {
                    let metric = Counter::default();
                    registry.write().await.register(
                        event.name.to_string().replace(".", "_"),
                        format_description(event),
                        metric.clone(),
                    );
                    CachedMetric::BareCounter(metric)
                } else {
                    let metric = Family::default();
                    registry.write().await.register(
                        event.name.to_string().replace(".", "_"),
                        format_description(event),
                        metric.clone(),
                    );
                    CachedMetric::FamilyCounter(metric)
                }),
            };
            match cached {
                CachedMetric::BareCounter(c) => {
                    c.inc_by(val as u64);
                }
                CachedMetric::FamilyCounter(f) => {
                    f.get_or_create(&labels).inc_by(val as u64);
                }
                _ => {}
            }
        }
        (MetricType::Counter, MetricValue::U64(val)) => {
            let cached = match cache.entry(event.name) {
                std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::hash_map::Entry::Vacant(e) => e.insert(if labels.0.is_empty() {
                    let metric = Counter::default();
                    registry.write().await.register(
                        event.name.to_string().replace(".", "_"),
                        format_description(event),
                        metric.clone(),
                    );
                    CachedMetric::BareCounter(metric)
                } else {
                    let metric = Family::default();
                    registry.write().await.register(
                        event.name.to_string().replace(".", "_"),
                        format_description(event),
                        metric.clone(),
                    );
                    CachedMetric::FamilyCounter(metric)
                }),
            };
            match cached {
                CachedMetric::BareCounter(c) => {
                    c.inc_by(val);
                }
                CachedMetric::FamilyCounter(f) => {
                    f.get_or_create(&labels).inc_by(val);
                }
                _ => {}
            }
        }
        (MetricType::UpDownCounter, MetricValue::F64(val)) => {
            let cached = match cache.entry(event.name) {
                std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::hash_map::Entry::Vacant(e) => e.insert(if labels.0.is_empty() {
                    let metric = Gauge::default();
                    registry.write().await.register(
                        event.name.to_string().replace(".", "_"),
                        format_description(event),
                        metric.clone(),
                    );
                    CachedMetric::BareGauge(metric)
                } else {
                    let metric = Family::default();
                    registry.write().await.register(
                        event.name.to_string().replace(".", "_"),
                        format_description(event),
                        metric.clone(),
                    );
                    CachedMetric::FamilyGauge(metric)
                }),
            };
            match cached {
                CachedMetric::BareGauge(g) => {
                    g.inc_by(val as i64);
                }
                CachedMetric::FamilyGauge(f) => {
                    f.get_or_create(&labels).inc_by(val as i64);
                }
                _ => {}
            }
        }
        (MetricType::UpDownCounter, MetricValue::I64(val)) => {
            let cached = match cache.entry(event.name) {
                std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::hash_map::Entry::Vacant(e) => e.insert(if labels.0.is_empty() {
                    let metric = Gauge::default();
                    registry.write().await.register(
                        event.name.to_string().replace(".", "_"),
                        format_description(event),
                        metric.clone(),
                    );
                    CachedMetric::BareGauge(metric)
                } else {
                    let metric = Family::default();
                    registry.write().await.register(
                        event.name.to_string().replace(".", "_"),
                        format_description(event),
                        metric.clone(),
                    );
                    CachedMetric::FamilyGauge(metric)
                }),
            };
            match cached {
                CachedMetric::BareGauge(g) => {
                    g.inc_by(val);
                }
                CachedMetric::FamilyGauge(f) => {
                    f.get_or_create(&labels).inc_by(val);
                }
                _ => {}
            }
        }
        (MetricType::Gauge, MetricValue::F64(val)) => {
            let cached = match cache.entry(event.name) {
                std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::hash_map::Entry::Vacant(e) => e.insert(if labels.0.is_empty() {
                    let metric = Gauge::default();
                    registry.write().await.register(
                        event.name.to_string().replace(".", "_"),
                        format_description(event),
                        metric.clone(),
                    );
                    CachedMetric::BareGauge(metric)
                } else {
                    let metric = Family::default();
                    registry.write().await.register(
                        event.name.to_string().replace(".", "_"),
                        format_description(event),
                        metric.clone(),
                    );
                    CachedMetric::FamilyGauge(metric)
                }),
            };
            match cached {
                CachedMetric::BareGauge(g) => {
                    g.set(val as i64);
                }
                CachedMetric::FamilyGauge(f) => {
                    f.get_or_create(&labels).set(val as i64);
                }
                _ => {}
            }
        }
        (MetricType::Gauge, MetricValue::I64(val)) => {
            let cached = match cache.entry(event.name) {
                std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::hash_map::Entry::Vacant(e) => e.insert(if labels.0.is_empty() {
                    let metric = Gauge::default();
                    registry.write().await.register(
                        event.name.to_string().replace(".", "_"),
                        format_description(event),
                        metric.clone(),
                    );
                    CachedMetric::BareGauge(metric)
                } else {
                    let metric = Family::default();
                    registry.write().await.register(
                        event.name.to_string().replace(".", "_"),
                        format_description(event),
                        metric.clone(),
                    );
                    CachedMetric::FamilyGauge(metric)
                }),
            };
            match cached {
                CachedMetric::BareGauge(g) => {
                    g.set(val);
                }
                CachedMetric::FamilyGauge(f) => {
                    f.get_or_create(&labels).set(val);
                }
                _ => {}
            }
        }
        (MetricType::Gauge, MetricValue::U64(val)) => {
            let cached = match cache.entry(event.name) {
                std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::hash_map::Entry::Vacant(e) => e.insert(if labels.0.is_empty() {
                    let metric = Gauge::default();
                    registry.write().await.register(
                        event.name.to_string().replace(".", "_"),
                        format_description(event),
                        metric.clone(),
                    );
                    CachedMetric::BareGauge(metric)
                } else {
                    let metric = Family::default();
                    registry.write().await.register(
                        event.name.to_string().replace(".", "_"),
                        format_description(event),
                        metric.clone(),
                    );
                    CachedMetric::FamilyGauge(metric)
                }),
            };
            match cached {
                CachedMetric::BareGauge(g) => {
                    g.set(val as i64);
                }
                CachedMetric::FamilyGauge(f) => {
                    f.get_or_create(&labels).set(val as i64);
                }
                _ => {}
            }
        }
        (MetricType::Histogram(_), MetricValue::F64(val)) => {
            let cached = match cache.entry(event.name) {
                std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::hash_map::Entry::Vacant(e) => e.insert({
                    let constructor = make_histogram_constructor(native_histograms);
                    if labels.0.is_empty() {
                        let metric = constructor();
                        registry.write().await.register(
                            event.name.to_string().replace(".", "_"),
                            format_description(event),
                            metric.clone(),
                        );
                        CachedMetric::BareHistogram(metric)
                    } else {
                        let metric = Family::new_with_constructor(constructor);
                        registry.write().await.register(
                            event.name.to_string().replace(".", "_"),
                            format_description(event),
                            metric.clone(),
                        );
                        CachedMetric::FamilyHistogram(metric)
                    }
                }),
            };
            match cached {
                CachedMetric::BareHistogram(h) => {
                    h.observe(val);
                }
                CachedMetric::FamilyHistogram(f) => {
                    f.get_or_create(&labels).observe(val);
                }
                _ => {}
            }
        }
        (MetricType::Histogram(_), MetricValue::U64(val)) => {
            let cached = match cache.entry(event.name) {
                std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::hash_map::Entry::Vacant(e) => e.insert({
                    let constructor = make_histogram_constructor(native_histograms);
                    if labels.0.is_empty() {
                        let metric = constructor();
                        registry.write().await.register(
                            event.name.to_string().replace(".", "_"),
                            format_description(event),
                            metric.clone(),
                        );
                        CachedMetric::BareHistogram(metric)
                    } else {
                        let metric = Family::new_with_constructor(constructor);
                        registry.write().await.register(
                            event.name.to_string().replace(".", "_"),
                            format_description(event),
                            metric.clone(),
                        );
                        CachedMetric::FamilyHistogram(metric)
                    }
                }),
            };
            match cached {
                CachedMetric::BareHistogram(h) => {
                    h.observe(val as f64);
                }
                CachedMetric::FamilyHistogram(f) => {
                    f.get_or_create(&labels).observe(val as f64);
                }
                _ => {}
            }
        }
        _ => {}
    }
}

fn format_description(event: &MetricEvent) -> String {
    let base = event
        .description
        .as_ref()
        .map(|d| d.to_string().trim_end_matches('.').to_string())
        .unwrap_or_else(|| "No description provided".to_string());
    let unit_suffix = if let Some(unit) = event.unit.as_ref() {
        format!(" (unit: {})", unit)
    } else {
        "".to_string()
    };
    format!("{}{}", base, unit_suffix)
}

struct PrometheusObservabilityProvider {
    inner: async_channel::Sender<ConfiguredEvent>,
}

impl Provider<ObservabilityContext> for PrometheusObservabilityProvider {
    fn name(&self) -> &str {
        "prometheus"
    }

    fn execute(&self, ctx: &mut ObservabilityContext) -> Result<(), Box<dyn Error>> {
        let _ = self.inner.try_send(ConfiguredEvent {
            event: None,
            log_config: ctx.log_config.clone(),
            control_plane_metadata: ctx.control_plane_metadata.clone(),
        });
        ctx.sink = Some(Arc::new(PrometheusEventSink {
            inner: self.inner.clone(),
            log_config: ctx.log_config.clone(),
            control_plane_metadata: ctx.control_plane_metadata.clone(),
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
