use std::{collections::HashMap, sync::Arc};

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
    /// Active spans: span_name -> (trace_id_hex, span)
    active_spans: DashMap<String, (String, opentelemetry_sdk::trace::Span)>,
}

impl CorrelationContext {
    pub fn new() -> Self {
        Self {
            active_spans: DashMap::new(),
        }
    }

    pub fn insert_span(
        &self,
        name: impl Into<String>,
        trace_id_hex: String,
        span: opentelemetry_sdk::trace::Span,
    ) {
        self.active_spans.insert(name.into(), (trace_id_hex, span));
    }

    pub fn remove_span(&self, name: &str) -> Option<(String, opentelemetry_sdk::trace::Span)> {
        self.active_spans.remove(name).map(|(_, v)| v)
    }

    /// Look up an active span's trace and span ID for use as a parent.
    pub fn get_parent_ids(&self, name: &str) -> Option<(String, String)> {
        use opentelemetry::trace::Span;
        self.active_spans.get(name).map(|entry| {
            let (trace_id_hex, span) = entry.value();
            (
                trace_id_hex.clone(),
                span.span_context().span_id().to_string(),
            )
        })
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
            let _guard = set_otlp_headers_temporarily("LOGS", &sig.authorization);
            build_logs_provider(sig, &config.no_verify, &resource)
        });

        let metrics_provider = config.metrics.as_ref().and_then(|sig| {
            let _guard = set_otlp_headers_temporarily("METRICS", &sig.authorization);
            build_metrics_provider(sig, &config.no_verify, &resource)
        });

        let traces_provider = config.traces.as_ref().and_then(|sig| {
            let _guard = set_otlp_headers_temporarily("TRACES", &sig.authorization);
            build_traces_provider(sig, &config.no_verify, &resource)
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
) -> Option<opentelemetry_sdk::logs::SdkLoggerProvider> {
    use opentelemetry_otlp::LogExporter;
    use opentelemetry_sdk::logs::log_processor_with_async_runtime::BatchLogProcessor;

    let exporter: LogExporter = match sig.protocol.as_str() {
        "http/protobuf" => LogExporter::builder()
            .with_http()
            .with_http_client(HyperOtelClient::new(*no_verify).ok()?)
            .with_protocol(opentelemetry_otlp::Protocol::HttpBinary)
            .with_endpoint(&sig.endpoint)
            .build()
            .ok()?,
        "http/json" => LogExporter::builder()
            .with_http()
            .with_http_client(HyperOtelClient::new(*no_verify).ok()?)
            .with_protocol(opentelemetry_otlp::Protocol::HttpJson)
            .with_endpoint(&sig.endpoint)
            .build()
            .ok()?,
        _ => LogExporter::builder()
            .with_tonic()
            .with_channel(build_tonic_channel(&sig.endpoint, *no_verify)?)
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
) -> Option<opentelemetry_sdk::metrics::SdkMeterProvider> {
    use opentelemetry_otlp::MetricExporter;
    use opentelemetry_sdk::metrics::periodic_reader_with_async_runtime::PeriodicReader;

    let exporter: MetricExporter = match sig.protocol.as_str() {
        "http/protobuf" => MetricExporter::builder()
            .with_http()
            .with_http_client(HyperOtelClient::new(*no_verify).ok()?)
            .with_protocol(opentelemetry_otlp::Protocol::HttpBinary)
            .with_endpoint(&sig.endpoint)
            .build()
            .ok()?,
        "http/json" => MetricExporter::builder()
            .with_http()
            .with_http_client(HyperOtelClient::new(*no_verify).ok()?)
            .with_protocol(opentelemetry_otlp::Protocol::HttpJson)
            .with_endpoint(&sig.endpoint)
            .build()
            .ok()?,
        _ => MetricExporter::builder()
            .with_tonic()
            .with_channel(build_tonic_channel(&sig.endpoint, *no_verify)?)
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
) -> Option<opentelemetry_sdk::trace::SdkTracerProvider> {
    use opentelemetry_otlp::SpanExporter;
    use opentelemetry_sdk::trace::span_processor_with_async_runtime::BatchSpanProcessor;

    let exporter: SpanExporter = match sig.protocol.as_str() {
        "http/protobuf" => SpanExporter::builder()
            .with_http()
            .with_http_client(HyperOtelClient::new(*no_verify).ok()?)
            .with_protocol(opentelemetry_otlp::Protocol::HttpBinary)
            .with_endpoint(&sig.endpoint)
            .build()
            .ok()?,
        "http/json" => SpanExporter::builder()
            .with_http()
            .with_http_client(HyperOtelClient::new(*no_verify).ok()?)
            .with_protocol(opentelemetry_otlp::Protocol::HttpJson)
            .with_endpoint(&sig.endpoint)
            .build()
            .ok()?,
        _ => SpanExporter::builder()
            .with_tonic()
            .with_channel(build_tonic_channel(&sig.endpoint, *no_verify)?)
            .build()
            .ok()?,
    };

    Some(
        opentelemetry_sdk::trace::SdkTracerProvider::builder()
            .with_span_processor(
                BatchSpanProcessor::builder(exporter, opentelemetry_sdk::runtime::Tokio).build(),
            )
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

/// Set OTEL env vars for the current signal's headers, build the exporter, then clear them.
/// This is called during provider initialization in a single-threaded context.
fn set_otlp_headers_temporarily(signal: &str, authorization: &Option<String>) -> TempHeaderGuard {
    let var_name = format!("OTEL_EXPORTER_OTLP_{signal}_HEADERS");
    let old_val = std::env::var(&var_name).ok();

    if let Some(auth) = authorization {
        std::env::set_var(&var_name, format!("Authorization={auth}"));
    }

    TempHeaderGuard {
        var_name,
        old_val,
        had_auth: authorization.is_some(),
    }
}

struct TempHeaderGuard {
    #[allow(dead_code)]
    var_name: String,
    #[allow(dead_code)]
    old_val: Option<String>,
    #[allow(dead_code)]
    had_auth: bool,
}

impl TempHeaderGuard {
    #[allow(dead_code)]
    fn cleanup(self) {
        if self.had_auth {
            if let Some(old) = self.old_val {
                std::env::set_var(&self.var_name, old);
            } else {
                std::env::remove_var(&self.var_name);
            }
        }
    }
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
    logger.emit(record);
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
            KeyValue::new(
                *k,
                match v {
                    MetricAttributeValue::F64(val) => opentelemetry::Value::from(*val),
                    MetricAttributeValue::I64(val) => opentelemetry::Value::from(*val),
                    MetricAttributeValue::String(val) => opentelemetry::Value::from(val.clone()),
                    MetricAttributeValue::StaticStr(val) => opentelemetry::Value::from(*val),
                    MetricAttributeValue::Bool(val) => opentelemetry::Value::from(*val),
                },
            )
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
                    b = b.with_boundaries(bkt.clone());
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
                    b = b.with_boundaries(bkt.clone());
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
    use opentelemetry::trace::{
        Span, SpanContext, SpanId, TraceContextExt, TraceFlags, TraceId, TraceState, Tracer,
    };
    use opentelemetry::Context;

    let tracer = provider.tracer("ferron");

    match event {
        TraceEvent::StartSpan {
            name,
            parent,
            attributes,
        } => {
            let mut span = if let Some(parent_val) = parent {
                match parent_val {
                    Parent::ByName(parent_name) => {
                        // Look up the parent span's trace_id and span_id by name
                        if let Some((trace_id_hex, parent_span_id_hex)) =
                            correlation.get_parent_ids(parent_name)
                        {
                            if let (Ok(trace_id), Ok(span_id)) = (
                                TraceId::from_hex(&trace_id_hex),
                                SpanId::from_hex(&parent_span_id_hex),
                            ) {
                                let parent_ctx = SpanContext::new(
                                    trace_id,
                                    span_id,
                                    TraceFlags::SAMPLED,
                                    true,
                                    TraceState::default(),
                                );
                                let parent_cx = Context::new().with_remote_span_context(parent_ctx);
                                tracer.start_with_context(name.clone(), &parent_cx)
                            } else {
                                tracer.start(name.clone())
                            }
                        } else {
                            tracer.start(name.clone())
                        }
                    }
                    Parent::ById {
                        trace_id: trace_id_hex,
                        span_id: parent_span_id_hex,
                    } => {
                        if let (Ok(trace_id), Ok(span_id)) = (
                            TraceId::from_hex(trace_id_hex),
                            SpanId::from_hex(parent_span_id_hex),
                        ) {
                            let parent_ctx = SpanContext::new(
                                trace_id,
                                span_id,
                                TraceFlags::SAMPLED,
                                true,
                                TraceState::default(),
                            );
                            let parent_cx = Context::new().with_remote_span_context(parent_ctx);
                            tracer.start_with_context(name.clone(), &parent_cx)
                        } else {
                            tracer.start(name.clone())
                        }
                    }
                }
            } else {
                tracer.start(name.clone())
            };

            // Set semantic convention attributes
            for (key, value) in attributes {
                span.set_attribute(trace_kv(key, value));
            }

            let trace_id_hex = span.span_context().trace_id().to_string();
            correlation.insert_span(name.clone(), trace_id_hex, span);
        }
        TraceEvent::EndSpan {
            name,
            error,
            attributes,
        } => {
            if let Some((_, mut span)) = correlation.remove_span(name) {
                // Apply any final attributes (e.g. http.response.status_code)
                for (key, value) in attributes {
                    span.set_attribute(trace_kv(key, value));
                }
                if let Some(error_desc) = error {
                    span.set_status(opentelemetry::trace::Status::error(error_desc.clone()));
                }
                span.end();
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

    logger.emit(record);
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
