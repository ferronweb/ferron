use std::borrow::Cow;
use std::{cell::RefCell, collections::HashMap, sync::Arc};

use dashmap::DashMap;
use ferron_core::{config::ServerConfigurationBlock, registry::Registry};
use ferron_observability::{
    baggage::{self, BaggageKeyPromotion, DistinctValueTracker, SignalSet},
    AccessEvent, AccessVisitor, LogAttributeValue, LogEvent, LogFormatterContext, LogLevel,
    MetricAttributeValue, MetricEvent, MetricType, MetricValue, Parent, TraceAttributeValue,
    TraceEvent,
};
use opentelemetry::{
    baggage::BaggageExt,
    logs::AnyValue,
    trace::{Link, SpanKind, TraceContextExt, TraceId, TracerProvider},
    KeyValue,
};
use opentelemetry_otlp::{WithExportConfig, WithHttpConfig, WithTonicConfig};
use opentelemetry_sdk::{
    trace::{Sampler, SamplingDecision, SamplingResult, ShouldSample, SdkTracerProvider},
    Resource,
};

use crate::{
    client::{build_tonic_channel, HyperOtelClient},
    config::{AttributeMatcher, AttributeSamplingRule, LogStyle, OtlpBackendConfig, SignalConfig, TraceSamplingConfig, TraceSamplingMode},
};

/// An attribute-based sampler that makes sampling decisions based on span
/// attributes provided at span creation time (via `builder_attributes`).
#[derive(Debug, Clone)]
struct AttributeBasedSampler {
    rules: Vec<AttributeSamplingRule>,
}

impl ShouldSample for AttributeBasedSampler {
    fn should_sample(
        &self,
        parent_context: Option<&opentelemetry::Context>,
        _trace_id: TraceId,
        _name: &str,
        _span_kind: &SpanKind,
        attributes: &[KeyValue],
        _links: &[Link],
    ) -> SamplingResult {
        let decision = if self.rules.iter().any(|rule| {
            match &rule.matcher {
                AttributeMatcher::Exact(expected) => attributes.iter().any(|kv| {
                    if kv.key.as_str() != rule.attribute.as_str() {
                        return false;
                    }
                    match &kv.value {
                        opentelemetry::Value::String(s) => s.as_ref() == expected.as_str(),
                        _ => false,
                    }
                }),
                AttributeMatcher::Prefix(prefix) => attributes.iter().any(|kv| {
                    if kv.key.as_str() != rule.attribute.as_str() {
                        return false;
                    }
                    match &kv.value {
                        opentelemetry::Value::String(s) => s.as_ref().starts_with(prefix.as_str()),
                        _ => false,
                    }
                }),
                AttributeMatcher::Exists => {
                    attributes.iter().any(|kv| kv.key.as_str() == rule.attribute.as_str())
                }
            }
        }) {
            SamplingDecision::RecordAndSample
        } else {
            SamplingDecision::Drop
        };

        SamplingResult {
            decision,
            attributes: vec![],
                trace_state: parent_context
                    .map(|cx| cx.span().span_context().trace_state().clone())
                    .unwrap_or_default(),
        }
    }
}

/// Build an OpenTelemetry `ShouldSample` implementation from the sampling config.
#[cfg_attr(test, allow(dead_code))]
pub(crate) fn build_sampler(config: &TraceSamplingConfig) -> Box<dyn ShouldSample> {
    match &config.mode {
        TraceSamplingMode::AlwaysOn => Box::new(Sampler::AlwaysOn),
        TraceSamplingMode::AlwaysOff => Box::new(Sampler::AlwaysOff),
        TraceSamplingMode::ParentBasedAlwaysOn => {
            Box::new(Sampler::ParentBased(Box::new(Sampler::AlwaysOn)))
        }
        TraceSamplingMode::TraceIdRatioBased { ratio } => {
            Box::new(Sampler::TraceIdRatioBased(*ratio))
        }
        TraceSamplingMode::ParentBasedTraceIdRatio { ratio } => Box::new(Sampler::ParentBased(
            Box::new(Sampler::TraceIdRatioBased(*ratio)),
        )),
        TraceSamplingMode::AttributeBased { rules } => {
            Box::new(AttributeBasedSampler { rules: rules.clone() })
        }
    }
}

/// Correlation context: tracks active spans per host sink instance.
pub struct CorrelationContext {
    /// Active spans: span_key -> active span entry
    active_spans: DashMap<String, ActiveSpan>,
}

struct ActiveSpan {
    trace_id_hex: String,
    span_id_hex: String,
    sampled: bool,
    span: opentelemetry_sdk::trace::Span,
    baggage: Option<String>,
}

#[derive(Debug, Default)]
struct RequestedIdGenerator;

#[derive(Clone, Copy, Debug, Default)]
struct RequestedIds {
    trace_id: Option<opentelemetry::TraceId>,
    span_id: Option<opentelemetry::SpanId>,
}

thread_local! {
    static REQUESTED_IDS: RefCell<Option<RequestedIds>> = const { RefCell::new(None) };
}

impl CorrelationContext {
    pub fn new() -> Self {
        Self {
            active_spans: DashMap::new(),
        }
    }

    pub fn insert_span(
        &self,
        key: impl Into<String>,
        trace_id_hex: String,
        span_id_hex: String,
        sampled: bool,
        span: opentelemetry_sdk::trace::Span,
        baggage: Option<String>,
    ) {
        self.active_spans.insert(
            key.into(),
            ActiveSpan {
                trace_id_hex,
                span_id_hex,
                sampled,
                span,
                baggage,
            },
        );
    }

    fn remove_span(&self, key: &str) -> Option<ActiveSpan> {
        self.active_spans.remove(key).map(|(_, v)| v)
    }

    /// Look up an active span's trace and span ID for use as a parent.
    pub fn get_parent_ids(&self, key: &str) -> Option<(String, String, bool, Option<String>)> {
        self.active_spans.get(key).map(|entry| {
            let span = entry.value();
            (
                span.trace_id_hex.clone(),
                span.span_id_hex.clone(),
                span.sampled,
                span.baggage.clone(),
            )
        })
    }
}

impl opentelemetry_sdk::trace::IdGenerator for RequestedIdGenerator {
    fn new_trace_id(&self) -> opentelemetry::TraceId {
        if let Some(trace_id) = REQUESTED_IDS.with(|requested| {
            requested
                .borrow_mut()
                .as_mut()
                .and_then(|requested| requested.trace_id.take())
        }) {
            return trace_id;
        }

        opentelemetry_sdk::trace::RandomIdGenerator::default().new_trace_id()
    }

    fn new_span_id(&self) -> opentelemetry::SpanId {
        if let Some(span_id) = REQUESTED_IDS.with(|requested| {
            requested
                .borrow_mut()
                .as_mut()
                .and_then(|requested| requested.span_id.take())
        }) {
            return span_id;
        }

        opentelemetry_sdk::trace::RandomIdGenerator::default().new_span_id()
    }
}

/// Build an OTLP resource from the service name
fn build_resource(service_name: String) -> Resource {
    Resource::builder().with_service_name(service_name).build()
}

/// Cached OTLP providers for a given config
pub struct OtlpProviderCache {
    pub logs_provider: Option<opentelemetry_sdk::logs::SdkLoggerProvider>,
    pub metrics_provider: Option<opentelemetry_sdk::metrics::SdkMeterProvider>,
    pub traces_provider: Option<opentelemetry_sdk::trace::SdkTracerProvider>,
    pub correlation: Arc<CorrelationContext>,
    pub metrics_instruments: HashMap<&'static str, CachedInstrument>,
    pub baggage_promotions: Vec<BaggageKeyPromotion>,
    pub baggage_tracker: DistinctValueTracker,
}

impl OtlpProviderCache {
    pub fn init(config: &OtlpBackendConfig) -> OtlpProviderCache {
        let resource = build_resource(config.service_name.clone());
        let correlation = Arc::new(CorrelationContext::new());

        let logs_provider = config.logs.as_ref().and_then(|sig| {
            build_logs_provider(sig, &config.no_verify, &resource, &sig.authorization)
        });

        let metrics_provider = config.metrics.as_ref().and_then(|sig| {
            build_metrics_provider(sig, &config.no_verify, &resource, &sig.authorization)
        });

        let traces_provider = config.traces.as_ref().and_then(|sig| {
            build_traces_provider(sig, &config.no_verify, &resource, &sig.authorization)
        });

        OtlpProviderCache {
            logs_provider,
            metrics_provider,
            traces_provider,
            correlation,
            metrics_instruments: HashMap::new(),
            baggage_promotions: config.baggage_promotions.clone(),
            baggage_tracker: DistinctValueTracker::new(),
        }
    }
}

fn build_logs_provider(
    sig: &SignalConfig,
    no_verify: &bool,
    resource: &Resource,
    authorization: &Option<String>,
) -> Option<opentelemetry_sdk::logs::SdkLoggerProvider> {
    use opentelemetry_otlp::LogExporter;
    use opentelemetry_sdk::logs::log_processor_with_async_runtime::BatchLogProcessor;

    let mut headers = http::HeaderMap::new();
    if let Some(auth) = authorization {
        headers.insert(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_str(auth).ok()?,
        );
    }

    let exporter: LogExporter = match sig.protocol.as_str() {
        "http/protobuf" => LogExporter::builder()
            .with_http()
            .with_http_client(HyperOtelClient::new(*no_verify).ok()?)
            .with_protocol(opentelemetry_otlp::Protocol::HttpBinary)
            .with_headers(
                headers
                    .into_iter()
                    .filter_map(|(n, v)| {
                        n.map(|n| {
                            (
                                n.as_str().to_string(),
                                String::from_utf8_lossy(v.as_bytes()).to_string(),
                            )
                        })
                    })
                    .collect(),
            )
            .with_endpoint(&sig.endpoint)
            .build()
            .ok()?,
        "http/json" => LogExporter::builder()
            .with_http()
            .with_http_client(HyperOtelClient::new(*no_verify).ok()?)
            .with_protocol(opentelemetry_otlp::Protocol::HttpJson)
            .with_headers(
                headers
                    .into_iter()
                    .filter_map(|(n, v)| {
                        n.map(|n| {
                            (
                                n.as_str().to_string(),
                                String::from_utf8_lossy(v.as_bytes()).to_string(),
                            )
                        })
                    })
                    .collect(),
            )
            .with_endpoint(&sig.endpoint)
            .build()
            .ok()?,
        _ => LogExporter::builder()
            .with_tonic()
            .with_channel(build_tonic_channel(&sig.endpoint, *no_verify)?)
            .with_metadata(tonic::metadata::MetadataMap::from_headers(headers))
            .build()
            .ok()?,
    };

    Some(
        opentelemetry_sdk::logs::SdkLoggerProvider::builder()
            .with_log_processor(
                BatchLogProcessor::builder(exporter, opentelemetry_sdk::runtime::Tokio).build(),
            )
            .with_resource(resource.clone())
            .build(),
    )
}

fn build_metrics_provider(
    sig: &SignalConfig,
    no_verify: &bool,
    resource: &Resource,
    authorization: &Option<String>,
) -> Option<opentelemetry_sdk::metrics::SdkMeterProvider> {
    use opentelemetry_otlp::MetricExporter;
    use opentelemetry_sdk::metrics::periodic_reader_with_async_runtime::PeriodicReader;

    let mut headers = http::HeaderMap::new();
    if let Some(auth) = authorization {
        headers.insert(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_str(auth).ok()?,
        );
    }

    let exporter: MetricExporter = match sig.protocol.as_str() {
        "http/protobuf" => MetricExporter::builder()
            .with_http()
            .with_http_client(HyperOtelClient::new(*no_verify).ok()?)
            .with_protocol(opentelemetry_otlp::Protocol::HttpBinary)
            .with_headers(
                headers
                    .into_iter()
                    .filter_map(|(n, v)| {
                        n.map(|n| {
                            (
                                n.as_str().to_string(),
                                String::from_utf8_lossy(v.as_bytes()).to_string(),
                            )
                        })
                    })
                    .collect(),
            )
            .with_endpoint(&sig.endpoint)
            .build()
            .ok()?,
        "http/json" => MetricExporter::builder()
            .with_http()
            .with_http_client(HyperOtelClient::new(*no_verify).ok()?)
            .with_protocol(opentelemetry_otlp::Protocol::HttpJson)
            .with_headers(
                headers
                    .into_iter()
                    .filter_map(|(n, v)| {
                        n.map(|n| {
                            (
                                n.as_str().to_string(),
                                String::from_utf8_lossy(v.as_bytes()).to_string(),
                            )
                        })
                    })
                    .collect(),
            )
            .with_endpoint(&sig.endpoint)
            .build()
            .ok()?,
        _ => MetricExporter::builder()
            .with_tonic()
            .with_channel(build_tonic_channel(&sig.endpoint, *no_verify)?)
            .with_metadata(tonic::metadata::MetadataMap::from_headers(headers))
            .build()
            .ok()?,
    };

    Some(
        opentelemetry_sdk::metrics::SdkMeterProvider::builder()
            .with_reader(
                PeriodicReader::builder(exporter, opentelemetry_sdk::runtime::Tokio)
                    .with_interval(std::time::Duration::from_secs(30))
                    .build(),
            )
            .with_resource(resource.clone())
            .build(),
    )
}

fn build_traces_provider(
    sig: &SignalConfig,
    no_verify: &bool,
    resource: &Resource,
    authorization: &Option<String>,
) -> Option<SdkTracerProvider> {
    use opentelemetry_otlp::SpanExporter;
    use opentelemetry_sdk::trace::span_processor_with_async_runtime::BatchSpanProcessor;

    let mut headers = http::HeaderMap::new();
    if let Some(auth) = authorization {
        headers.insert(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_str(auth).ok()?,
        );
    }

    let exporter: SpanExporter = match sig.protocol.as_str() {
        "http/protobuf" => SpanExporter::builder()
            .with_http()
            .with_http_client(HyperOtelClient::new(*no_verify).ok()?)
            .with_protocol(opentelemetry_otlp::Protocol::HttpBinary)
            .with_headers(
                headers
                    .into_iter()
                    .filter_map(|(n, v)| {
                        n.map(|n| {
                            (
                                n.as_str().to_string(),
                                String::from_utf8_lossy(v.as_bytes()).to_string(),
                            )
                        })
                    })
                    .collect(),
            )
            .with_endpoint(&sig.endpoint)
            .build()
            .ok()?,
        "http/json" => SpanExporter::builder()
            .with_http()
            .with_http_client(HyperOtelClient::new(*no_verify).ok()?)
            .with_protocol(opentelemetry_otlp::Protocol::HttpJson)
            .with_headers(
                headers
                    .into_iter()
                    .filter_map(|(n, v)| {
                        n.map(|n| {
                            (
                                n.as_str().to_string(),
                                String::from_utf8_lossy(v.as_bytes()).to_string(),
                            )
                        })
                    })
                    .collect(),
            )
            .with_endpoint(&sig.endpoint)
            .build()
            .ok()?,
        _ => SpanExporter::builder()
            .with_tonic()
            .with_channel(build_tonic_channel(&sig.endpoint, *no_verify)?)
            .with_metadata(tonic::metadata::MetadataMap::from_headers(headers))
            .build()
            .ok()?,
    };

    let sampler = build_sampler(&sig.sampling);

    Some(
        SdkTracerProvider::builder()
            .with_span_processor(
                BatchSpanProcessor::builder(exporter, opentelemetry_sdk::runtime::Tokio).build(),
            )
            .with_id_generator(RequestedIdGenerator)
            .with_resource(resource.clone())
            .with_sampler(sampler)
            .build(),
    )
}

pub enum CachedInstrument {
    F64Counter(opentelemetry::metrics::Counter<f64>),
    F64Gauge(opentelemetry::metrics::Gauge<f64>),
    F64Histogram(opentelemetry::metrics::Histogram<f64>),
    F64UpDownCounter(opentelemetry::metrics::UpDownCounter<f64>),
    I64Gauge(opentelemetry::metrics::Gauge<i64>),
    I64UpDownCounter(opentelemetry::metrics::UpDownCounter<i64>),
    U64Counter(opentelemetry::metrics::Counter<u64>),
    U64Gauge(opentelemetry::metrics::Gauge<u64>),
    U64Histogram(opentelemetry::metrics::Histogram<u64>),
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

pub fn emit_access_log(
    provider: &opentelemetry_sdk::logs::SdkLoggerProvider,
    event: &Arc<dyn AccessEvent>,
    log_config: &Arc<ServerConfigurationBlock>,
    registry: &Registry,
    promotions: &[BaggageKeyPromotion],
    log_style: LogStyle,
) {
    use opentelemetry::logs::{LogRecord, Logger, LoggerProvider};

    let logger = provider.logger("ferron.access");
    let mut record = logger.create_log_record();
    match log_style {
        LogStyle::Legacy => {
            if let Some(body) = format_access_event(event, log_config, registry) {
                record.set_body(AnyValue::String(body.into()));
            } else {
                record.set_body(AnyValue::String("<unknown access log>".into()));
            }
        }
        LogStyle::Modern => {
            record.set_body(AnyValue::String(
                format!("Access log ({})", event.protocol()).into(),
            ));
            // Set timestamp from the access event when available
            if let Some(time) = event.event_time() {
                record.set_timestamp(time);
            }
            // Map traditional access-log fields onto OTEL semantic-convention
            // attributes. Header fields become `http.request.header.<name>`.
            let mut visitor = OtelAccessAttributeVisitor::default();
            event.visit(&mut visitor);
            for (key, value) in visitor.attributes {
                record.add_attribute(key, value);
            }
        }
    }
    if let Some(trace_context) = event.trace_context() {
        if let (Ok(trace_id_str), Ok(span_id_str)) = (
            std::str::from_utf8(&trace_context.trace_id),
            std::str::from_utf8(&trace_context.span_id),
        ) {
            if let (Ok(trace_id), Ok(span_id)) = (
                opentelemetry::TraceId::from_hex(trace_id_str),
                opentelemetry::SpanId::from_hex(span_id_str),
            ) {
                record.set_trace_context(trace_id, span_id, trace_flags(trace_context.sampled));
            }
        }
    }

    // Promote configured baggage keys into access log attributes
    if let Some(baggage_str) = event.trace_context().and_then(|c| c.baggage.as_deref()) {
        let extracted = baggage::extract_promoted_keys(baggage_str, promotions, SignalSet::LOGS);
        for attr in extracted {
            record.add_attribute(attr.attribute_name, AnyValue::String(attr.value.into()));
        }
    }

    logger.emit(record);
}

/// Captures access-log fields as typed OTEL semantic-convention attributes.
///
/// This visitor drives [`AccessEvent::visit`] and translates the legacy
/// field names (e.g. `client_ip`, `status`, `header_user_agent`) into their
/// OTEL semantic-convention equivalents (e.g. `client.address`,
/// `http.response.status_code`, `http.request.header.user_agent`).
#[derive(Default)]
pub struct OtelAccessAttributeVisitor {
    pub attributes: Vec<(String, AnyValue)>,
}

impl OtelAccessAttributeVisitor {
    fn push(&mut self, key: impl Into<String>, value: AnyValue) {
        self.attributes.push((key.into(), value));
    }
}

impl AccessVisitor for OtelAccessAttributeVisitor {
    fn field_string(&mut self, name: &str, value: &str) {
        match name {
            "path" => self.push("url.path", AnyValue::String(value.to_string().into())),
            "path_and_query" => self.push("url.full", AnyValue::String(value.to_string().into())),
            "method" => self.push(
                "http.request.method",
                AnyValue::String(value.to_string().into()),
            ),
            "version" => self.push(
                "network.protocol.version",
                AnyValue::String(value.to_string().into()),
            ),
            "scheme" => self.push("url.scheme", AnyValue::String(value.to_string().into())),
            "client_ip" => self.push("client.address", AnyValue::String(value.to_string().into())),
            "server_ip" => self.push("server.address", AnyValue::String(value.to_string().into())),
            "auth_user" => self.push("user.name", AnyValue::String(value.to_string().into())),
            "timestamp"
            | "trace_id"
            | "span_id"
            | "client_ip_canonical"
            | "server_ip_canonical" => {
                // Drop legacy-only fields; modern telemetry consumers prefer the
                // standard attributes and the record timestamp.
            }
            "content_length" => {
                if let Ok(value) = str::parse::<i64>(value) {
                    self.push("http.response.body.size", AnyValue::Int(value))
                }
            }
            _ => {
                if let Some(header) = name.strip_prefix("header_") {
                    self.push(
                        format!("http.request.header.{}", header),
                        AnyValue::String(value.to_string().into()),
                    );
                } else {
                    self.push(
                        format!("ferron.legacy_field.{name}"),
                        AnyValue::String(value.to_string().into()),
                    );
                }
            }
        }
    }

    fn field_u64(&mut self, name: &str, value: u64) {
        match name {
            "status" => self.push(
                "http.response.status_code",
                AnyValue::Int(i64::try_from(value).unwrap_or(i64::MAX)),
            ),
            "client_port" => self.push(
                "client.port",
                AnyValue::Int(i64::try_from(value).unwrap_or(i64::MAX)),
            ),
            "server_port" => self.push(
                "server.port",
                AnyValue::Int(i64::try_from(value).unwrap_or(i64::MAX)),
            ),
            "content_length" => self.push(
                "http.response.body.size",
                AnyValue::Int(i64::try_from(value).unwrap_or(i64::MAX)),
            ),
            s => self.push(
                format!("ferron.legacy_field.{s}"),
                AnyValue::Int(i64::try_from(value).unwrap_or(i64::MAX)),
            ),
        }
    }

    fn field_f64(&mut self, name: &str, value: f64) {
        if name == "duration_secs" {
            self.push("http.server.request.duration", AnyValue::Double(value));
        } else {
            self.push(
                format!("ferron.legacy_field.{name}"),
                AnyValue::Double(value),
            );
        }
    }

    fn field_bool(&mut self, name: &str, value: bool) {
        self.push(
            format!("ferron.legacy_field.{name}"),
            AnyValue::Boolean(value),
        );
    }
}

/// Maximum length for a metric label value before it is hashed to prevent cardinality explosion.
const MAX_LABEL_VALUE_LEN: usize = 128;

/// Sanitize a metric label value to prevent high-cardinality telemetry poisoning.
///
/// Values longer than 128 characters are replaced with its hash.
/// Control characters are replaced with `?` to avoid log injection.
pub(crate) fn sanitize_label_value(s: &str) -> String {
    let s = s.trim();
    if s.len() <= MAX_LABEL_VALUE_LEN {
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

pub fn emit_metric(
    provider: &opentelemetry_sdk::metrics::SdkMeterProvider,
    event: &MetricEvent,
    instruments: &mut HashMap<&'static str, CachedInstrument>,
    promotions: &[BaggageKeyPromotion],
    tracker: &mut DistinctValueTracker,
) {
    use opentelemetry::metrics::MeterProvider;

    let meter = provider.meter("ferron");
    let mut attrs: Vec<KeyValue> = event
        .attributes
        .iter()
        .map(|(k, v)| {
            let value = match v {
                MetricAttributeValue::F64(val) => opentelemetry::Value::from(*val),
                MetricAttributeValue::I64(val) => opentelemetry::Value::from(*val),
                MetricAttributeValue::String(val) => {
                    opentelemetry::Value::from(sanitize_label_value(val))
                }
                MetricAttributeValue::StaticStr(val) => {
                    if val.len() > MAX_LABEL_VALUE_LEN {
                        opentelemetry::Value::from(sanitize_label_value(val))
                    } else {
                        opentelemetry::Value::from(*val)
                    }
                }
                MetricAttributeValue::Bool(val) => opentelemetry::Value::from(*val),
            };
            KeyValue::new(*k, value)
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
            attrs.push(KeyValue::new(attr.attribute_name, value));
        }
    }

    match (&event.ty, event.value) {
        (MetricType::Counter, MetricValue::F64(val)) => {
            let instrument = instruments.entry(event.name).or_insert_with(|| {
                let mut b = meter.f64_counter(event.name);
                if let Some(u) = event.unit {
                    b = b.with_unit(u);
                }
                if let Some(d) = event.description {
                    b = b.with_description(d);
                }
                CachedInstrument::F64Counter(b.build())
            });
            if let CachedInstrument::F64Counter(i) = instrument {
                i.add(val, &attrs);
            }
        }
        (MetricType::Counter, MetricValue::U64(val)) => {
            let instrument = instruments.entry(event.name).or_insert_with(|| {
                let mut b = meter.u64_counter(event.name);
                if let Some(u) = event.unit {
                    b = b.with_unit(u);
                }
                if let Some(d) = event.description {
                    b = b.with_description(d);
                }
                CachedInstrument::U64Counter(b.build())
            });
            if let CachedInstrument::U64Counter(i) = instrument {
                i.add(val, &attrs);
            }
        }
        (MetricType::UpDownCounter, MetricValue::F64(val)) => {
            let instrument = instruments.entry(event.name).or_insert_with(|| {
                let mut b = meter.f64_up_down_counter(event.name);
                if let Some(u) = event.unit {
                    b = b.with_unit(u);
                }
                if let Some(d) = event.description {
                    b = b.with_description(d);
                }
                CachedInstrument::F64UpDownCounter(b.build())
            });
            if let CachedInstrument::F64UpDownCounter(i) = instrument {
                i.add(val, &attrs);
            }
        }
        (MetricType::UpDownCounter, MetricValue::I64(val)) => {
            let instrument = instruments.entry(event.name).or_insert_with(|| {
                let mut b = meter.i64_up_down_counter(event.name);
                if let Some(u) = event.unit {
                    b = b.with_unit(u);
                }
                if let Some(d) = event.description {
                    b = b.with_description(d);
                }
                CachedInstrument::I64UpDownCounter(b.build())
            });
            if let CachedInstrument::I64UpDownCounter(i) = instrument {
                i.add(val, &attrs);
            }
        }
        (MetricType::Gauge, MetricValue::F64(val)) => {
            let instrument = instruments.entry(event.name).or_insert_with(|| {
                let mut b = meter.f64_gauge(event.name);
                if let Some(u) = event.unit {
                    b = b.with_unit(u);
                }
                if let Some(d) = event.description {
                    b = b.with_description(d);
                }
                CachedInstrument::F64Gauge(b.build())
            });
            if let CachedInstrument::F64Gauge(i) = instrument {
                i.record(val, &attrs);
            }
        }
        (MetricType::Gauge, MetricValue::I64(val)) => {
            let instrument = instruments.entry(event.name).or_insert_with(|| {
                let mut b = meter.i64_gauge(event.name);
                if let Some(u) = event.unit {
                    b = b.with_unit(u);
                }
                if let Some(d) = event.description {
                    b = b.with_description(d);
                }
                CachedInstrument::I64Gauge(b.build())
            });
            if let CachedInstrument::I64Gauge(i) = instrument {
                i.record(val, &attrs);
            }
        }
        (MetricType::Gauge, MetricValue::U64(val)) => {
            let instrument = instruments.entry(event.name).or_insert_with(|| {
                let mut b = meter.u64_gauge(event.name);
                if let Some(u) = event.unit {
                    b = b.with_unit(u);
                }
                if let Some(d) = event.description {
                    b = b.with_description(d);
                }
                CachedInstrument::U64Gauge(b.build())
            });
            if let CachedInstrument::U64Gauge(i) = instrument {
                i.record(val, &attrs);
            }
        }
        (MetricType::Histogram(buckets), MetricValue::F64(val)) => {
            let instrument = instruments.entry(event.name).or_insert_with(|| {
                let mut b = meter.f64_histogram(event.name);
                if let Some(u) = event.unit {
                    b = b.with_unit(u);
                }
                if let Some(ref bkt) = buckets {
                    b = b.with_boundaries(bkt.to_vec());
                }
                if let Some(d) = event.description {
                    b = b.with_description(d);
                }
                CachedInstrument::F64Histogram(b.build())
            });
            if let CachedInstrument::F64Histogram(i) = instrument {
                i.record(val, &attrs);
            }
        }
        (MetricType::Histogram(buckets), MetricValue::U64(val)) => {
            let instrument = instruments.entry(event.name).or_insert_with(|| {
                let mut b = meter.u64_histogram(event.name);
                if let Some(u) = event.unit {
                    b = b.with_unit(u);
                }
                if let Some(ref bkt) = buckets {
                    b = b.with_boundaries(bkt.to_vec());
                }
                if let Some(d) = event.description {
                    b = b.with_description(d);
                }
                CachedInstrument::U64Histogram(b.build())
            });
            if let CachedInstrument::U64Histogram(i) = instrument {
                i.record(val, &attrs);
            }
        }
        _ => {}
    }
}

pub fn emit_trace(
    provider: &SdkTracerProvider,
    event: &TraceEvent,
    correlation: &CorrelationContext,
    promotions: &[BaggageKeyPromotion],
) {
    use opentelemetry::trace::{Span, SpanBuilder, Tracer};

    let tracer = provider.tracer("ferron");

    match event {
        TraceEvent::StartSpan {
            key,
            name,
            parent,
            trace_context,
            builder_attributes,
            attributes,
        } => {
            let mut builder = SpanBuilder::from_name(name.clone());

            // Set SpanKind::Server for HTTP request spans
            if name.as_ref() == "ferron.request" {
                builder = builder.with_kind(SpanKind::Server);
            }

            // Set builder-level attributes (visible to the sampler)
            if !builder_attributes.is_empty() {
                let otel_attrs: Vec<KeyValue> = builder_attributes
                    .iter()
                    .map(|(k, v)| {
                        let key: &'static str = match k {
                            Cow::Borrowed(s) => s,
                            Cow::Owned(s) => leak_string(s.clone()),
                        };
                        trace_kv(key, v)
                    })
                    .collect();
                builder = builder.with_attributes(otel_attrs);
            }

            let requested_ids = trace_context.as_ref().and_then(parse_requested_ids);
            let mut span = with_requested_ids(requested_ids, || {
                if let Some(parent_val) = parent {
                    if let Some(parent_cx) = build_parent_context(correlation, parent_val) {
                        tracer.build_with_context(builder, &parent_cx)
                    } else {
                        tracer.build(builder)
                    }
                } else {
                    tracer.build(builder)
                }
            });

            // Set semantic convention attributes (post-build, not visible to sampler)
            for (key, value) in attributes {
                span.set_attribute(trace_kv(key, value));
            }

            // Promote configured baggage keys into span attributes
            if let Some(baggage_str) = trace_context.as_ref().and_then(|c| c.baggage.as_deref()) {
                let extracted =
                    baggage::extract_promoted_keys(baggage_str, promotions, SignalSet::TRACES);
                for attr in extracted {
                    span.set_attribute(KeyValue::new(attr.attribute_name, attr.value));
                }
            }

            let trace_id_hex = span.span_context().trace_id().to_string();
            let span_id_hex = span.span_context().span_id().to_string();
            let sampled = span.span_context().trace_flags().is_sampled();
            let baggage = trace_context.as_ref().and_then(|c| c.baggage.clone());
            correlation.insert_span(
                key.clone(),
                trace_id_hex,
                span_id_hex,
                sampled,
                span,
                baggage,
            );
        }
        TraceEvent::EndSpan {
            key,
            name: _,
            error,
            attributes,
        } => {
            if let Some(mut active_span) = correlation.remove_span(key) {
                // Apply any final attributes (e.g. http.response.status_code)
                for (key, value) in attributes {
                    active_span.span.set_attribute(trace_kv(key, value));
                }
                if let Some(error_desc) = error {
                    active_span
                        .span
                        .set_status(opentelemetry::trace::Status::error(error_desc.clone()));
                }
                active_span.span.end();
            }
        }
    }
}

pub fn emit_log(
    provider: &opentelemetry_sdk::logs::SdkLoggerProvider,
    event: &LogEvent,
    promotions: &[BaggageKeyPromotion],
    log_style: LogStyle,
) {
    use opentelemetry::logs::{LogRecord, Logger, LoggerProvider, Severity};

    let logger = provider.logger("ferron");
    let mut record = logger.create_log_record();

    // In modern mode the log body is the short OTEL summary and per-event
    // attributes are published as typed AnyValues. In legacy mode the body is
    // the human-readable message and attributes are not exposed.
    match log_style {
        LogStyle::Legacy => {
            record.set_body(AnyValue::String(event.message.clone().into()));
            record.add_attribute("log.target", event.target);
        }
        LogStyle::Modern => {
            record.set_body(AnyValue::String(event.summary.as_ref().to_string().into()));
            record.add_attribute("log.target", event.target);
            for (key, value) in &event.attributes {
                record.add_attribute(*key, log_attribute_to_anyvalue(value));
            }
        }
    }

    record.set_severity_number(match event.level {
        LogLevel::Error => Severity::Error,
        LogLevel::Warn => Severity::Warn,
        LogLevel::Info => Severity::Info,
        LogLevel::Debug => Severity::Debug,
    });
    record.set_severity_text(match event.level {
        LogLevel::Error => "ERROR",
        LogLevel::Warn => "WARN",
        LogLevel::Info => "INFO",
        LogLevel::Debug => "DEBUG",
    });
    if let Some(trace_context) = &event.trace_context {
        if let (Ok(trace_id_str), Ok(span_id_str)) = (
            std::str::from_utf8(&trace_context.trace_id),
            std::str::from_utf8(&trace_context.span_id),
        ) {
            if let (Ok(trace_id), Ok(span_id)) = (
                opentelemetry::TraceId::from_hex(trace_id_str),
                opentelemetry::SpanId::from_hex(span_id_str),
            ) {
                record.set_trace_context(trace_id, span_id, trace_flags(trace_context.sampled));
            }
        }
    }

    // Promote configured baggage keys into log record attributes
    if let Some(baggage_str) = event
        .trace_context
        .as_ref()
        .and_then(|c| c.baggage.as_deref())
    {
        let extracted = baggage::extract_promoted_keys(baggage_str, promotions, SignalSet::LOGS);
        for attr in extracted {
            record.add_attribute(attr.attribute_name, AnyValue::String(attr.value.into()));
        }
    }

    logger.emit(record);
}

/// Convert a [`LogAttributeValue`] into an OTEL [`AnyValue`] preserving its
/// underlying type (string, bool, integer, float).
fn log_attribute_to_anyvalue(value: &LogAttributeValue) -> AnyValue {
    match value {
        LogAttributeValue::String(s) => AnyValue::String(s.clone().into()),
        LogAttributeValue::StaticStr(s) => AnyValue::String((*s).into()),
        LogAttributeValue::Bool(b) => AnyValue::Boolean(*b),
        LogAttributeValue::I64(i) => AnyValue::Int(*i),
        LogAttributeValue::F64(f) => AnyValue::Double(*f),
    }
}

fn build_parent_context(
    correlation: &CorrelationContext,
    parent: &Parent,
) -> Option<opentelemetry::Context> {
    use opentelemetry::trace::{SpanContext, SpanId, TraceContextExt, TraceId, TraceState};

    let (trace_id_hex, span_id_hex, sampled, baggage) = match parent {
        Parent::ByKey(parent_key) => {
            let (trace_id_hex, span_id_hex, sampled, baggage) =
                correlation.get_parent_ids(parent_key)?;
            (trace_id_hex, span_id_hex, Some(sampled), baggage)
        }
        Parent::ById {
            trace_id,
            span_id,
            sampled,
            baggage,
        } => (trace_id.clone(), span_id.clone(), *sampled, baggage.clone()),
    };

    let (trace_id, span_id) = (
        TraceId::from_hex(&trace_id_hex).ok()?,
        SpanId::from_hex(&span_id_hex).ok()?,
    );
    let parent_ctx = SpanContext::new(
        trace_id,
        span_id,
        trace_flags(sampled).unwrap_or_default(),
        true,
        TraceState::default(),
    );
    let mut context = opentelemetry::Context::new().with_remote_span_context(parent_ctx);
    if let Some(baggage) = baggage {
        // Parse baggage values
        let mut baggage_vec = Vec::new();
        for item in baggage.split(',') {
            let item = item.trim();
            if !item.is_empty() {
                let (kv, metadata) = if let Some(idx) = item.find(';') {
                    (&item[..idx], Some(&item[idx + 1..]))
                } else {
                    (item, None)
                };
                let Some((key, value)) = kv.split_once("=") else {
                    continue;
                };
                let metadata = if let Some(metadata) = metadata {
                    opentelemetry::baggage::BaggageMetadata::from(metadata)
                } else {
                    opentelemetry::baggage::BaggageMetadata::default()
                };
                let key = opentelemetry::Key::from(key.trim_end().to_owned());
                let Some(value) = urlencoding::decode(value.trim_start())
                    .ok()
                    .map(|v| opentelemetry::StringValue::from(v.to_string()))
                else {
                    continue;
                };
                baggage_vec.push((key, (value, metadata)));
            }
        }
        context = context.with_baggage(opentelemetry::baggage::Baggage::from_iter(baggage_vec));
    }
    Some(context)
}

fn trace_flags(sampled: Option<bool>) -> Option<opentelemetry::TraceFlags> {
    sampled.map(|sampled| {
        if sampled {
            opentelemetry::TraceFlags::SAMPLED
        } else {
            opentelemetry::TraceFlags::default()
        }
    })
}

fn parse_requested_ids(
    trace_context: &ferron_observability::EventTraceContext,
) -> Option<RequestedIds> {
    let trace_id_str = std::str::from_utf8(&trace_context.trace_id).ok()?;
    let span_id_str = std::str::from_utf8(&trace_context.span_id).ok()?;
    Some(RequestedIds {
        trace_id: opentelemetry::TraceId::from_hex(trace_id_str).ok(),
        span_id: opentelemetry::SpanId::from_hex(span_id_str).ok(),
    })
}

fn with_requested_ids<T>(requested_ids: Option<RequestedIds>, f: impl FnOnce() -> T) -> T {
    REQUESTED_IDS.with(|current| {
        let previous = current.replace(requested_ids);
        let result = f();
        current.replace(previous);
        result
    })
}

/// Convert a TraceAttributeValue into an OTEL KeyValue.
fn trace_kv(key: &'static str, value: &TraceAttributeValue) -> KeyValue {
    match value {
        TraceAttributeValue::String(s) => KeyValue::new(key, s.clone()),
        TraceAttributeValue::StaticStr(s) => KeyValue::new(key, *s),
        TraceAttributeValue::Bool(b) => KeyValue::new(key, *b),
        TraceAttributeValue::I64(i) => KeyValue::new(key, *i),
        TraceAttributeValue::F64(f) => KeyValue::new(key, *f),
    }
}

/// Leak a String to get a `&'static str`. Used for converting owned Cow keys
/// to static str for `trace_kv`. The leaked memory is acceptable because
/// provider caches live for the lifetime of the server.
fn leak_string(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}
