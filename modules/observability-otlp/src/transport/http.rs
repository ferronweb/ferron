use std::error::Error;
use std::time::Duration;

use bytes::Bytes;
use http::{HeaderValue, StatusCode};
use http_body_util::Full;
use prost::Message;

use crate::proto::opentelemetry::proto::collector::{
    logs::v1::{ExportLogsServiceRequest, ExportLogsServiceResponse},
    metrics::v1::{ExportMetricsServiceRequest, ExportMetricsServiceResponse},
    trace::v1::{ExportTraceServiceRequest, ExportTraceServiceResponse},
};

use super::client::{retry_with_backoff, ExportResult, RetryConfig, MAX_RESPONSE_SIZE};
use super::grpc::SignalKind;
use super::http_client::HyperOtelClient;
use super::json::request_to_json;

const CONTENT_TYPE_PROTOBUF: &str = "application/x-protobuf";
const CONTENT_TYPE_JSON: &str = "application/json";
const USER_AGENT: &str = concat!("Ferron/", env!("CARGO_PKG_VERSION"));

/// Gzip-compress a fully buffered request body.
fn gzip_compress(body: &[u8]) -> Bytes {
    use std::io::Write;

    use flate2::write::GzEncoder;
    use flate2::Compression;

    let mut encoder = GzEncoder::new(Vec::with_capacity(body.len() / 2), Compression::default());
    encoder
        .write_all(body)
        .expect("writing into a Vec must not fail");
    encoder
        .finish()
        .expect("gzip compression must not fail")
        .into()
}

/// HTTP statuses that are retryable per the OTLP specification.
const RETRYABLE_STATUSES: [StatusCode; 4] = [
    StatusCode::TOO_MANY_REQUESTS,
    StatusCode::BAD_GATEWAY,
    StatusCode::SERVICE_UNAVAILABLE,
    StatusCode::GATEWAY_TIMEOUT,
];

/// HTTP transport for one OTLP signal: POSTs the encoded request to the
/// configured endpoint (which includes the `/v1/<signal>` path) with
/// `Content-Type: application/x-protobuf` or `application/json`.
pub struct HttpSignal {
    client: HyperOtelClient,
    endpoint: String,
    json: bool,
    gzip: bool,
    authorization: Option<HeaderValue>,
}

impl HttpSignal {
    /// Build an HTTP transport for one signal.
    pub fn new(
        _kind: SignalKind,
        endpoint: String,
        json: bool,
        no_verify: bool,
        authorization: Option<&str>,
        gzip: bool,
    ) -> Result<Self, Box<dyn Error + Send + Sync>> {
        Ok(Self {
            client: HyperOtelClient::new(no_verify)?,
            endpoint,
            json,
            gzip,
            authorization: authorization.map(HeaderValue::from_str).transpose()?,
        })
    }

    /// Export a batch of log records over HTTP, with retry/backoff.
    pub async fn export_logs(
        &self,
        request: &ExportLogsServiceRequest,
        retry: &RetryConfig,
    ) -> ExportResult {
        self.export_with(request, retry, |response: &ExportLogsServiceResponse| {
            partial_success_count(
                response
                    .partial_success
                    .as_ref()
                    .filter(|partial| partial.rejected_log_records > 0),
            )
        })
        .await
    }

    /// Export a batch of metric data points over HTTP, with retry/backoff.
    pub async fn export_metrics(
        &self,
        request: &ExportMetricsServiceRequest,
        retry: &RetryConfig,
    ) -> ExportResult {
        self.export_with(request, retry, |response: &ExportMetricsServiceResponse| {
            partial_success_count(
                response
                    .partial_success
                    .as_ref()
                    .filter(|partial| partial.rejected_data_points > 0),
            )
        })
        .await
    }

    /// Export a batch of spans over HTTP, with retry/backoff.
    pub async fn export_traces(
        &self,
        request: &ExportTraceServiceRequest,
        retry: &RetryConfig,
    ) -> ExportResult {
        self.export_with(request, retry, |response: &ExportTraceServiceResponse| {
            partial_success_count(
                response
                    .partial_success
                    .as_ref()
                    .filter(|partial| partial.rejected_spans > 0),
            )
        })
        .await
    }

    /// Encode and export a request with retry/backoff, decoding the response
    /// and surfacing `partial_success` per signal.
    async fn export_with<Req, Resp, F>(
        &self,
        request: &Req,
        retry: &RetryConfig,
        extract_rejected: F,
    ) -> ExportResult
    where
        Req: Message + Default + serde::Serialize,
        Resp: Message + Default + serde::de::DeserializeOwned,
        F: Fn(&Resp) -> (u64, String),
    {
        let body = self.encode(request);
        retry_with_backoff(retry, || {
            self.single_attempt::<Resp, _>(&body, &extract_rejected)
        })
        .await
    }

    /// Encode the request body: OTLP JSON (pbjson + hex-ID handling) or
    /// binary protobuf, optionally gzip-compressed.
    fn encode<T>(&self, request: &T) -> Bytes
    where
        T: serde::Serialize + Message + Default,
    {
        let body = if self.json {
            serde_json::to_vec(&request_to_json(request))
                .expect("OTLP request JSON serialization must not fail")
        } else {
            request.encode_to_vec()
        };
        if self.gzip {
            gzip_compress(&body)
        } else {
            body.into()
        }
    }

    fn content_type(&self) -> &'static str {
        if self.json {
            CONTENT_TYPE_JSON
        } else {
            CONTENT_TYPE_PROTOBUF
        }
    }

    /// One POST attempt: encode-free, sends the pre-built body, classifies
    /// the response, and decodes the response body on HTTP 200.
    async fn single_attempt<Resp, F>(&self, body: &Bytes, extract_rejected: &F) -> ExportResult
    where
        Resp: Message + Default + serde::de::DeserializeOwned,
        F: Fn(&Resp) -> (u64, String),
    {
        let mut request = match hyper::Request::builder()
            .method(http::Method::POST)
            .uri(&self.endpoint)
            .header(http::header::CONTENT_TYPE, self.content_type())
            .header(http::header::ACCEPT, self.content_type())
            .header(http::header::USER_AGENT, USER_AGENT)
            .body(Full::new(body.clone()))
        {
            Ok(request) => request,
            Err(err) => {
                return ExportResult::Failure {
                    retryable: false,
                    retry_after: None,
                    message: err.to_string(),
                }
            }
        };
        if self.gzip {
            request.headers_mut().insert(
                http::header::CONTENT_ENCODING,
                HeaderValue::from_static("gzip"),
            );
        }
        if let Some(authorization) = &self.authorization {
            request
                .headers_mut()
                .insert(http::header::AUTHORIZATION, authorization.clone());
        }

        let response = match self.client.send(request, MAX_RESPONSE_SIZE).await {
            Ok(response) => response,
            Err(err) => {
                return ExportResult::Failure {
                    retryable: !matches!(err, super::http_client::ClientError::TooLargeResponse),
                    retry_after: None,
                    message: err.to_string(),
                }
            }
        };

        let status = response.status();
        if status == StatusCode::OK {
            return self.parse_success_response::<Resp, F>(&response, extract_rejected);
        }

        let retryable = RETRYABLE_STATUSES.contains(&status);
        let retry_after = parse_retry_after(response.headers());
        let message = if retryable {
            format!("OTLP receiver returned HTTP {status}")
        } else {
            format!(
                "OTLP receiver returned HTTP {status}: {}",
                String::from_utf8_lossy(response.body())
                    .chars()
                    .take(200)
                    .collect::<String>()
            )
        };
        ExportResult::Failure {
            retryable,
            retry_after,
            message,
        }
    }

    /// Decode the response body (protobuf or JSON) and report partial success.
    fn parse_success_response<Resp, F>(
        &self,
        response: &http::Response<Bytes>,
        extract_rejected: &F,
    ) -> ExportResult
    where
        Resp: Message + Default + serde::de::DeserializeOwned,
        F: Fn(&Resp) -> (u64, String),
    {
        let decoded: Result<Resp, String> = if self.json {
            serde_json::from_slice(response.body()).map_err(|err| err.to_string())
        } else {
            Resp::decode(response.body().as_ref()).map_err(|err| err.to_string())
        };
        match decoded {
            Ok(parsed) => {
                let (rejected, message) = extract_rejected(&parsed);
                if rejected > 0 {
                    ExportResult::PartialSuccess { rejected, message }
                } else {
                    ExportResult::Success
                }
            }
            Err(err) => ExportResult::Failure {
                retryable: false,
                retry_after: None,
                message: format!("could not parse OTLP response body: {err}"),
            },
        }
    }
}

fn partial_success_count<T>(partial: Option<&T>) -> (u64, String)
where
    T: PartialSuccess,
{
    match partial {
        Some(partial) => (partial.rejected() as u64, partial.message().to_string()),
        None => (0, String::new()),
    }
}

/// Accessors for the generated `Export*PartialSuccess` messages.
trait PartialSuccess {
    fn rejected(&self) -> i64;
    fn message(&self) -> &str;
}

impl PartialSuccess
    for crate::proto::opentelemetry::proto::collector::logs::v1::ExportLogsPartialSuccess
{
    fn rejected(&self) -> i64 {
        self.rejected_log_records
    }
    fn message(&self) -> &str {
        &self.error_message
    }
}

impl PartialSuccess
    for crate::proto::opentelemetry::proto::collector::metrics::v1::ExportMetricsPartialSuccess
{
    fn rejected(&self) -> i64 {
        self.rejected_data_points
    }
    fn message(&self) -> &str {
        &self.error_message
    }
}

impl PartialSuccess
    for crate::proto::opentelemetry::proto::collector::trace::v1::ExportTracePartialSuccess
{
    fn rejected(&self) -> i64 {
        self.rejected_spans
    }
    fn message(&self) -> &str {
        &self.error_message
    }
}

/// Parse the `Retry-After` header as delta-seconds (the form used by OTLP
/// receivers).
fn parse_retry_after(headers: &http::HeaderMap) -> Option<Duration> {
    headers
        .get(http::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use bytes::Bytes;
    use prost::Message;

    use crate::proto::opentelemetry::proto::collector::trace::v1::{
        ExportTracePartialSuccess, ExportTraceServiceRequest, ExportTraceServiceResponse,
    };
    use crate::proto::opentelemetry::proto::resource::v1::Resource;
    use crate::proto::opentelemetry::proto::trace::v1::{ResourceSpans, ScopeSpans, Span};

    use super::*;

    type Handler = Arc<dyn Fn(http::Request<Bytes>) -> http::Response<Full<Bytes>> + Send + Sync>;

    /// A captured request: body bytes plus the content-type and authorization
    /// headers the transport sent.
    #[derive(Default)]
    struct CapturedRequest {
        body: Bytes,
        content_type: Option<String>,
        authorization: Option<String>,
    }

    async fn spawn_http_server(handler: Handler) -> SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let handler = handler.clone();
                let service = hyper::service::service_fn(
                    move |req: hyper::Request<hyper::body::Incoming>| {
                        let handler = handler.clone();
                        async move {
                            let (parts, body) = req.into_parts();
                            let bytes = http_body_util::BodyExt::collect(body)
                                .await
                                .map_err(|err| std::io::Error::other(err.to_string()))?
                                .to_bytes();
                            Ok::<_, std::io::Error>(handler(http::Request::from_parts(
                                parts, bytes,
                            )))
                        }
                    },
                );
                tokio::spawn(async move {
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                        .await;
                });
            }
        });
        addr
    }

    /// A small but realistic trace request.
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

    /// Small backoffs so the retry tests run fast.
    fn test_retry() -> RetryConfig {
        RetryConfig {
            max_attempts: 3,
            initial_backoff: Duration::from_millis(10),
            max_backoff: Duration::from_millis(200),
        }
    }

    #[tokio::test]
    async fn http_protobuf_traces_roundtrip_with_content_type() {
        let captured = Arc::new(Mutex::new(None::<CapturedRequest>));
        let handler = {
            let captured = captured.clone();
            move |req: http::Request<Bytes>| {
                *captured.lock().unwrap() = Some(CapturedRequest {
                    content_type: req
                        .headers()
                        .get(http::header::CONTENT_TYPE)
                        .map(|v| v.to_str().unwrap().to_string()),
                    authorization: req
                        .headers()
                        .get(http::header::AUTHORIZATION)
                        .map(|v| v.to_str().unwrap().to_string()),
                    body: req.into_body(),
                });
                http::Response::builder()
                    .status(StatusCode::OK)
                    .body(Full::new(Bytes::new()))
                    .unwrap()
            }
        };
        let addr = spawn_http_server(Arc::new(handler)).await;
        let signal = HttpSignal::new(
            SignalKind::Traces,
            format!("http://{addr}/v1/traces"),
            false,
            false,
            Some("Bearer token"),
            false,
        )
        .unwrap();

        let request = sample_trace_request();
        let result = signal.export_traces(&request, &test_retry()).await;

        assert_eq!(result, ExportResult::Success);
        let captured = captured.lock().unwrap().take().unwrap();
        assert_eq!(
            captured.content_type.as_deref(),
            Some(CONTENT_TYPE_PROTOBUF)
        );
        assert_eq!(captured.authorization.as_deref(), Some("Bearer token"));
        let decoded = ExportTraceServiceRequest::decode(captured.body.as_ref()).unwrap();
        assert_eq!(decoded, request);
    }

    #[tokio::test]
    async fn http_gzip_compresses_body_and_sets_content_encoding() {
        use std::io::Read;

        let captured = Arc::new(Mutex::new(None::<CapturedRequest>));
        let handler = {
            let captured = captured.clone();
            move |req: http::Request<Bytes>| {
                *captured.lock().unwrap() = Some(CapturedRequest {
                    content_type: req
                        .headers()
                        .get(http::header::CONTENT_TYPE)
                        .map(|v| v.to_str().unwrap().to_string()),
                    authorization: req
                        .headers()
                        .get(http::header::CONTENT_ENCODING)
                        .map(|v| v.to_str().unwrap().to_string()),
                    body: req.into_body(),
                });
                http::Response::builder()
                    .status(StatusCode::OK)
                    .body(Full::new(Bytes::new()))
                    .unwrap()
            }
        };
        let addr = spawn_http_server(Arc::new(handler)).await;
        let signal = HttpSignal::new(
            SignalKind::Traces,
            format!("http://{addr}/v1/traces"),
            false,
            false,
            None,
            true,
        )
        .unwrap();

        let request = sample_trace_request();
        let result = signal.export_traces(&request, &test_retry()).await;

        assert_eq!(result, ExportResult::Success);
        let captured = captured.lock().unwrap().take().unwrap();
        assert_eq!(captured.authorization.as_deref(), Some("gzip"));
        assert_eq!(&captured.body[..2], &[0x1F, 0x8B], "gzip magic bytes");
        let mut decoder = flate2::read::GzDecoder::new(captured.body.as_ref());
        let mut decoded = Vec::new();
        decoder.read_to_end(&mut decoded).unwrap();
        let decoded = ExportTraceServiceRequest::decode(decoded.as_ref()).unwrap();
        assert_eq!(decoded, request);
    }

    #[tokio::test]
    async fn http_json_roundtrip_with_hex_ids_and_partial_success() {
        let captured = Arc::new(Mutex::new(None::<CapturedRequest>));
        let handler = {
            let captured = captured.clone();
            move |req: http::Request<Bytes>| {
                *captured.lock().unwrap() = Some(CapturedRequest {
                    content_type: req
                        .headers()
                        .get(http::header::CONTENT_TYPE)
                        .map(|v| v.to_str().unwrap().to_string()),
                    authorization: None,
                    body: req.into_body(),
                });
                http::Response::builder()
                    .status(StatusCode::OK)
                    .body(Full::new(Bytes::from_static(
                        br#"{"partialSuccess":{"rejectedSpans":2,"errorMessage":"nope"}}"#,
                    )))
                    .unwrap()
            }
        };
        let addr = spawn_http_server(Arc::new(handler)).await;
        let signal = HttpSignal::new(
            SignalKind::Traces,
            format!("http://{addr}/v1/traces"),
            true,
            false,
            None,
            false,
        )
        .unwrap();

        let result = signal
            .export_traces(&sample_trace_request(), &test_retry())
            .await;

        assert_eq!(
            result,
            ExportResult::PartialSuccess {
                rejected: 2,
                message: "nope".to_string()
            }
        );
        let captured = captured.lock().unwrap().take().unwrap();
        assert_eq!(captured.content_type.as_deref(), Some(CONTENT_TYPE_JSON));
        let json: serde_json::Value = serde_json::from_slice(&captured.body).unwrap();
        // OTLP JSON deviations: hex IDs and integer enums.
        assert_eq!(
            json["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["traceId"],
            "5B8E"
        );
        assert_eq!(
            json["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["spanId"],
            "EEE1"
        );
        assert_eq!(
            json["resourceSpans"][0]["resource"]["attributes"][0]["key"],
            "service.name"
        );
    }

    #[tokio::test]
    async fn http_retries_retryable_status_then_succeeds() {
        let requests = Arc::new(AtomicUsize::new(0));
        let handler = {
            let requests = requests.clone();
            move |_req: http::Request<Bytes>| {
                let count = requests.fetch_add(1, Ordering::SeqCst);
                let status = if count < 2 {
                    StatusCode::SERVICE_UNAVAILABLE
                } else {
                    StatusCode::OK
                };
                http::Response::builder()
                    .status(status)
                    .body(Full::new(Bytes::new()))
                    .unwrap()
            }
        };
        let addr = spawn_http_server(Arc::new(handler)).await;
        let signal = HttpSignal::new(
            SignalKind::Traces,
            format!("http://{addr}/v1/traces"),
            false,
            false,
            None,
            false,
        )
        .unwrap();

        let result = signal
            .export_traces(&sample_trace_request(), &test_retry())
            .await;

        assert_eq!(result, ExportResult::Success);
        assert_eq!(requests.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn http_retry_after_zero_retries_immediately() {
        let requests = Arc::new(AtomicUsize::new(0));
        let handler = {
            let requests = requests.clone();
            move |_req: http::Request<Bytes>| {
                let count = requests.fetch_add(1, Ordering::SeqCst);
                if count == 0 {
                    http::Response::builder()
                        .status(StatusCode::SERVICE_UNAVAILABLE)
                        .header(http::header::RETRY_AFTER, "0")
                        .body(Full::new(Bytes::new()))
                        .unwrap()
                } else {
                    http::Response::builder()
                        .status(StatusCode::OK)
                        .body(Full::new(Bytes::new()))
                        .unwrap()
                }
            }
        };
        let addr = spawn_http_server(Arc::new(handler)).await;
        let signal = HttpSignal::new(
            SignalKind::Traces,
            format!("http://{addr}/v1/traces"),
            false,
            false,
            None,
            false,
        )
        .unwrap();

        let started = Instant::now();
        let result = signal
            .export_traces(&sample_trace_request(), &test_retry())
            .await;

        assert_eq!(result, ExportResult::Success);
        assert_eq!(requests.load(Ordering::SeqCst), 2);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "a zero Retry-After must not delay the retry"
        );
    }

    #[test]
    fn parse_retry_after_reads_delta_seconds() {
        let mut headers = http::HeaderMap::new();
        headers.insert(
            http::header::RETRY_AFTER,
            http::HeaderValue::from_static("120"),
        );
        assert_eq!(parse_retry_after(&headers), Some(Duration::from_secs(120)));

        headers.insert(
            http::header::RETRY_AFTER,
            http::HeaderValue::from_static("not-a-number"),
        );
        assert_eq!(parse_retry_after(&headers), None);

        headers.remove(http::header::RETRY_AFTER);
        assert_eq!(parse_retry_after(&headers), None);
    }

    #[tokio::test]
    async fn http_non_retryable_status_is_not_retried() {
        let requests = Arc::new(AtomicUsize::new(0));
        let handler = {
            let requests = requests.clone();
            move |_req: http::Request<Bytes>| {
                requests.fetch_add(1, Ordering::SeqCst);
                http::Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Full::new(Bytes::from_static(b"bad request")))
                    .unwrap()
            }
        };
        let addr = spawn_http_server(Arc::new(handler)).await;
        let signal = HttpSignal::new(
            SignalKind::Traces,
            format!("http://{addr}/v1/traces"),
            false,
            false,
            None,
            false,
        )
        .unwrap();

        let result = signal
            .export_traces(&sample_trace_request(), &test_retry())
            .await;

        assert!(
            matches!(
                &result,
                ExportResult::Failure {
                    retryable: false,
                    message,
                    ..
                } if message.contains("400")
            ),
            "unexpected result: {result:?}"
        );
        assert_eq!(requests.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn http_response_larger_than_cap_is_dropped() {
        let handler = {
            move |_req: http::Request<Bytes>| {
                http::Response::builder()
                    .status(StatusCode::OK)
                    .body(Full::new(Bytes::from(vec![0u8; MAX_RESPONSE_SIZE + 1])))
                    .unwrap()
            }
        };
        let addr = spawn_http_server(Arc::new(handler)).await;
        let signal = HttpSignal::new(
            SignalKind::Traces,
            format!("http://{addr}/v1/traces"),
            false,
            false,
            None,
            false,
        )
        .unwrap();

        let result = signal
            .export_traces(&sample_trace_request(), &test_retry())
            .await;

        assert!(
            matches!(
                &result,
                ExportResult::Failure {
                    retryable: false,
                    message,
                    ..
                } if message.contains("size cap")
            ),
            "unexpected result: {result:?}"
        );
    }

    #[tokio::test]
    async fn http_protobuf_partial_success_is_reported_not_retried() {
        let requests = Arc::new(AtomicUsize::new(0));
        let handler = {
            let requests = requests.clone();
            move |_req: http::Request<Bytes>| {
                requests.fetch_add(1, Ordering::SeqCst);
                let response = ExportTraceServiceResponse {
                    partial_success: Some(ExportTracePartialSuccess {
                        rejected_spans: 3,
                        error_message: "too many spans".to_string(),
                    }),
                };
                http::Response::builder()
                    .status(StatusCode::OK)
                    .body(Full::new(Bytes::from(response.encode_to_vec())))
                    .unwrap()
            }
        };
        let addr = spawn_http_server(Arc::new(handler)).await;
        let signal = HttpSignal::new(
            SignalKind::Traces,
            format!("http://{addr}/v1/traces"),
            false,
            false,
            None,
            false,
        )
        .unwrap();

        let result = signal
            .export_traces(&sample_trace_request(), &test_retry())
            .await;

        assert_eq!(
            result,
            ExportResult::PartialSuccess {
                rejected: 3,
                message: "too many spans".to_string()
            }
        );
        assert_eq!(requests.load(Ordering::SeqCst), 1);
    }
}
