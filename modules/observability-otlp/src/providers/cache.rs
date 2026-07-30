use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use ferron_observability::baggage::{BaggageKeyPromotion, DistinctValueTracker};
use ferron_observability::{CompositeEventSink, Event, LogAttributeValue, LogEvent, LogLevel};

use crate::client::{build_tonic_channel, HyperOtelClient};
use crate::config::{OtlpBackendConfig, SignalConfig};

use super::context::{build_resource, CorrelationContext, RequestedIdGenerator};
use super::metrics::CachedInstrument;

/// Cached OTLP providers for a given config
pub struct OtlpProviderCache {
    pub logs_provider: Option<opentelemetry_sdk::logs::SdkLoggerProvider>,
    pub metrics_provider: Option<opentelemetry_sdk::metrics::SdkMeterProvider>,
    pub traces_provider: Option<opentelemetry_sdk::trace::SdkTracerProvider>,
    pub correlation: CorrelationContext,
    pub metrics_instruments: HashMap<&'static str, CachedInstrument>,
    pub baggage_promotions: Vec<BaggageKeyPromotion>,
    pub baggage_tracker: DistinctValueTracker,
    /// Control plane metadata to include in all observability signals.
    pub control_plane_metadata: Option<Arc<BTreeMap<String, String>>>,
}

impl OtlpProviderCache {
    pub fn init(
        config: &OtlpBackendConfig,
        event_sink: Option<&CompositeEventSink>,
        control_plane_metadata: Option<Arc<BTreeMap<String, String>>>,
    ) -> OtlpProviderCache {
        let resource = build_resource(config.service_name.clone());
        let correlation = CorrelationContext::new();

        let logs_provider = config.logs.as_ref().and_then(|sig| {
            let result = build_logs_provider(
                sig,
                &config.no_verify,
                &resource,
                sig.authorization
                    .as_deref()
                    .or(config.authorization.as_deref()),
            );
            if let (Err(err), Some(sink)) = (&result, event_sink) {
                sink.emit(Event::Log(LogEvent {
                    level: LogLevel::Warn,
                    message: format!("Error with logs provider: {err}"),
                    summary: "Error with logs provider".into(),
                    target: "ferron-observability-otlp",
                    attributes: vec![("error.message", LogAttributeValue::String(err.to_string()))],
                    trace_context: None,
                    control_plane_metadata: None,
                }));
            }
            result.ok()
        });

        let metrics_provider = config.metrics.as_ref().and_then(|sig| {
            let result = build_metrics_provider(
                sig,
                &config.no_verify,
                &resource,
                sig.authorization
                    .as_deref()
                    .or(config.authorization.as_deref()),
            );
            if let (Err(err), Some(sink)) = (&result, event_sink) {
                sink.emit(Event::Log(LogEvent {
                    level: LogLevel::Warn,
                    message: format!("Error with metrics provider: {err}"),
                    summary: "Error with metrics provider".into(),
                    target: "ferron-observability-otlp",
                    attributes: vec![("error.message", LogAttributeValue::String(err.to_string()))],
                    trace_context: None,
                    control_plane_metadata: None,
                }));
            }
            result.ok()
        });

        let traces_provider = config.traces.as_ref().and_then(|sig| {
            let result = build_traces_provider(
                sig,
                &config.no_verify,
                &resource,
                sig.authorization
                    .as_deref()
                    .or(config.authorization.as_deref()),
            );
            if let (Err(err), Some(sink)) = (&result, event_sink) {
                sink.emit(Event::Log(LogEvent {
                    level: LogLevel::Warn,
                    message: format!("Error with traces provider: {err}"),
                    summary: "Error with traces provider".into(),
                    target: "ferron-observability-otlp",
                    attributes: vec![("error.message", LogAttributeValue::String(err.to_string()))],
                    trace_context: None,
                    control_plane_metadata: None,
                }));
            }
            result.ok()
        });

        OtlpProviderCache {
            logs_provider,
            metrics_provider,
            traces_provider,
            correlation,
            metrics_instruments: HashMap::new(),
            baggage_promotions: config.baggage_promotions.clone(),
            baggage_tracker: DistinctValueTracker::new(),
            control_plane_metadata,
        }
    }
}

fn build_logs_provider(
    sig: &SignalConfig,
    no_verify: &bool,
    resource: &opentelemetry_sdk::Resource,
    authorization: Option<&str>,
) -> Result<opentelemetry_sdk::logs::SdkLoggerProvider, Box<dyn std::error::Error + Send + Sync>> {
    use opentelemetry_otlp::{LogExporter, WithExportConfig, WithHttpConfig, WithTonicConfig};
    use opentelemetry_sdk::logs::log_processor_with_async_runtime::BatchLogProcessor;

    let mut headers = http::HeaderMap::new();
    if let Some(auth) = authorization {
        headers.insert(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_str(auth)?,
        );
    }

    let exporter: LogExporter = match sig.protocol.as_str() {
        "http/protobuf" => LogExporter::builder()
            .with_http()
            .with_http_client(HyperOtelClient::new(*no_verify)?)
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
            .build()?,
        "http/json" => LogExporter::builder()
            .with_http()
            .with_http_client(HyperOtelClient::new(*no_verify)?)
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
            .build()?,
        _ => LogExporter::builder()
            .with_tonic()
            .with_channel(build_tonic_channel(&sig.endpoint, *no_verify)?)
            .with_metadata(tonic::metadata::MetadataMap::from_headers(headers))
            .build()?,
    };

    Ok(opentelemetry_sdk::logs::SdkLoggerProvider::builder()
        .with_log_processor(
            BatchLogProcessor::builder(exporter, opentelemetry_sdk::runtime::Tokio).build(),
        )
        .with_resource(resource.clone())
        .build())
}

fn build_metrics_provider(
    sig: &SignalConfig,
    no_verify: &bool,
    resource: &opentelemetry_sdk::Resource,
    authorization: Option<&str>,
) -> Result<opentelemetry_sdk::metrics::SdkMeterProvider, Box<dyn std::error::Error + Send + Sync>>
{
    use opentelemetry_otlp::{MetricExporter, WithExportConfig, WithHttpConfig, WithTonicConfig};
    use opentelemetry_sdk::metrics::periodic_reader_with_async_runtime::PeriodicReader;

    let mut headers = http::HeaderMap::new();
    if let Some(auth) = authorization {
        headers.insert(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_str(auth)?,
        );
    }

    let exporter: MetricExporter = match sig.protocol.as_str() {
        "http/protobuf" => MetricExporter::builder()
            .with_http()
            .with_http_client(HyperOtelClient::new(*no_verify)?)
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
            .build()?,
        "http/json" => MetricExporter::builder()
            .with_http()
            .with_http_client(HyperOtelClient::new(*no_verify)?)
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
            .build()?,
        _ => MetricExporter::builder()
            .with_tonic()
            .with_channel(build_tonic_channel(&sig.endpoint, *no_verify)?)
            .with_metadata(tonic::metadata::MetadataMap::from_headers(headers))
            .build()?,
    };

    Ok(opentelemetry_sdk::metrics::SdkMeterProvider::builder()
        .with_reader(
            PeriodicReader::builder(exporter, opentelemetry_sdk::runtime::Tokio)
                .with_interval(std::time::Duration::from_secs(30))
                .build(),
        )
        .with_view(|i: &opentelemetry_sdk::metrics::Instrument| {
            if i.kind() == opentelemetry_sdk::metrics::InstrumentKind::Histogram {
                Some(
                    opentelemetry_sdk::metrics::Stream::builder()
                        .with_aggregation(
                            opentelemetry_sdk::metrics::Aggregation::Base2ExponentialHistogram {
                                max_size: 160,
                                max_scale: 20,
                                record_min_max: true,
                            },
                        )
                        .build()
                        .unwrap(),
                )
            } else {
                None
            }
        })
        .with_resource(resource.clone())
        .build())
}

fn build_traces_provider(
    sig: &SignalConfig,
    no_verify: &bool,
    resource: &opentelemetry_sdk::Resource,
    authorization: Option<&str>,
) -> Result<opentelemetry_sdk::trace::SdkTracerProvider, Box<dyn std::error::Error + Send + Sync>> {
    use opentelemetry_otlp::{SpanExporter, WithExportConfig, WithHttpConfig, WithTonicConfig};
    use opentelemetry_sdk::trace::span_processor_with_async_runtime::BatchSpanProcessor;

    let mut headers = http::HeaderMap::new();
    if let Some(auth) = authorization {
        headers.insert(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_str(auth)?,
        );
    }

    let exporter: SpanExporter = match sig.protocol.as_str() {
        "http/protobuf" => SpanExporter::builder()
            .with_http()
            .with_http_client(HyperOtelClient::new(*no_verify)?)
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
            .build()?,

        "http/json" => SpanExporter::builder()
            .with_http()
            .with_http_client(HyperOtelClient::new(*no_verify)?)
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
            .build()?,
        _ => SpanExporter::builder()
            .with_tonic()
            .with_channel(build_tonic_channel(&sig.endpoint, *no_verify)?)
            .with_metadata(tonic::metadata::MetadataMap::from_headers(headers))
            .build()?,
    };

    Ok(opentelemetry_sdk::trace::SdkTracerProvider::builder()
        .with_span_processor(
            BatchSpanProcessor::builder(exporter, opentelemetry_sdk::runtime::Tokio).build(),
        )
        .with_id_generator(RequestedIdGenerator)
        .with_resource(resource.clone())
        .with_sampler(opentelemetry_sdk::trace::Sampler::AlwaysOn)
        .build())
}
