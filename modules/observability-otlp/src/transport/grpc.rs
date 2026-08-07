use std::error::Error;
use std::time::Duration;

use prost::Message;
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;

use super::client::{
    retry_with_backoff, ExportResult, RetryConfig, MAX_REQUEST_SIZE, MAX_RESPONSE_SIZE,
};
use super::http_client::build_tonic_channel;
use crate::proto::opentelemetry::proto::collector::{
    logs::v1::logs_service_client::LogsServiceClient, logs::v1::ExportLogsServiceRequest,
    metrics::v1::metrics_service_client::MetricsServiceClient,
    metrics::v1::ExportMetricsServiceRequest, trace::v1::trace_service_client::TraceServiceClient,
    trace::v1::ExportTraceServiceRequest,
};

/// Which OTLP signal a transport instance carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalKind {
    Logs,
    Metrics,
    Traces,
}

/// gRPC transport for one OTLP signal: the generated tonic client over a
/// channel built with the shared TLS configuration, with optional
/// `authorization` metadata on every request.
pub struct GrpcSignal {
    client: tokio::sync::Mutex<GrpcSignalClient>,
}

/// The three generated service clients, specialized to the signal.
enum GrpcSignalClient {
    Logs(LogsServiceClient<tonic::codegen::InterceptedService<Channel, AuthInterceptor>>),
    Metrics(MetricsServiceClient<tonic::codegen::InterceptedService<Channel, AuthInterceptor>>),
    Traces(TraceServiceClient<tonic::codegen::InterceptedService<Channel, AuthInterceptor>>),
}

/// Adds the configured `authorization` metadata value to every gRPC request.
#[derive(Debug, Clone)]
pub struct AuthInterceptor(pub Option<MetadataValue<tonic::metadata::Ascii>>);

impl tonic::service::Interceptor for AuthInterceptor {
    fn call(
        &mut self,
        mut request: tonic::Request<()>,
    ) -> Result<tonic::Request<()>, tonic::Status> {
        if let Some(authorization) = &self.0 {
            request
                .metadata_mut()
                .insert("authorization", authorization.clone());
        }
        Ok(request)
    }
}

impl GrpcSignal {
    /// Connect to the OTLP receiver over gRPC for one signal.
    pub fn new(
        kind: SignalKind,
        endpoint: &str,
        no_verify: bool,
        authorization: Option<&str>,
    ) -> Result<Self, Box<dyn Error + Send + Sync>> {
        let channel = build_tonic_channel(endpoint, no_verify)?;
        let authorization = authorization
            .map(|value| value.parse::<MetadataValue<tonic::metadata::Ascii>>())
            .transpose()?;
        let interceptor = AuthInterceptor(authorization);
        let client = match kind {
            SignalKind::Logs => GrpcSignalClient::Logs(
                LogsServiceClient::with_interceptor(channel.clone(), interceptor)
                    .max_decoding_message_size(MAX_RESPONSE_SIZE)
                    .max_encoding_message_size(MAX_REQUEST_SIZE),
            ),
            SignalKind::Metrics => GrpcSignalClient::Metrics(
                MetricsServiceClient::with_interceptor(channel.clone(), interceptor)
                    .max_decoding_message_size(MAX_RESPONSE_SIZE)
                    .max_encoding_message_size(MAX_REQUEST_SIZE),
            ),
            SignalKind::Traces => GrpcSignalClient::Traces(
                TraceServiceClient::with_interceptor(channel.clone(), interceptor)
                    .max_decoding_message_size(MAX_RESPONSE_SIZE)
                    .max_encoding_message_size(MAX_REQUEST_SIZE),
            ),
        };
        Ok(Self {
            client: tokio::sync::Mutex::new(client),
        })
    }

    /// Export a batch of log records over gRPC, with retry/backoff.
    pub async fn export_logs(
        &self,
        request: &ExportLogsServiceRequest,
        retry: &RetryConfig,
    ) -> ExportResult {
        retry_with_backoff(retry, || async {
            let mut guard = self.client.lock().await;
            let GrpcSignalClient::Logs(client) = &mut *guard else {
                return ExportResult::Failure {
                    retryable: false,
                    retry_after: None,
                    message: "logs signal is not configured for the gRPC transport".to_string(),
                };
            };
            match client.export(request.clone()).await {
                Ok(response) => match response.into_inner().partial_success {
                    Some(partial) if partial.rejected_log_records > 0 => {
                        ExportResult::PartialSuccess {
                            rejected: partial.rejected_log_records as u64,
                            message: partial.error_message,
                        }
                    }
                    _ => ExportResult::Success,
                },
                Err(status) => failure_from_status(status),
            }
        })
        .await
    }

    /// Export a batch of metric data points over gRPC, with retry/backoff.
    pub async fn export_metrics(
        &self,
        request: &ExportMetricsServiceRequest,
        retry: &RetryConfig,
    ) -> ExportResult {
        retry_with_backoff(retry, || async {
            let mut guard = self.client.lock().await;
            let GrpcSignalClient::Metrics(client) = &mut *guard else {
                return ExportResult::Failure {
                    retryable: false,
                    retry_after: None,
                    message: "metrics signal is not configured for the gRPC transport".to_string(),
                };
            };
            match client.export(request.clone()).await {
                Ok(response) => match response.into_inner().partial_success {
                    Some(partial) if partial.rejected_data_points > 0 => {
                        ExportResult::PartialSuccess {
                            rejected: partial.rejected_data_points as u64,
                            message: partial.error_message,
                        }
                    }
                    _ => ExportResult::Success,
                },
                Err(status) => failure_from_status(status),
            }
        })
        .await
    }

    /// Export a batch of spans over gRPC, with retry/backoff.
    pub async fn export_traces(
        &self,
        request: &ExportTraceServiceRequest,
        retry: &RetryConfig,
    ) -> ExportResult {
        retry_with_backoff(retry, || async {
            let mut guard = self.client.lock().await;
            let GrpcSignalClient::Traces(client) = &mut *guard else {
                return ExportResult::Failure {
                    retryable: false,
                    retry_after: None,
                    message: "traces signal is not configured for the gRPC transport".to_string(),
                };
            };
            match client.export(request.clone()).await {
                Ok(response) => match response.into_inner().partial_success {
                    Some(partial) if partial.rejected_spans > 0 => ExportResult::PartialSuccess {
                        rejected: partial.rejected_spans as u64,
                        message: partial.error_message,
                    },
                    _ => ExportResult::Success,
                },
                Err(status) => failure_from_status(status),
            }
        })
        .await
    }
}

/// Classify a `tonic::Status` into an [`ExportResult`], mapping the gRPC code
/// to retryable per the OTLP specification. `RESOURCE_EXHAUSTED` is retryable
/// only when the status carries a `google.rpc.RetryInfo` detail; its
/// `retry_delay` is surfaced as the retry hint.
fn failure_from_status(status: tonic::Status) -> ExportResult {
    let (retryable, retry_after) = match status.code() {
        tonic::Code::Cancelled
        | tonic::Code::DeadlineExceeded
        | tonic::Code::Aborted
        | tonic::Code::OutOfRange
        | tonic::Code::Unavailable
        | tonic::Code::DataLoss => (true, None),
        tonic::Code::ResourceExhausted => match decode_retry_info(status.details()) {
            Some(retry_delay) => (true, retry_delay),
            None => (false, None),
        },
        _ => (false, None),
    };
    ExportResult::Failure {
        retryable,
        retry_after,
        message: status.to_string(),
    }
}

/// Decode `google.rpc.RetryInfo` from the `grpc-status-details-bin` value
/// (a serialized `google.rpc.Status`). Returns `Some(retry_delay)` when a
/// `RetryInfo` detail is present (the delay is `None` if the server did not
/// specify one), or `None` when no `RetryInfo` is present.
fn decode_retry_info(details: &[u8]) -> Option<Option<Duration>> {
    let status = rpc::Status::decode(details).ok()?;
    for detail in status.details {
        if detail.type_url.ends_with("google.rpc.RetryInfo") {
            let info = rpc::RetryInfo::decode(detail.value.as_slice()).ok()?;
            let delay = info
                .retry_delay
                .filter(|d| d.seconds > 0 || d.nanos > 0)
                .map(|d| Duration::new(d.seconds.max(0) as u64, d.nanos.max(0) as u32));
            return Some(delay);
        }
    }
    None
}

/// Minimal `google.rpc` / `google.protobuf` types needed to decode
/// `google.rpc.RetryInfo` from `grpc-status-details-bin`. The OTLP proto
/// subset does not include these, so they are hand-written with prost derives.
mod rpc {
    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct Status {
        #[prost(int32, tag = "1")]
        pub code: i32,
        #[prost(string, tag = "2")]
        pub message: String,
        #[prost(message, repeated, tag = "3")]
        pub details: Vec<Any>,
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct Any {
        #[prost(string, tag = "1")]
        pub type_url: String,
        #[prost(bytes = "vec", tag = "2")]
        pub value: Vec<u8>,
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct Duration {
        #[prost(int64, tag = "1")]
        pub seconds: i64,
        #[prost(int32, tag = "2")]
        pub nanos: i32,
    }

    #[derive(Clone, PartialEq, ::prost::Message)]
    pub struct RetryInfo {
        #[prost(message, optional, tag = "1")]
        pub retry_delay: Option<Duration>,
    }
}
#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use tokio::sync::Mutex;
    use tokio_stream::wrappers::TcpListenerStream;
    use tonic::transport::Server;

    use crate::proto::opentelemetry::proto::collector::trace::v1::{
        trace_service_server::{TraceService, TraceServiceServer},
        ExportTracePartialSuccess, ExportTraceServiceRequest, ExportTraceServiceResponse,
    };
    use crate::proto::opentelemetry::proto::resource::v1::Resource;
    use crate::proto::opentelemetry::proto::trace::v1::{ResourceSpans, ScopeSpans, Span};

    use super::*;

    /// Behavior of the fake gRPC receiver for the next call(s).
    #[derive(Default)]
    enum FakeMode {
        #[default]
        Ok,
        /// Fail with `UNAVAILABLE` this many times, then succeed.
        Unavailable { remaining: u32 },
        /// Fail with `RESOURCE_EXHAUSTED` + `RetryInfo` (delay `retry_delay`)
        /// this many times, then succeed.
        ResourceExhausted {
            remaining: u32,
            retry_delay: Option<Duration>,
        },
        /// Fail with plain `RESOURCE_EXHAUSTED` (no `RetryInfo`) every call.
        ResourceExhaustedNoDetails,
        /// Fail with `INVALID_ARGUMENT` every call.
        InvalidArgument,
        /// Answer with a partial-success response.
        PartialSuccess,
    }

    /// State shared between the fake receiver (running in a server task) and
    /// the test.
    #[derive(Default)]
    struct FakeState {
        requests: Mutex<Vec<ExportTraceServiceRequest>>,
        authorizations: Mutex<Vec<Option<String>>>,
        mode: Mutex<FakeMode>,
        call_count: AtomicU32,
    }

    struct FakeTraceService {
        state: Arc<FakeState>,
    }

    #[tonic::async_trait]
    impl TraceService for FakeTraceService {
        async fn export(
            &self,
            request: tonic::Request<ExportTraceServiceRequest>,
        ) -> Result<tonic::Response<ExportTraceServiceResponse>, tonic::Status> {
            let state = &self.state;
            state.call_count.fetch_add(1, Ordering::SeqCst);
            state.requests.lock().await.push(request.get_ref().clone());
            state.authorizations.lock().await.push(
                request
                    .metadata()
                    .get("authorization")
                    .map(|value| value.to_str().unwrap().to_string()),
            );

            let mut mode = state.mode.lock().await;
            match &mut *mode {
                FakeMode::Ok => Ok(tonic::Response::new(ExportTraceServiceResponse::default())),
                FakeMode::Unavailable { remaining } => {
                    if *remaining > 0 {
                        *remaining -= 1;
                        Err(tonic::Status::unavailable("receiver is down"))
                    } else {
                        Ok(tonic::Response::new(ExportTraceServiceResponse::default()))
                    }
                }
                FakeMode::ResourceExhausted {
                    remaining,
                    retry_delay,
                } => {
                    if *remaining > 0 {
                        *remaining -= 1;
                        Err(tonic::Status::with_details(
                            tonic::Code::ResourceExhausted,
                            "slow down",
                            status_with_retry_info(*retry_delay).into(),
                        ))
                    } else {
                        Ok(tonic::Response::new(ExportTraceServiceResponse::default()))
                    }
                }
                FakeMode::ResourceExhaustedNoDetails => {
                    Err(tonic::Status::resource_exhausted("slow down"))
                }
                FakeMode::InvalidArgument => Err(tonic::Status::invalid_argument("bad request")),
                FakeMode::PartialSuccess => Ok(tonic::Response::new(ExportTraceServiceResponse {
                    partial_success: Some(ExportTracePartialSuccess {
                        rejected_spans: 2,
                        error_message: "partial".to_string(),
                    }),
                })),
            }
        }
    }

    /// Serialize a `google.rpc.Status` containing a `google.rpc.RetryInfo`
    /// detail with the given delay (the wire format of
    /// `grpc-status-details-bin`).
    fn status_with_retry_info(retry_delay: Option<Duration>) -> Vec<u8> {
        let retry_delay = retry_delay.map(|delay| rpc::Duration {
            seconds: delay.as_secs() as i64,
            nanos: delay.subsec_nanos() as i32,
        });
        let detail = rpc::Any {
            type_url: "type.googleapis.com/google.rpc.RetryInfo".to_string(),
            value: rpc::RetryInfo { retry_delay }.encode_to_vec(),
        };
        rpc::Status {
            code: 8,
            message: "slow down".to_string(),
            details: vec![detail],
        }
        .encode_to_vec()
    }

    async fn spawn_grpc_server(service: FakeTraceService) -> SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            Server::builder()
                .add_service(TraceServiceServer::new(service))
                .serve_with_incoming(TcpListenerStream::new(listener))
                .await
                .unwrap();
        });
        addr
    }

    fn sample_trace_request() -> ExportTraceServiceRequest {
        ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                resource: Some(Resource {
                    attributes: vec![crate::proto::opentelemetry::proto::common::v1::KeyValue {
                        key: "service.name".to_string(),
                        value: Some(crate::proto::opentelemetry::proto::common::v1::AnyValue {
                            value: Some(
                                crate::proto::opentelemetry::proto::common::v1::any_value::Value::StringValue("test".to_string()),
                            ),
                        }),
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
                scope_spans: vec![ScopeSpans {
                    spans: vec![Span {
                        name: "http GET /".to_string(),
                        trace_id: vec![0x5B, 0x8E],
                        span_id: vec![0xEE, 0xE1],
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        }
    }

    fn test_retry() -> RetryConfig {
        RetryConfig {
            max_attempts: 3,
            initial_backoff: Duration::from_millis(10),
            max_backoff: Duration::from_millis(200),
        }
    }

    async fn new_test_signal(state: &Arc<FakeState>, mode: FakeMode) -> (GrpcSignal, SocketAddr) {
        *state.mode.lock().await = mode;
        let service = FakeTraceService {
            state: state.clone(),
        };
        let addr = spawn_grpc_server(service).await;
        let signal = GrpcSignal::new(
            SignalKind::Traces,
            &format!("http://{addr}"),
            false,
            Some("Bearer token"),
        )
        .unwrap();
        (signal, addr)
    }

    #[tokio::test]
    async fn grpc_traces_roundtrip_with_authorization_metadata() {
        let state = Arc::new(FakeState::default());
        let (signal, _addr) = new_test_signal(&state, FakeMode::Ok).await;

        let request = sample_trace_request();
        let result = signal.export_traces(&request, &test_retry()).await;

        assert_eq!(result, ExportResult::Success);
        assert_eq!(state.call_count.load(Ordering::SeqCst), 1);
        assert_eq!(*state.requests.lock().await, vec![request]);
        assert_eq!(
            *state.authorizations.lock().await,
            vec![Some("Bearer token".to_string())]
        );
    }

    #[tokio::test]
    async fn grpc_retries_unavailable_then_succeeds() {
        let state = Arc::new(FakeState::default());
        let (signal, _addr) = new_test_signal(&state, FakeMode::Unavailable { remaining: 2 }).await;

        let result = signal
            .export_traces(&sample_trace_request(), &test_retry())
            .await;

        assert_eq!(result, ExportResult::Success);
        assert_eq!(state.call_count.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn grpc_resource_exhausted_with_retry_info_respects_delay() {
        let state = Arc::new(FakeState::default());
        let (signal, _addr) = new_test_signal(
            &state,
            FakeMode::ResourceExhausted {
                remaining: 1,
                retry_delay: Some(Duration::from_millis(100)),
            },
        )
        .await;

        let started = Instant::now();
        let result = signal
            .export_traces(&sample_trace_request(), &test_retry())
            .await;

        assert_eq!(result, ExportResult::Success);
        assert_eq!(state.call_count.load(Ordering::SeqCst), 2);
        assert!(
            started.elapsed() >= Duration::from_millis(90),
            "retry happened before RetryInfo delay elapsed"
        );
    }

    #[tokio::test]
    async fn grpc_resource_exhausted_without_retry_info_is_not_retryable() {
        let state = Arc::new(FakeState::default());
        let (signal, _addr) = new_test_signal(&state, FakeMode::ResourceExhaustedNoDetails).await;

        let result = signal
            .export_traces(&sample_trace_request(), &test_retry())
            .await;

        assert!(
            matches!(
                &result,
                ExportResult::Failure {
                    retryable: false,
                    ..
                }
            ),
            "unexpected result: {result:?}"
        );
        assert_eq!(state.call_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn grpc_invalid_argument_is_not_retried() {
        let state = Arc::new(FakeState::default());
        let (signal, _addr) = new_test_signal(&state, FakeMode::InvalidArgument).await;

        let result = signal
            .export_traces(&sample_trace_request(), &test_retry())
            .await;

        assert!(
            matches!(
                &result,
                ExportResult::Failure {
                    retryable: false,
                    ..
                }
            ),
            "unexpected result: {result:?}"
        );
        assert_eq!(state.call_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn grpc_partial_success_is_reported_not_retried() {
        let state = Arc::new(FakeState::default());
        let (signal, _addr) = new_test_signal(&state, FakeMode::PartialSuccess).await;

        let result = signal
            .export_traces(&sample_trace_request(), &test_retry())
            .await;

        assert_eq!(
            result,
            ExportResult::PartialSuccess {
                rejected: 2,
                message: "partial".to_string()
            }
        );
        assert_eq!(state.call_count.load(Ordering::SeqCst), 1);
    }
}
