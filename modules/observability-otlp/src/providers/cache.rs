use std::collections::HashMap;

use ferron_observability::baggage::{BaggageKeyPromotion, DistinctValueTracker};
use ferron_observability::{CompositeEventSink, Event, LogAttributeValue, LogEvent, LogLevel};

use crate::client::{build_tonic_channel, HyperOtelClient};
use crate::config::{OtlpBackendConfig, SignalConfig};

use super::context::build_resource;
use super::metrics::CachedInstrument;

/// Cached OTLP providers for a given config
pub struct OtlpProviderCache {
    pub metrics_provider: Option<opentelemetry_sdk::metrics::SdkMeterProvider>,
    pub metrics_instruments: HashMap<&'static str, CachedInstrument>,
    pub baggage_promotions: Vec<BaggageKeyPromotion>,
    pub baggage_tracker: DistinctValueTracker,
}

impl OtlpProviderCache {
    pub fn init(
        config: &OtlpBackendConfig,
        event_sink: Option<&CompositeEventSink>,
    ) -> OtlpProviderCache {
        let resource = build_resource(config.service_name.clone());

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
                }));
            }
            result.ok()
        });

        OtlpProviderCache {
            metrics_provider,
            metrics_instruments: HashMap::new(),
            baggage_promotions: config.baggage_promotions.clone(),
            baggage_tracker: DistinctValueTracker::new(),
        }
    }
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
                        .ok()?,
                )
            } else {
                None
            }
        })
        .with_resource(resource.clone())
        .build())
}
