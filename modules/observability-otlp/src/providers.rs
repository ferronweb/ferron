use std::{cell::RefCell, collections::HashMap, sync::Arc};

use dashmap::DashMap;
use ferron_core::{config::ServerConfigurationBlock, registry::Registry};
use ferron_observability::{
    AccessEvent, LogEvent, LogFormatterContext, LogLevel, MetricAttributeValue, MetricEvent,
    MetricType, MetricValue, Parent, TraceAttributeValue, TraceEvent,
};
use opentelemetry::{logs::AnyValue, trace::TracerProvider, KeyValue};
use opentelemetry_otlp::{WithExportConfig, WithHttpConfig, WithTonicConfig};
use opentelemetry_sdk::Resource;

use crate::{
    client::{build_tonic_channel, HyperOtelClient},
    config::{OtlpBackendConfig, SignalConfig},
};

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
    ) {
        self.active_spans.insert(
            key.into(),
            ActiveSpan {
                trace_id_hex,
                span_id_hex,
                sampled,
                span,
            },
        );
    }

    fn remove_span(&self, key: &str) -> Option<ActiveSpan> {
        self.active_spans.remove(key).map(|(_, v)| v)
    }

    /// Look up an active span's trace and span ID for use as a parent.
    pub fn get_parent_ids(&self, key: &str) -> Option<(String, String, bool)> {
        self.active_spans.get(key).map(|entry| {
            let span = entry.value();
            (
                span.trace_id_hex.clone(),
                span.span_id_hex.clone(),
                span.sampled,
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
) -> Option<opentelemetry_sdk::trace::SdkTracerProvider> {
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

    Some(
        opentelemetry_sdk::trace::SdkTracerProvider::builder()
            .with_span_processor(
                BatchSpanProcessor::builder(exporter, opentelemetry_sdk::runtime::Tokio).build(),
            )
            .with_id_generator(RequestedIdGenerator)
            .with_resource(resource.clone())
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
) {
    use opentelemetry::logs::{LogRecord, Logger, LoggerProvider};

    let logger = provider.logger("ferron.access");
    let mut record = logger.create_log_record();
    if let Some(body) = format_access_event(event, log_config, registry) {
        record.set_body(AnyValue::String(body.into()));
    } else {
        record.set_body(AnyValue::String("<unknown access log>".into()));
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

    logger.emit(record);
}

/// Maximum length for a metric label value before it is hashed to prevent cardinality explosion.
const MAX_LABEL_VALUE_LEN: usize = 128;

/// Sanitize a metric label value to prevent high-cardinality telemetry poisoning.
///
/// Values longer than 128 characters are replaced with a deterministic hash.
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
) {
    use opentelemetry::metrics::MeterProvider;

    let meter = provider.meter("ferron");
    let attrs: Vec<KeyValue> = event
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
    provider: &opentelemetry_sdk::trace::SdkTracerProvider,
    event: &TraceEvent,
    correlation: &CorrelationContext,
) {
    use opentelemetry::trace::{Span, SpanBuilder, Tracer};

    let tracer = provider.tracer("ferron");

    match event {
        TraceEvent::StartSpan {
            key,
            name,
            parent,
            trace_context,
            attributes,
        } => {
            let builder = SpanBuilder::from_name(name.clone());
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

            // Set semantic convention attributes
            for (key, value) in attributes {
                span.set_attribute(trace_kv(key, value));
            }

            let trace_id_hex = span.span_context().trace_id().to_string();
            let span_id_hex = span.span_context().span_id().to_string();
            let sampled = span.span_context().trace_flags().is_sampled();
            correlation.insert_span(key.clone(), trace_id_hex, span_id_hex, sampled, span);
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

pub fn emit_log(provider: &opentelemetry_sdk::logs::SdkLoggerProvider, event: &LogEvent) {
    use opentelemetry::logs::{LogRecord, Logger, LoggerProvider, Severity};

    let logger = provider.logger("ferron");
    let mut record = logger.create_log_record();

    record.set_body(AnyValue::String(event.message.clone().into()));
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
    record.add_attribute("log.target", event.target);
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

    logger.emit(record);
}

fn build_parent_context(
    correlation: &CorrelationContext,
    parent: &Parent,
) -> Option<opentelemetry::Context> {
    use opentelemetry::trace::{SpanContext, SpanId, TraceContextExt, TraceId, TraceState};

    let (trace_id_hex, span_id_hex, sampled) = match parent {
        Parent::ByKey(parent_key) => {
            let (trace_id_hex, span_id_hex, sampled) = correlation.get_parent_ids(parent_key)?;
            (trace_id_hex, span_id_hex, Some(sampled))
        }
        Parent::ById {
            trace_id,
            span_id,
            sampled,
        } => (trace_id.clone(), span_id.clone(), *sampled),
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
    Some(opentelemetry::Context::new().with_remote_span_context(parent_ctx))
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
