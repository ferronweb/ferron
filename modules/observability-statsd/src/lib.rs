mod config;
mod validator;

use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Once};

use ferron_core::config::ServerConfigurationBlock;
use ferron_core::loader::ModuleLoader;
use ferron_core::providers::Provider;
use ferron_core::registry::{Registry, RegistryBuilder};
use ferron_core::shutdown::RELOAD_TOKEN;
use ferron_core::{config_validator_scoped_key, Module};
use ferron_observability::baggage::{self, BaggageKeyPromotion, DistinctValueTracker, SignalSet};
use ferron_observability::module::{try_send_event, ConfiguredEvent, InitAccessEvent};
use ferron_observability::{
    Event, EventSink, MetricAttributeValue, MetricEvent, MetricType, MetricValue,
    ObservabilityContext,
};

use crate::config::StatsdBackendConfig;

static FAILED_DATAGRAM_SEND: Once = Once::new();

/// Maximum length of a UDP datagram carrying StatsD metrics. 1432 bytes stays
/// safely below the typical 1500-byte MTU even with IP and UDP headers.
const MAX_DATAGRAM_LEN: usize = 1432;

/// The StatsD event sink that forwards metric events through the channel
struct StatsdEventSink {
    inner: async_channel::Sender<ConfiguredEvent>,
    log_config: Arc<ServerConfigurationBlock>,
    control_plane_metadata: Option<Arc<BTreeMap<String, String>>>,
}

impl EventSink for StatsdEventSink {
    #[inline]
    fn emit(&self, event: Event) {
        if matches!(event, Event::Metric(_)) {
            try_send_event(
                &self.inner,
                Arc::new(event),
                &self.log_config,
                &self.control_plane_metadata,
                "statsd",
            );
        }
    }

    #[inline]
    fn emit_arc(&self, event: std::sync::Arc<Event>) {
        if matches!(&*event, Event::Metric(_)) {
            try_send_event(
                &self.inner,
                event,
                &self.log_config,
                &self.control_plane_metadata,
                "statsd",
            );
        }
    }
}

/// Parse the StatsD backend configuration from a ServerConfigurationBlock
fn parse_statsd_config(config: &ServerConfigurationBlock) -> StatsdBackendConfig {
    StatsdBackendConfig::parse_config(config)
}

/// Format the full metric name by prepending the configured prefix.
fn format_metric_name(event: &MetricEvent, config: &StatsdBackendConfig) -> String {
    match &config.prefix {
        Some(prefix) => format!("{}.{}", prefix, event.name),
        None => event.name.to_string(),
    }
}

/// Format a numeric metric value for the StatsD protocol.
///
/// Returns `None` when the value cannot be represented (NaN or infinity).
fn format_value(value: MetricValue, signed_delta: bool) -> Option<String> {
    let rendered = match value {
        MetricValue::U64(v) => v.to_string(),
        MetricValue::I64(v) => {
            if signed_delta && v > 0 {
                format!("+{}", v)
            } else {
                v.to_string()
            }
        }
        MetricValue::F64(v) => {
            if !v.is_finite() {
                return None;
            }
            if signed_delta && v > 0.0 {
                format!("+{}", v)
            } else {
                v.to_string()
            }
        }
        _ => return None,
    };
    Some(rendered)
}

/// Sanitize a tag value for the DogStatsD tag syntax.
///
/// Control characters and the DogStatsD reserved characters (`,`, `#`, `:`)
/// are replaced with `?`.
fn sanitize_tag_value(s: &str) -> String {
    let s = s.trim();
    let cleaned: String = s
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, ',' | '#' | ':') {
                '?'
            } else {
                c
            }
        })
        .collect();
    cleaned
}

/// Render the metric attributes as DogStatsD tags (`|#key:value,key:value`).
///
/// Returns `None` when there are no attributes. Control plane metadata is
/// included with the `ferron_control_plane_` key prefix. Promoted W3C Baggage
/// keys are added as tags using `tracker` to cap distinct values.
fn format_tags(
    event: &MetricEvent,
    control_plane_metadata: &Option<Arc<BTreeMap<String, String>>>,
    promotions: &[BaggageKeyPromotion],
    tracker: &mut DistinctValueTracker,
) -> Option<String> {
    let mut tags: Vec<(String, String)> = Vec::new();

    for (key, value) in &event.attributes {
        let rendered = match value {
            MetricAttributeValue::String(s) => s.clone(),
            MetricAttributeValue::StaticStr(s) => (*s).to_string(),
            MetricAttributeValue::Bool(b) => {
                if *b {
                    "true".to_string()
                } else {
                    "false".to_string()
                }
            }
            MetricAttributeValue::I64(i) => i.to_string(),
            MetricAttributeValue::F64(f) => f.to_string(),
        };
        tags.push((key.to_string(), sanitize_tag_value(&rendered)));
    }

    if let Some(baggage_str) = event
        .trace_context
        .as_ref()
        .and_then(|c| c.baggage.as_deref())
    {
        let extracted = baggage::extract_promoted_keys(baggage_str, promotions, SignalSet::METRICS);
        for attr in extracted {
            let max_distinct = promotions
                .iter()
                .find(|p| p.effective_attribute_name() == attr.attribute_name)
                .and_then(|p| p.max_distinct);
            let Some(value) = tracker.canonicalize(&attr.attribute_name, &attr.value, max_distinct)
            else {
                continue;
            };
            tags.push((attr.attribute_name, sanitize_tag_value(&value)));
        }
    }

    if let Some(metadata) = control_plane_metadata {
        for (key, value) in metadata.iter() {
            tags.push((
                format!("ferron_control_plane_{}", key),
                sanitize_tag_value(value),
            ));
        }
    }

    if tags.is_empty() {
        return None;
    }

    let mut out = String::from("|#");
    for (i, (key, value)) in tags.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(key);
        out.push(':');
        out.push_str(value);
    }
    Some(out)
}

/// Format a metric event as a StatsD datagram string.
///
/// Returns `None` when the event cannot be represented (for example, a
/// non-finite histogram or gauge value).
fn format_metric_datagram(
    event: &MetricEvent,
    config: &StatsdBackendConfig,
    control_plane_metadata: &Option<Arc<BTreeMap<String, String>>>,
    tracker: &mut DistinctValueTracker,
) -> Option<String> {
    let name = format_metric_name(event, config);

    let (type_suffix, signed_delta, scale) = match (&event.ty, config.datadog) {
        (MetricType::Counter, _) => ("c", false, 1.0),
        (MetricType::Gauge, _) => ("g", false, 1.0),
        (MetricType::UpDownCounter, _) => ("g", true, 1.0),
        (MetricType::Histogram(_), false) => {
            // Vanilla StatsD has no histogram type; timers use `ms`. Values
            // with a seconds unit are converted to milliseconds.
            let scale = if event.unit == Some("s") { 1000.0 } else { 1.0 };
            ("ms", false, scale)
        }
        (MetricType::Histogram(_), true) => {
            // DogStatsD histogram type. No unit conversion is applied.
            ("h", false, 1.0)
        }
    };

    let scaled = match event.value {
        MetricValue::F64(v) => MetricValue::F64(v * scale),
        other => other,
    };

    let value = format_value(scaled, signed_delta)?;

    let mut datagram = format!("{}:{}|{}", name, value, type_suffix);

    if config.datadog {
        if let Some(tags) = format_tags(
            event,
            control_plane_metadata,
            &config.baggage_promotions,
            tracker,
        ) {
            datagram.push_str(&tags);
        }
    }

    Some(datagram)
}

struct StatsdObservabilityModule {
    inner: async_channel::Receiver<ConfiguredEvent>,
    cancel_token: tokio_util::sync::CancellationToken,
}

impl Module for StatsdObservabilityModule {
    fn name(&self) -> &str {
        "observability-statsd"
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
            run_statsd_consumer(rx, cancel_token).await;
        });

        Ok(())
    }
}

/// Consume metric events from the channel and send them to the StatsD server
/// as UDP datagrams. The UDP socket is recreated when the target address
/// changes (for example, after a configuration reload).
async fn run_statsd_consumer(
    rx: async_channel::Receiver<ConfiguredEvent>,
    cancel_token: tokio_util::sync::CancellationToken,
) {
    let reload_token = RELOAD_TOKEN.load_full();
    let mut sender: Option<(String, u16, tokio::net::UdpSocket, SocketAddr)> = None;
    let mut baggage_tracker = DistinctValueTracker::new();

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

        let config = parse_statsd_config(&msg.log_config);

        // (Re)create the connected UDP socket when the target address changes.
        let needs_socket = match sender.as_ref() {
            Some((host, port, _, _)) => *host != config.host || *port != config.port,
            None => true,
        };

        if needs_socket {
            let addr = match resolve_target(&config.host, config.port).await {
                Some(addr) => addr,
                None => {
                    ferron_core::log_warn!(
                        "Failed to resolve StatsD server address `{}:{}`, dropping metric event",
                        config.host,
                        config.port
                    );
                    continue;
                }
            };
            match tokio::net::UdpSocket::bind("0.0.0.0:0").await {
                Ok(socket) => match socket.connect(addr).await {
                    Ok(_) => {
                        sender = Some((config.host.clone(), config.port, socket, addr));
                    }
                    Err(err) => {
                        ferron_core::log_warn!(
                            "Failed to connect to StatsD server at `{}`: {}",
                            addr,
                            err
                        );
                        continue;
                    }
                },
                Err(err) => {
                    ferron_core::log_warn!("Failed to create UDP socket for StatsD: {}", err);
                    continue;
                }
            }
        }

        let Some((_, _, socket, _)) = sender.as_ref() else {
            continue;
        };

        let Event::Metric(metric_event) = &*msg.event else {
            continue;
        };

        let Some(datagram) = format_metric_datagram(
            metric_event,
            &config,
            &msg.control_plane_metadata,
            &mut baggage_tracker,
        ) else {
            continue;
        };

        if datagram.len() > MAX_DATAGRAM_LEN {
            ferron_core::log_warn!(
                "StatsD datagram for metric `{}` exceeds {} bytes and was dropped",
                metric_event.name,
                MAX_DATAGRAM_LEN
            );
            continue;
        }

        if let Err(err) = socket.send(datagram.as_bytes()).await {
            FAILED_DATAGRAM_SEND.call_once(move || {
                ferron_core::log_warn!(
                    "Failed to send StatsD datagram to {} (further errors suppressed): {}",
                    socket
                        .peer_addr()
                        .map_or("<unknown address>".into(), |s| s.to_string()),
                    err
                );
            });
        }
    }
}

/// Resolve a StatsD server host and port into a socket address.
async fn resolve_target(host: &str, port: u16) -> Option<SocketAddr> {
    match tokio::net::lookup_host((host, port)).await {
        Ok(mut addrs) => addrs.next(),
        Err(_) => None,
    }
}

struct StatsdObservabilityProvider {
    inner: async_channel::Sender<ConfiguredEvent>,
}

impl Provider<ObservabilityContext> for StatsdObservabilityProvider {
    fn name(&self) -> &str {
        "statsd"
    }

    fn execute(&self, ctx: &mut ObservabilityContext) -> Result<(), Box<dyn Error>> {
        try_send_event(
            &self.inner,
            Arc::new(Event::Access(Arc::new(InitAccessEvent))),
            &ctx.log_config,
            &ctx.control_plane_metadata,
            "statsd",
        );
        ctx.sink = Some(Arc::new(StatsdEventSink {
            inner: self.inner.clone(),
            log_config: ctx.log_config.clone(),
            control_plane_metadata: ctx.control_plane_metadata.clone(),
        }));
        Ok(())
    }
}

pub struct StatsdObservabilityModuleLoader {
    channel: (
        async_channel::Sender<ConfiguredEvent>,
        async_channel::Receiver<ConfiguredEvent>,
    ),
    cancel_token: tokio_util::sync::CancellationToken,
}

impl Default for StatsdObservabilityModuleLoader {
    fn default() -> Self {
        Self {
            channel: async_channel::bounded(131072),
            cancel_token: tokio_util::sync::CancellationToken::new(),
        }
    }
}

impl ModuleLoader for StatsdObservabilityModuleLoader {
    fn register_providers(&mut self, registry: RegistryBuilder) -> RegistryBuilder {
        let channel = self.channel.0.clone();

        registry.with_provider::<ObservabilityContext, _>(move || {
            Arc::new(StatsdObservabilityProvider {
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

        modules.push(Arc::new(StatsdObservabilityModule {
            inner: self.channel.1.clone(),
            cancel_token: self.cancel_token.clone(),
        }));

        Ok(())
    }

    fn register_directives(&mut self, registry: &mut ferron_core::directives::DirectiveRegistry) {
        use ferron_core::directives::{Directive, DirectiveSubblock};
        registry
            .register(
                Directive {
                    name: "host",
                    usage: "host <hostname>",
                    description: "This directive specifies the hostname or IP address of the StatsD server.",
                    applicable_protocols: None,
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("observability"),
            )
            .register(
                Directive {
                    name: "port",
                    usage: "port <number>",
                    description: "This directive specifies the UDP port of the StatsD server.",
                    applicable_protocols: None,
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("observability"),
            )
            .register(
                Directive {
                    name: "prefix",
                    usage: "prefix <string>",
                    description: "This directive prepends a prefix to all StatsD metric names.",
                    applicable_protocols: None,
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("observability"),
            )
            .register(
                Directive {
                    name: "datadog",
                    usage: "datadog [bool]",
                    description: "This directive enables DogStatsD extensions (tags and the histogram metric type).",
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
                    description: "This directive configures baggage key promotion for StatsD. Contains key blocks with attribute and max_distinct. Promoted keys are rendered as DogStatsD tags.",
                    applicable_protocols: None,
                    global_only: false,
                    subblock_link: None,
                },
                DirectiveSubblock::custom("observability"),
            );
    }

    fn register_scoped_configuration_validators(
        &mut self,
        registry: &mut HashMap<
            ferron_core::config::validator::ConfigurationValidatorScopedKey,
            Box<dyn ferron_core::config::validator::ConfigurationValidator>,
        >,
    ) {
        registry.insert(
            config_validator_scoped_key!("observability", "statsd"),
            Box::new(validator::StatsdObservabilityConfigurationValidator),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use ferron_observability::MetricAttributeValue;

    use super::*;
    use crate::config::StatsdBackendConfig;

    fn config(prefix: Option<&str>, datadog: bool) -> StatsdBackendConfig {
        StatsdBackendConfig {
            host: "127.0.0.1".to_string(),
            port: 8125,
            prefix: prefix.map(|s| s.to_string()),
            datadog,
            baggage_promotions: Vec::new(),
        }
    }

    fn metric(
        name: &'static str,
        ty: MetricType,
        value: MetricValue,
        unit: Option<&'static str>,
        attributes: Vec<(&'static str, MetricAttributeValue)>,
    ) -> MetricEvent {
        MetricEvent {
            name,
            attributes,
            ty,
            value,
            unit,
            description: None,
            trace_context: None,
        }
    }

    #[test]
    fn counter_formats_as_delta() {
        let event = metric(
            "ferron.http.server.request_count",
            MetricType::Counter,
            MetricValue::U64(1),
            Some("{request}"),
            vec![],
        );
        let datagram = format_metric_datagram(
            &event,
            &config(None, false),
            &None,
            &mut DistinctValueTracker::new(),
        )
        .unwrap();
        assert_eq!(datagram, "ferron.http.server.request_count:1|c");
    }

    #[test]
    fn counter_float_value() {
        let event = metric(
            "ferron.test.counter",
            MetricType::Counter,
            MetricValue::F64(1.5),
            None,
            vec![],
        );
        let datagram = format_metric_datagram(
            &event,
            &config(None, false),
            &None,
            &mut DistinctValueTracker::new(),
        )
        .unwrap();
        assert_eq!(datagram, "ferron.test.counter:1.5|c");
    }

    #[test]
    fn gauge_formats_absolute() {
        let event = metric(
            "ferron.admin.connections_active",
            MetricType::Gauge,
            MetricValue::I64(42),
            None,
            vec![],
        );
        let datagram = format_metric_datagram(
            &event,
            &config(None, false),
            &None,
            &mut DistinctValueTracker::new(),
        )
        .unwrap();
        assert_eq!(datagram, "ferron.admin.connections_active:42|g");
    }

    #[test]
    fn up_down_counter_uses_signed_delta() {
        let pos = metric(
            "ferron.test.updown",
            MetricType::UpDownCounter,
            MetricValue::I64(3),
            None,
            vec![],
        );
        let datagram = format_metric_datagram(
            &pos,
            &config(None, false),
            &None,
            &mut DistinctValueTracker::new(),
        )
        .unwrap();
        assert_eq!(datagram, "ferron.test.updown:+3|g");

        let neg = metric(
            "ferron.test.updown",
            MetricType::UpDownCounter,
            MetricValue::I64(-3),
            None,
            vec![],
        );
        let datagram = format_metric_datagram(
            &neg,
            &config(None, false),
            &None,
            &mut DistinctValueTracker::new(),
        )
        .unwrap();
        assert_eq!(datagram, "ferron.test.updown:-3|g");

        let zero = metric(
            "ferron.test.updown",
            MetricType::UpDownCounter,
            MetricValue::I64(0),
            None,
            vec![],
        );
        let datagram = format_metric_datagram(
            &zero,
            &config(None, false),
            &None,
            &mut DistinctValueTracker::new(),
        )
        .unwrap();
        assert_eq!(datagram, "ferron.test.updown:0|g");
    }

    #[test]
    fn histogram_uses_ms_type_vanilla() {
        let event = metric(
            "http.server.request.duration",
            MetricType::Histogram(None),
            MetricValue::F64(0.25),
            Some("s"),
            vec![],
        );
        let datagram = format_metric_datagram(
            &event,
            &config(None, false),
            &None,
            &mut DistinctValueTracker::new(),
        )
        .unwrap();
        assert_eq!(datagram, "http.server.request.duration:250|ms");
    }

    #[test]
    fn histogram_without_seconds_unit_is_not_scaled() {
        let event = metric(
            "ferron.test.histogram",
            MetricType::Histogram(None),
            MetricValue::F64(12.5),
            None,
            vec![],
        );
        let datagram = format_metric_datagram(
            &event,
            &config(None, false),
            &None,
            &mut DistinctValueTracker::new(),
        )
        .unwrap();
        assert_eq!(datagram, "ferron.test.histogram:12.5|ms");
    }

    #[test]
    fn histogram_uses_h_type_in_datadog_mode() {
        let event = metric(
            "http.server.request.duration",
            MetricType::Histogram(None),
            MetricValue::F64(0.25),
            Some("s"),
            vec![],
        );
        let datagram = format_metric_datagram(
            &event,
            &config(None, true),
            &None,
            &mut DistinctValueTracker::new(),
        )
        .unwrap();
        assert_eq!(datagram, "http.server.request.duration:0.25|h");
    }

    #[test]
    fn prefix_is_prepended() {
        let event = metric(
            "ferron.http.server.request_count",
            MetricType::Counter,
            MetricValue::U64(1),
            None,
            vec![],
        );
        let datagram = format_metric_datagram(
            &event,
            &config(Some("myapp"), false),
            &None,
            &mut DistinctValueTracker::new(),
        )
        .unwrap();
        assert_eq!(datagram, "myapp.ferron.http.server.request_count:1|c");
    }

    #[test]
    fn non_finite_values_are_skipped() {
        let nan = metric(
            "ferron.test.gauge",
            MetricType::Gauge,
            MetricValue::F64(f64::NAN),
            None,
            vec![],
        );
        assert!(format_metric_datagram(
            &nan,
            &config(None, false),
            &None,
            &mut DistinctValueTracker::new()
        )
        .is_none());

        let inf = metric(
            "ferron.test.gauge",
            MetricType::Gauge,
            MetricValue::F64(f64::INFINITY),
            None,
            vec![],
        );
        assert!(format_metric_datagram(
            &inf,
            &config(None, false),
            &None,
            &mut DistinctValueTracker::new()
        )
        .is_none());
    }

    #[test]
    fn no_tags_in_vanilla_mode() {
        let event = metric(
            "ferron.test.counter",
            MetricType::Counter,
            MetricValue::U64(1),
            None,
            vec![("ferron.host", MetricAttributeValue::StaticStr("localhost"))],
        );
        let datagram = format_metric_datagram(
            &event,
            &config(None, false),
            &None,
            &mut DistinctValueTracker::new(),
        )
        .unwrap();
        assert_eq!(datagram, "ferron.test.counter:1|c");
    }

    #[test]
    fn tags_rendered_in_datadog_mode() {
        let event = metric(
            "ferron.test.counter",
            MetricType::Counter,
            MetricValue::U64(1),
            None,
            vec![
                ("ferron.host", MetricAttributeValue::StaticStr("localhost")),
                ("http.response.status_code", MetricAttributeValue::I64(200)),
                ("ferron.upstream.is_tls", MetricAttributeValue::Bool(true)),
            ],
        );
        let datagram = format_metric_datagram(
            &event,
            &config(None, true),
            &None,
            &mut DistinctValueTracker::new(),
        )
        .unwrap();
        assert_eq!(
            datagram,
            "ferron.test.counter:1|c|#ferron.host:localhost,http.response.status_code:200,ferron.upstream.is_tls:true"
        );
    }

    #[test]
    fn control_plane_metadata_becomes_tags() {
        let event = metric(
            "ferron.test.counter",
            MetricType::Counter,
            MetricValue::U64(1),
            None,
            vec![],
        );
        let mut metadata = BTreeMap::new();
        metadata.insert("region".to_string(), "eu-west".to_string());
        let metadata = Some(Arc::new(metadata));
        let datagram = format_metric_datagram(
            &event,
            &config(None, true),
            &metadata,
            &mut DistinctValueTracker::new(),
        )
        .unwrap();
        assert_eq!(
            datagram,
            "ferron.test.counter:1|c|#ferron_control_plane_region:eu-west"
        );
    }

    #[test]
    fn tag_values_are_sanitized() {
        let event = metric(
            "ferron.test.counter",
            MetricType::Counter,
            MetricValue::U64(1),
            None,
            vec![(
                "ferron.host",
                MetricAttributeValue::String("a,b#c:d".to_string()),
            )],
        );
        let datagram = format_metric_datagram(
            &event,
            &config(None, true),
            &None,
            &mut DistinctValueTracker::new(),
        )
        .unwrap();
        assert_eq!(datagram, "ferron.test.counter:1|c|#ferron.host:a?b?c?d");
    }

    #[test]
    fn baggage_keys_promoted_to_tags_in_datadog_mode() {
        use ferron_observability::baggage::{BaggageKeyPromotion, SignalSet};

        let cfg = StatsdBackendConfig {
            host: "127.0.0.1".to_string(),
            port: 8125,
            prefix: None,
            datadog: true,
            baggage_promotions: vec![BaggageKeyPromotion {
                baggage_key: "tenant.id".to_string(),
                attribute_name: None,
                signals: Some(SignalSet::METRICS),
                max_distinct: None,
            }],
        };

        let mut event = metric(
            "ferron.test.counter",
            MetricType::Counter,
            MetricValue::U64(1),
            None,
            vec![("ferron.host", MetricAttributeValue::StaticStr("localhost"))],
        );
        event.trace_context = Some(ferron_observability::EventTraceContext {
            trace_id: [0; 32],
            span_id: [0; 16],
            baggage: Some("tenant.id=acme,other=skip".to_string()),
            sampled: None,
        });

        let mut tracker = DistinctValueTracker::new();
        let datagram = format_metric_datagram(&event, &cfg, &None, &mut tracker).unwrap();
        assert_eq!(
            datagram,
            "ferron.test.counter:1|c|#ferron.host:localhost,tenant.id:acme"
        );
    }

    #[test]
    fn baggage_promotion_ignored_without_datadog() {
        use ferron_observability::baggage::{BaggageKeyPromotion, SignalSet};

        let cfg = StatsdBackendConfig {
            host: "127.0.0.1".to_string(),
            port: 8125,
            prefix: None,
            datadog: false,
            baggage_promotions: vec![BaggageKeyPromotion {
                baggage_key: "tenant.id".to_string(),
                attribute_name: None,
                signals: Some(SignalSet::METRICS),
                max_distinct: None,
            }],
        };

        let mut event = metric(
            "ferron.test.counter",
            MetricType::Counter,
            MetricValue::U64(1),
            None,
            vec![],
        );
        event.trace_context = Some(ferron_observability::EventTraceContext {
            trace_id: [0; 32],
            span_id: [0; 16],
            baggage: Some("tenant.id=acme".to_string()),
            sampled: None,
        });

        let mut tracker = DistinctValueTracker::new();
        let datagram = format_metric_datagram(&event, &cfg, &None, &mut tracker).unwrap();
        assert_eq!(datagram, "ferron.test.counter:1|c");
    }

    #[tokio::test]
    async fn round_trip_sends_datagram_over_udp() {
        // Bind a UDP receiver on a random local port.
        let receiver = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let target = receiver.local_addr().unwrap();

        let (tx, rx) = async_channel::bounded(16);
        let cancel = tokio_util::sync::CancellationToken::new();
        let consumer = tokio::spawn(run_statsd_consumer(rx, cancel.clone()));

        // Feed a metric event through the sink machinery. The consumer parses
        // the target address from the event's log configuration.
        let mut directives = HashMap::new();
        directives.insert(
            "host".to_string(),
            vec![ferron_core::config::ServerConfigurationDirectiveEntry {
                args: vec![ferron_core::config::ServerConfigurationValue::String(
                    "127.0.0.1".to_string(),
                    None,
                )],
                children: None,
                span: None,
            }],
        );
        directives.insert(
            "port".to_string(),
            vec![ferron_core::config::ServerConfigurationDirectiveEntry {
                args: vec![ferron_core::config::ServerConfigurationValue::Number(
                    target.port() as i64,
                    None,
                )],
                children: None,
                span: None,
            }],
        );
        directives.insert(
            "prefix".to_string(),
            vec![ferron_core::config::ServerConfigurationDirectiveEntry {
                args: vec![ferron_core::config::ServerConfigurationValue::String(
                    "e2e".to_string(),
                    None,
                )],
                children: None,
                span: None,
            }],
        );
        directives.insert(
            "datadog".to_string(),
            vec![ferron_core::config::ServerConfigurationDirectiveEntry {
                args: vec![ferron_core::config::ServerConfigurationValue::Boolean(
                    true, None,
                )],
                children: None,
                span: None,
            }],
        );
        let block = ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: HashMap::new(),
            span: None,
        };
        let sink = StatsdEventSink {
            inner: tx,
            log_config: Arc::new(block),
            control_plane_metadata: None,
        };

        sink.emit(Event::Metric(metric(
            "ferron.http.server.request_count",
            MetricType::Counter,
            MetricValue::U64(1),
            None,
            vec![("ferron.host", MetricAttributeValue::StaticStr("localhost"))],
        )));

        let mut buf = [0u8; 512];
        let (len, from) = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            receiver.recv_from(&mut buf),
        )
        .await
        .expect("timed out waiting for datagram")
        .unwrap();
        assert_eq!(from.ip(), target.ip());
        assert_eq!(
            &buf[..len],
            b"e2e.ferron.http.server.request_count:1|c|#ferron.host:localhost"
        );

        cancel.cancel();
        consumer.await.unwrap();
    }
}
