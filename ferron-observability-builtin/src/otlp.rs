use std::{error::Error, sync::Arc, time::Duration};

use async_channel::Sender;
use ferron_common::{
  config::ServerConfiguration,
  get_entries_for_validation, get_entry, get_value,
  logging::LogMessage,
  observability::{ObservabilityBackend, ObservabilityBackendLoader},
  util::{ModuleCache, NoServerVerifier},
};
use hyper::header::HeaderValue;
use opentelemetry::logs::{LogRecord, Logger, LoggerProvider};
use opentelemetry_otlp::{WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::Resource;
use rustls::{client::WebPkiServerVerifier, ClientConfig};
use rustls_platform_verifier::BuilderVerifierExt;

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
      cache: ModuleCache::new(vec!["otlp_no_verification", "otlp_service_name", "otlp_logs"]),
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
            let (logging_tx, logging_rx) = async_channel::unbounded::<LogMessage>();
            logging_tx_option = Some(logging_tx);
            secondary_runtime.spawn(async move {
              let log_provider = match match logs_protocol.as_deref() {
                Some("http/protobuf") | Some("http/json") => opentelemetry_otlp::LogExporter::builder()
                  .with_http()
                  .with_http_client(opentelemetry_http::hyper::HyperClient::new(
                    hyper_rustls::HttpsConnectorBuilder::new()
                      .with_tls_config(hyper_tls_config.clone())
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
                  _ = cancel_token_clone.cancelled() => return,
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

          Ok(Arc::new(OtlpObservabilityBackend {
            cancel_token,
            logging_tx_option,
          }))
        })?,
    )
  }

  fn get_requirements(&self) -> Vec<&'static str> {
    vec!["otlp_logs"]
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

    Ok(())
  }
}

struct OtlpObservabilityBackend {
  cancel_token: tokio_util::sync::CancellationToken,
  logging_tx_option: Option<Sender<LogMessage>>,
}

impl ObservabilityBackend for OtlpObservabilityBackend {
  fn get_log_channel(&self) -> Option<Sender<LogMessage>> {
    self.logging_tx_option.clone()
  }
}

impl Drop for OtlpObservabilityBackend {
  fn drop(&mut self) {
    self.cancel_token.cancel();
  }
}
