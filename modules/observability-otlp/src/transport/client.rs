use std::error::Error;
use std::time::Duration;

use crate::config::OtlpBackendConfig;
use crate::config::SignalConfig;
use crate::proto::opentelemetry::proto::collector::{
    logs::v1::ExportLogsServiceRequest, metrics::v1::ExportMetricsServiceRequest,
    trace::v1::ExportTraceServiceRequest,
};

use super::grpc::GrpcSignal;
use super::grpc::SignalKind;
use super::http::HttpSignal;

/// Default cap on the exponential backoff delay between retries.
pub const DEFAULT_MAX_BACKOFF: Duration = Duration::from_secs(5);
/// Upper bound for a server-provided retry hint (`Retry-After` / `RetryInfo`).
pub const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);
/// Max size of a request body sent to the OTLP receiver (spec recommendation).
pub const MAX_REQUEST_SIZE: usize = 64 * 1024 * 1024;
/// Max size of a response body accepted from the OTLP receiver (spec
/// recommendation). Responses larger than this are discarded.
pub const MAX_RESPONSE_SIZE: usize = 4 * 1024 * 1024;

/// Outcome of an OTLP export after all retries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportResult {
    /// The receiver accepted the whole request.
    Success,
    /// The receiver accepted the request partially and rejected some items.
    /// Per the OTLP specification, partial success is never retried.
    PartialSuccess { rejected: u64, message: String },
    /// The request failed. `retryable` is `true` when the failure may succeed
    /// on a later attempt (the export loop already exhausted its retries).
    /// `retry_after` carries a server-provided retry hint, if any.
    Failure {
        retryable: bool,
        retry_after: Option<Duration>,
        message: String,
    },
}

/// Retry/backoff parameters for OTLP exports (see the OTLP specification,
/// "Retry" section).
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Total number of attempts, including the first one.
    pub max_attempts: u32,
    /// Backoff delay before the first retry.
    pub initial_backoff: Duration,
    /// Upper bound for the exponential backoff delay.
    pub max_backoff: Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_backoff: Duration::from_secs(1),
            max_backoff: DEFAULT_MAX_BACKOFF,
        }
    }
}

/// Run a single export attempt up to `max_attempts` times, waiting between
/// attempts according to the backoff policy. `attempt` returns the outcome of
/// one try; a server-provided retry hint (`Retry-After` for HTTP,
/// `RetryInfo.retry_delay` for gRPC) overrides the exponential backoff for
/// that attempt.
pub(crate) async fn retry_with_backoff<F, Fut>(config: &RetryConfig, mut attempt: F) -> ExportResult
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = ExportResult>,
{
    let mut backoff = config.initial_backoff;
    for attempt_number in 1..=config.max_attempts {
        let result = attempt().await;
        match &result {
            ExportResult::Success | ExportResult::PartialSuccess { .. } => return result,
            ExportResult::Failure {
                retryable: false, ..
            } => return result,
            ExportResult::Failure {
                retryable: true,
                retry_after,
                ..
            } => {
                if attempt_number == config.max_attempts {
                    return result;
                }
                let delay = match retry_after {
                    Some(delay) => (*delay).min(MAX_RETRY_AFTER),
                    None => jittered(backoff).min(config.max_backoff),
                };
                tokio::time::sleep(delay).await;
                backoff = (backoff * 2).min(config.max_backoff);
            }
        }
    }
    unreachable!("the retry loop returns from every iteration")
}

/// Add a pseudo-random factor in the [0.5, 1.5) range to the base delay, to
/// avoid thundering-herd retries from many clients at once.
fn jittered(base: Duration) -> Duration {
    let nanos = base.as_nanos() as u64;
    if nanos == 0 {
        return base;
    }
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(nanos);
    let mut x = seed ^ nanos.rotate_left(17);
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    let factor = 0.5 + (x % 1000) as f64 / 1000.0;
    Duration::from_nanos((nanos as f64 * factor) as u64)
}

/// One transport instance for all three configured OTLP signals. Each signal
/// selects its own protocol (`grpc`, `http/protobuf`, or `http/json`), so the
/// three signals may use different transports.
pub struct OtlpTransport {
    logs: Option<SignalTransport>,
    metrics: Option<SignalTransport>,
    traces: Option<SignalTransport>,
    retry: RetryConfig,
}

/// Transport used by a single signal.
enum SignalTransport {
    Grpc(GrpcSignal),
    Http(HttpSignal),
}

impl OtlpTransport {
    /// Build transports for every configured signal from the OTLP backend
    /// configuration.
    pub fn from_config(config: &OtlpBackendConfig) -> Result<Self, Box<dyn Error + Send + Sync>> {
        let retry = RetryConfig::default();
        Ok(Self {
            logs: build_signal(
                SignalKind::Logs,
                config.logs.as_ref(),
                config.no_verify,
                config.authorization.as_deref(),
            )?,
            metrics: build_signal(
                SignalKind::Metrics,
                config.metrics.as_ref(),
                config.no_verify,
                config.authorization.as_deref(),
            )?,
            traces: build_signal(
                SignalKind::Traces,
                config.traces.as_ref(),
                config.no_verify,
                config.authorization.as_deref(),
            )?,
            retry,
        })
    }

    /// Export a batch of log records.
    pub async fn export_logs(&self, request: &ExportLogsServiceRequest) -> ExportResult {
        match &self.logs {
            Some(SignalTransport::Grpc(t)) => t.export_logs(request, &self.retry).await,
            Some(SignalTransport::Http(t)) => t.export_logs(request, &self.retry).await,
            None => self.signal_not_configured("logs"),
        }
    }

    /// Export a batch of metric data points.
    pub async fn export_metrics(&self, request: &ExportMetricsServiceRequest) -> ExportResult {
        match &self.metrics {
            Some(SignalTransport::Grpc(t)) => t.export_metrics(request, &self.retry).await,
            Some(SignalTransport::Http(t)) => t.export_metrics(request, &self.retry).await,
            None => self.signal_not_configured("metrics"),
        }
    }

    /// Export a batch of spans.
    pub async fn export_traces(&self, request: &ExportTraceServiceRequest) -> ExportResult {
        match &self.traces {
            Some(SignalTransport::Grpc(t)) => t.export_traces(request, &self.retry).await,
            Some(SignalTransport::Http(t)) => t.export_traces(request, &self.retry).await,
            None => self.signal_not_configured("traces"),
        }
    }

    fn signal_not_configured(&self, signal: &str) -> ExportResult {
        ExportResult::Failure {
            retryable: false,
            retry_after: None,
            message: format!("OTLP {signal} signal is not configured"),
        }
    }
}

fn build_signal(
    kind: SignalKind,
    sig: Option<&SignalConfig>,
    no_verify: bool,
    fallback_authorization: Option<&str>,
) -> Result<Option<SignalTransport>, Box<dyn Error + Send + Sync>> {
    let Some(sig) = sig else {
        return Ok(None);
    };
    let authorization = sig.authorization.as_deref().or(fallback_authorization);
    let transport = match sig.protocol.as_str() {
        "grpc" => SignalTransport::Grpc(GrpcSignal::new(
            kind,
            &sig.endpoint,
            no_verify,
            authorization,
        )?),
        "http/protobuf" => SignalTransport::Http(HttpSignal::new(
            kind,
            sig.endpoint.clone(),
            false,
            no_verify,
            authorization,
        )?),
        "http/json" => SignalTransport::Http(HttpSignal::new(
            kind,
            sig.endpoint.clone(),
            true,
            no_verify,
            authorization,
        )?),
        other => return Err(format!("unsupported OTLP protocol: {other}").into()),
    };
    Ok(Some(transport))
}
