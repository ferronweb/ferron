use std::{collections::HashMap, error::Error, sync::Arc, time::Duration};

use async_channel::Sender;
use ferron_common::{
  config::ServerConfiguration,
  get_entries_for_validation, get_entry, get_value,
  logging::LogMessage,
  observability::{
    Metric, MetricAttributeValue, MetricType, MetricValue, ObservabilityBackend, ObservabilityBackendLoader,
    TraceSignal,
  },
  util::{ModuleCache, NoServerVerifier},
};
use hashlink::LinkedHashMap;
use hyper::header::HeaderValue;
use opentelemetry::trace::{Tracer, TracerProvider};
use opentelemetry::KeyValue;
use opentelemetry::{
  logs::{LogRecord, Logger, LoggerProvider},
  Context,
};
use opentelemetry::{metrics::MeterProvider, trace::TraceContextExt};
use opentelemetry_otlp::{WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::Resource;
use rustls::{client::WebPkiServerVerifier, ClientConfig};
use rustls_platform_verifier::BuilderVerifierExt;

enum CachedInstrument {
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

/// OTLP observability backend loader
pub struct OtlpObservabilityBackendLoader {
  cache: ModuleCache<OtlpObservabilityBackend>,
}

impl Default for OtlpObservabilityBackendLoader {
  fn default() -> Self {
    Self::new()
  }
}

impl OtlpObservabilityBackendLoader {
  /// Creates a new observability backend loader
  pub fn new() -> Self {
    Self {
      cache: ModuleCache::new(vec![
        "otlp_no_verification",
        "otlp_service_name",
        "otlp_logs",
        "otlp_metrics",
        "otlp_traces",
      ]),
    }
  }
}

impl ObservabilityBackendLoader for OtlpObservabilityBackendLoader {
  fn load_observability_backend(
    &mut self,
    config: &ServerConfiguration,
    _global_config: Option<&ServerConfiguration>,
    secondary_runtime: &tokio::runtime::Runtime,
  ) -> Result<Arc<dyn ObservabilityBackend + Send + Sync>, Box<dyn Error + Send + Sync>> {
    Ok(
      self
        .cache
        .get_or_init::<_, Box<dyn std::error::Error + Send + Sync>>(config, move |config| {
          // Configuration properties
          let otlp_verify = !get_value!("otlp_no_verification", config)
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
          let service_name = get_value!("otlp_service_name", config)
            .and_then(|v| v.as_str())
            .unwrap_or("ferron")
            .to_string();

          // Common code
          let cancel_token = tokio_util::sync::CancellationToken::new();
          let crypto_provider = rustls::crypto::CryptoProvider::get_default()
            .cloned()
            .unwrap_or(Arc::new(rustls::crypto::aws_lc_rs::default_provider()));
          let opentelemetry_resource = Resource::builder().with_service_name(service_name).build();
          let hyper_tls_config = (if !otlp_verify {
            ClientConfig::builder_with_provider(crypto_provider.clone())
              .with_safe_default_protocol_versions()?
              .dangerous()
              .with_custom_certificate_verifier(Arc::new(NoServerVerifier::new()))
          } else if let Ok(client_config) = BuilderVerifierExt::with_platform_verifier(
            ClientConfig::builder_with_provider(crypto_provider.clone()).with_safe_default_protocol_versions()?,
          ) {
            client_config
          } else {
            ClientConfig::builder_with_provider(crypto_provider.clone())
              .with_safe_default_protocol_versions()?
              .with_webpki_verifier(
                WebPkiServerVerifier::builder(Arc::new(rustls::RootCertStore {
                  roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
                }))
                .build()?,
              )
          })
          .with_no_client_auth();

          // Logging
          let otlp_logs = get_entry!("otlp_logs", config);
          let logs_endpoint = otlp_logs
            .and_then(|e| e.values.first())
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
          let logs_authorization = otlp_logs
            .and_then(|e| e.props.get("authorization"))
            .and_then(|v| v.as_str())
            .and_then(|s| HeaderValue::from_str(s).ok());
          let logs_protocol = otlp_logs
            .and_then(|e| e.props.get("protocol"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
          let mut logging_tx_option = None;
          if let Some(logs_endpoint) = logs_endpoint {
            let cancel_token_clone = cancel_token.clone();
            let opentelemetry_resource = opentelemetry_resource.clone();
            let hyper_tls_config = hyper_tls_config.clone();
            let (logging_tx, logging_rx) = async_channel::unbounded::<LogMessage>();
            logging_tx_option = Some(logging_tx);
            secondary_runtime.spawn(async move {
              let log_provider = match match logs_protocol.as_deref() {
                Some("http/protobuf") | Some("http/json") => opentelemetry_otlp::LogExporter::builder()
                  .with_http()
                  .with_http_client(opentelemetry_http::hyper::HyperClient::new(
                    hyper_rustls::HttpsConnectorBuilder::new()
                      .with_tls_config(hyper_tls_config)
                      .https_or_http()
                      .enable_http1()
                      .enable_http2()
                      .build(),
                    Duration::from_secs(10),
                    logs_authorization,
                  ))
                  .with_protocol(match logs_protocol.as_deref() {
                    Some("http/json") => opentelemetry_otlp::Protocol::HttpJson,
                    _ => opentelemetry_otlp::Protocol::HttpBinary, // Also: Some("http/protobuf")
                  })
                  .with_endpoint(logs_endpoint)
                  .build(),
                _ => opentelemetry_otlp::LogExporter::builder() // Also: Some("grpc")
                  .with_tonic()
                  .with_endpoint(logs_endpoint)
                  .build(),
              } {
                Ok(exporter) => Some(
                  opentelemetry_sdk::logs::SdkLoggerProvider::builder()
                    .with_log_processor(
                      opentelemetry_sdk::logs::log_processor_with_async_runtime::BatchLogProcessor::builder(
                        exporter,
                        opentelemetry_sdk::runtime::Tokio,
                      )
                      .with_batch_config(opentelemetry_sdk::logs::BatchConfig::default())
                      .build(),
                    )
                    .with_resource(opentelemetry_resource)
                    .build(),
                ),
                Err(e) => {
                  eprintln!("Failed to initialize OTLP log exporter: {e}");
                  None
                }
              };

              if let Some(log_provider) = log_provider {
                let access_logger = log_provider.logger("access");
                let error_logger = log_provider.logger("error");
                while let Ok(message) = tokio::select! {
                  message = logging_rx.recv() => message,
                  _ = cancel_token_clone.cancelled() => {
                      log_provider.shutdown().unwrap_or_default();
                      return;
                  },
                } {
                  let (message_inner, is_error) = message.get_message();
                  if is_error {
                    let mut log_record = error_logger.create_log_record();
                    log_record.set_body(message_inner.into());
                    error_logger.emit(log_record);
                  } else {
                    let mut log_record = access_logger.create_log_record();
                    log_record.set_body(message_inner.into());
                    access_logger.emit(log_record);
                  }
                }

                log_provider.shutdown().unwrap_or_default();
              }
            });
          }

          // Metrics
          let otlp_metrics = get_entry!("otlp_metrics", config);
          let metrics_endpoint = otlp_metrics
            .and_then(|e| e.values.first())
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
          let metrics_authorization = otlp_metrics
            .and_then(|e| e.props.get("authorization"))
            .and_then(|v| v.as_str())
            .and_then(|s| HeaderValue::from_str(s).ok());
          let metrics_protocol = otlp_metrics
            .and_then(|e| e.props.get("protocol"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
          let mut metrics_tx_option = None;
          if let Some(metrics_endpoint) = metrics_endpoint {
            let cancel_token_clone = cancel_token.clone();
            let opentelemetry_resource = opentelemetry_resource.clone();
            let hyper_tls_config = hyper_tls_config.clone();
            let (metrics_tx, metrics_rx) = async_channel::unbounded::<Metric>();
            metrics_tx_option = Some(metrics_tx);
            secondary_runtime.spawn(async move {
              let metric_provider = match match metrics_protocol.as_deref() {
                Some("http/protobuf") | Some("http/json") => opentelemetry_otlp::MetricExporter::builder()
                  .with_http()
                  .with_http_client(opentelemetry_http::hyper::HyperClient::new(
                    hyper_rustls::HttpsConnectorBuilder::new()
                      .with_tls_config(hyper_tls_config)
                      .https_or_http()
                      .enable_http1()
                      .enable_http2()
                      .build(),
                    Duration::from_secs(10),
                    metrics_authorization,
                  ))
                  .with_protocol(match metrics_protocol.as_deref() {
                    Some("http/json") => opentelemetry_otlp::Protocol::HttpJson,
                    _ => opentelemetry_otlp::Protocol::HttpBinary, // Also: Some("http/protobuf")
                  })
                  .with_endpoint(metrics_endpoint)
                  .build(),
                _ => opentelemetry_otlp::MetricExporter::builder() // Also: Some("grpc")
                  .with_tonic()
                  .with_endpoint(metrics_endpoint)
                  .build(),
              } {
                Ok(exporter) => Some(
                  opentelemetry_sdk::metrics::SdkMeterProvider::builder()
                    .with_reader(
                      opentelemetry_sdk::metrics::periodic_reader_with_async_runtime::PeriodicReader::builder(
                        exporter,
                        opentelemetry_sdk::runtime::Tokio,
                      )
                      .with_interval(Duration::from_secs(30))
                      .build(),
                    )
                    .with_resource(opentelemetry_resource)
                    .build(),
                ),
                Err(e) => {
                  eprintln!("Failed to initialize OTLP metrics exporter: {e}");
                  None
                }
              };

              let mut instrument_cache: HashMap<&'static str, CachedInstrument> = HashMap::new();

              if let Some(metric_provider) = metric_provider {
                let meter = metric_provider.meter("ferron");
                while let Ok(metric) = tokio::select! {
                  message = metrics_rx.recv() => message,
                  _ = cancel_token_clone.cancelled() => {
                      metric_provider.shutdown().unwrap_or_default();
                      return;
                  },
                } {
                  let attributes = metric
                    .attributes
                    .into_iter()
                    .map(|(key, value)| {
                      KeyValue::new(
                        key,
                        match value {
                          MetricAttributeValue::F64(value) => opentelemetry::Value::from(value),
                          MetricAttributeValue::I64(value) => opentelemetry::Value::from(value),
                          MetricAttributeValue::String(value) => opentelemetry::Value::from(value),
                          MetricAttributeValue::Bool(value) => opentelemetry::Value::from(value),
                        },
                      )
                    })
                    .collect::<Vec<_>>();
                  match (metric.ty, metric.value) {
                    (MetricType::Counter, MetricValue::F64(value)) => {
                      if let CachedInstrument::F64Counter(metric) =
                        instrument_cache.entry(metric.name).or_insert_with(|| {
                          let mut metric_builder = meter.f64_counter(metric.name);
                          if let Some(unit) = metric.unit {
                            metric_builder = metric_builder.with_unit(unit);
                          }
                          if let Some(description) = metric.description {
                            metric_builder = metric_builder.with_description(description);
                          }
                          CachedInstrument::F64Counter(metric_builder.build())
                        })
                      {
                        metric.add(value, &attributes);
                      }
                    }
                    (MetricType::Counter, MetricValue::U64(value)) => {
                      if let CachedInstrument::U64Counter(metric) =
                        instrument_cache.entry(metric.name).or_insert_with(|| {
                          let mut metric_builder = meter.u64_counter(metric.name);
                          if let Some(unit) = metric.unit {
                            metric_builder = metric_builder.with_unit(unit);
                          }
                          if let Some(description) = metric.description {
                            metric_builder = metric_builder.with_description(description);
                          }
                          CachedInstrument::U64Counter(metric_builder.build())
                        })
                      {
                        metric.add(value, &attributes);
                      }
                    }
                    (MetricType::UpDownCounter, MetricValue::F64(value)) => {
                      if let CachedInstrument::F64UpDownCounter(metric) =
                        instrument_cache.entry(metric.name).or_insert_with(|| {
                          let mut metric_builder = meter.f64_up_down_counter(metric.name);
                          if let Some(unit) = metric.unit {
                            metric_builder = metric_builder.with_unit(unit);
                          }
                          if let Some(description) = metric.description {
                            metric_builder = metric_builder.with_description(description);
                          }
                          CachedInstrument::F64UpDownCounter(metric_builder.build())
                        })
                      {
                        metric.add(value, &attributes);
                      }
                    }
                    (MetricType::UpDownCounter, MetricValue::I64(value)) => {
                      if let CachedInstrument::I64UpDownCounter(metric) =
                        instrument_cache.entry(metric.name).or_insert_with(|| {
                          let mut metric_builder = meter.i64_up_down_counter(metric.name);
                          if let Some(unit) = metric.unit {
                            metric_builder = metric_builder.with_unit(unit);
                          }
                          if let Some(description) = metric.description {
                            metric_builder = metric_builder.with_description(description);
                          }
                          CachedInstrument::I64UpDownCounter(metric_builder.build())
                        })
                      {
                        metric.add(value, &attributes);
                      }
                    }
                    (MetricType::Gauge, MetricValue::F64(value)) => {
                      if let CachedInstrument::F64Gauge(metric) =
                        instrument_cache.entry(metric.name).or_insert_with(|| {
                          let mut metric_builder = meter.f64_gauge(metric.name);
                          if let Some(unit) = metric.unit {
                            metric_builder = metric_builder.with_unit(unit);
                          }
                          if let Some(description) = metric.description {
                            metric_builder = metric_builder.with_description(description);
                          }
                          CachedInstrument::F64Gauge(metric_builder.build())
                        })
                      {
                        metric.record(value, &attributes);
                      }
                    }
                    (MetricType::Gauge, MetricValue::I64(value)) => {
                      if let CachedInstrument::I64Gauge(metric) =
                        instrument_cache.entry(metric.name).or_insert_with(|| {
                          let mut metric_builder = meter.i64_gauge(metric.name);
                          if let Some(unit) = metric.unit {
                            metric_builder = metric_builder.with_unit(unit);
                          }
                          if let Some(description) = metric.description {
                            metric_builder = metric_builder.with_description(description);
                          }
                          CachedInstrument::I64Gauge(metric_builder.build())
                        })
                      {
                        metric.record(value, &attributes);
                      }
                    }
                    (MetricType::Gauge, MetricValue::U64(value)) => {
                      if let CachedInstrument::U64Gauge(metric) =
                        instrument_cache.entry(metric.name).or_insert_with(|| {
                          let mut metric_builder = meter.u64_gauge(metric.name);
                          if let Some(unit) = metric.unit {
                            metric_builder = metric_builder.with_unit(unit);
                          }
                          if let Some(description) = metric.description {
                            metric_builder = metric_builder.with_description(description);
                          }
                          CachedInstrument::U64Gauge(metric_builder.build())
                        })
                      {
                        metric.record(value, &attributes);
                      }
                    }
                    (MetricType::Histogram(buckets), MetricValue::F64(value)) => {
                      if let CachedInstrument::F64Histogram(metric) =
                        instrument_cache.entry(metric.name).or_insert_with(|| {
                          let mut metric_builder = meter.f64_histogram(metric.name);
                          if let Some(unit) = metric.unit {
                            metric_builder = metric_builder.with_unit(unit);
                          }
                          if let Some(buckets) = buckets {
                            metric_builder = metric_builder.with_boundaries(buckets);
                          }
                          if let Some(description) = metric.description {
                            metric_builder = metric_builder.with_description(description);
                          }
                          CachedInstrument::F64Histogram(metric_builder.build())
                        })
                      {
                        metric.record(value, &attributes);
                      }
                    }
                    (MetricType::Histogram(buckets), MetricValue::U64(value)) => {
                      if let CachedInstrument::U64Histogram(metric) =
                        instrument_cache.entry(metric.name).or_insert_with(|| {
                          let mut metric_builder = meter.u64_histogram(metric.name);
                          if let Some(unit) = metric.unit {
                            metric_builder = metric_builder.with_unit(unit);
                          }
                          if let Some(buckets) = buckets {
                            metric_builder = metric_builder.with_boundaries(buckets);
                          }
                          if let Some(description) = metric.description {
                            metric_builder = metric_builder.with_description(description);
                          }
                          CachedInstrument::U64Histogram(metric_builder.build())
                        })
                      {
                        metric.record(value, &attributes);
                      }
                    }
                    _ => {} // Ignore unsupported metric types
                  }
                }

                metric_provider.shutdown().unwrap_or_default();
              }
            });
          }

          // Traces
          let otlp_traces = get_entry!("otlp_traces", config);
          let traces_endpoint = otlp_traces
            .and_then(|e| e.values.first())
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
          let traces_authorization = otlp_traces
            .and_then(|e| e.props.get("authorization"))
            .and_then(|v| v.as_str())
            .and_then(|s| HeaderValue::from_str(s).ok());
          let traces_protocol = otlp_traces
            .and_then(|e| e.props.get("protocol"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
          let mut traces_channel_option = None;
          if let Some(traces_endpoint) = traces_endpoint {
            let cancel_token_clone = cancel_token.clone();
            let opentelemetry_resource = opentelemetry_resource.clone();
            let hyper_tls_config = hyper_tls_config.clone();
            let (traces_channel_tx, traces_channel_rx) = async_channel::unbounded::<Sender<TraceSignal>>();
            let (traces_request_tx, traces_request_rx) = async_channel::unbounded::<()>();
            traces_channel_option = Some((traces_request_tx, traces_channel_rx));
            secondary_runtime.spawn(async move {
              let traces_provider = match match traces_protocol.as_deref() {
                Some("http/protobuf") | Some("http/json") => opentelemetry_otlp::SpanExporter::builder()
                  .with_http()
                  .with_http_client(opentelemetry_http::hyper::HyperClient::new(
                    hyper_rustls::HttpsConnectorBuilder::new()
                      .with_tls_config(hyper_tls_config)
                      .https_or_http()
                      .enable_http1()
                      .enable_http2()
                      .build(),
                    Duration::from_secs(10),
                    traces_authorization,
                  ))
                  .with_protocol(match traces_protocol.as_deref() {
                    Some("http/json") => opentelemetry_otlp::Protocol::HttpJson,
                    _ => opentelemetry_otlp::Protocol::HttpBinary, // Also: Some("http/protobuf")
                  })
                  .with_endpoint(traces_endpoint)
                  .build(),
                _ => opentelemetry_otlp::SpanExporter::builder() // Also: Some("grpc")
                  .with_tonic()
                  .with_endpoint(traces_endpoint)
                  .build(),
              } {
                Ok(exporter) => Some(
                  opentelemetry_sdk::trace::SdkTracerProvider::builder()
                    .with_span_processor(
                      opentelemetry_sdk::trace::span_processor_with_async_runtime::BatchSpanProcessor::builder(
                        exporter,
                        opentelemetry_sdk::runtime::Tokio,
                      )
                      .with_batch_config(opentelemetry_sdk::trace::BatchConfig::default())
                      .build(),
                    )
                    .with_resource(opentelemetry_resource)
                    .build(),
                ),
                Err(e) => {
                  eprintln!("Failed to initialize OTLP trace exporter: {e}");
                  None
                }
              };

              if let Some(traces_provider) = traces_provider {
                while tokio::select! {
                  message = traces_request_rx.recv() => message.is_ok(),
                  _ = cancel_token_clone.cancelled() => {
                      traces_provider.shutdown().unwrap_or_default();
                      return;
                  },
                } {
                  let tracer = traces_provider.tracer("ferron");
                  let (traces_tx, traces_rx) = async_channel::unbounded::<TraceSignal>();
                  let cancel_token_clone = cancel_token_clone.clone();
                  tokio::spawn(async move {
                    let mut spans: LinkedHashMap<String, Context> = LinkedHashMap::new();
                    while let Ok(signal) = tokio::select! {
                      message = traces_rx.recv() => message,
                      _ = cancel_token_clone.cancelled() => {
                          return;
                      },
                    } {
                      match signal {
                        TraceSignal::StartSpan(span) => {
                          let span_context = spans.back().map(|(_, context)| context.clone()).unwrap_or_default();
                          let new_span = tracer.start_with_context(span.clone(), &span_context);
                          let new_span_context = span_context.with_span(new_span);
                          spans.insert(span, new_span_context);
                        }
                        TraceSignal::EndSpan(span) => {
                          if let Some(span_context) = spans.get(&span) {
                            span_context.span().end();
                          }
                        }
                      }
                    }
                  });
                  traces_channel_tx.send(traces_tx).await.unwrap_or_default();
                }

                traces_provider.shutdown().unwrap_or_default();
              }
            });
          }

          Ok(Arc::new(OtlpObservabilityBackend {
            cancel_token,
            logging_tx_option,
            metrics_tx_option,
            traces_channel_option,
          }))
        })?,
    )
  }

  fn get_requirements(&self) -> Vec<&'static str> {
    vec!["otlp_logs", "otlp_metrics", "otlp_traces"]
  }

  fn validate_configuration(
    &self,
    config: &ferron_common::config::ServerConfiguration,
    used_properties: &mut std::collections::HashSet<String>,
  ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if let Some(entries) = get_entries_for_validation!("otlp_no_verification", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `otlp_no_verification` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_bool() {
          Err(anyhow::anyhow!(
            "Invalid TLS certificate verification for OTLP disabling option"
          ))?
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("otlp_service_name", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `otlp_service_name` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_string() {
          Err(anyhow::anyhow!("Invalid OTLP service name"))?
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("otlp_logs", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `otlp_logs` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_string() && !entry.values[0].is_null() {
          Err(anyhow::anyhow!("Invalid OTLP logs endpoint"))?
        } else if !entry.props.get("authorization").is_none_or(|v| v.is_string()) {
          Err(anyhow::anyhow!(
            "Invalid Authorization header value for OTLP logs endpoint"
          ))?
        } else if !entry.props.get("protocol").is_none_or(|v| v.is_string()) {
          Err(anyhow::anyhow!("Invalid protocol for OTLP logs endpoint"))?
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("otlp_metrics", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `otlp_metrics` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_string() && !entry.values[0].is_null() {
          Err(anyhow::anyhow!("Invalid OTLP metrics endpoint"))?
        } else if !entry.props.get("authorization").is_none_or(|v| v.is_string()) {
          Err(anyhow::anyhow!(
            "Invalid Authorization header value for OTLP metrics endpoint"
          ))?
        } else if !entry.props.get("protocol").is_none_or(|v| v.is_string()) {
          Err(anyhow::anyhow!("Invalid protocol for OTLP metrics endpoint"))?
        }
      }
    }

    if let Some(entries) = get_entries_for_validation!("otlp_traces", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `otlp_traces` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_string() && !entry.values[0].is_null() {
          Err(anyhow::anyhow!("Invalid OTLP traces endpoint"))?
        } else if !entry.props.get("authorization").is_none_or(|v| v.is_string()) {
          Err(anyhow::anyhow!(
            "Invalid Authorization header value for OTLP traces endpoint"
          ))?
        } else if !entry.props.get("protocol").is_none_or(|v| v.is_string()) {
          Err(anyhow::anyhow!("Invalid protocol for OTLP traces endpoint"))?
        }
      }
    }

    Ok(())
  }
}

struct OtlpObservabilityBackend {
  cancel_token: tokio_util::sync::CancellationToken,
  logging_tx_option: Option<Sender<LogMessage>>,
  metrics_tx_option: Option<Sender<Metric>>,
  traces_channel_option: Option<(Sender<()>, async_channel::Receiver<Sender<TraceSignal>>)>,
}

impl ObservabilityBackend for OtlpObservabilityBackend {
  fn get_log_channel(&self) -> Option<Sender<LogMessage>> {
    self.logging_tx_option.clone()
  }

  fn get_metric_channel(&self) -> Option<Sender<Metric>> {
    self.metrics_tx_option.clone()
  }

  fn get_trace_channel(&self) -> Option<(Sender<()>, async_channel::Receiver<Sender<TraceSignal>>)> {
    self.traces_channel_option.clone()
  }
}

impl Drop for OtlpObservabilityBackend {
  fn drop(&mut self) {
    self.cancel_token.cancel();
  }
}
