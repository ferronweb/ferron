//! Batch trace pipeline: finished spans flow through a bounded buffer and
//! are exported in batches by a background task.
//!
//! The event loop (see `crate::OtlpObservabilityModule::start`) pushes
//! finished spans into the [`TraceBuffer`]. The [`BatchTraceExporter`] task
//! flushes the buffer when it reaches `batch_size` or when `interval`
//! elapses, wraps each batch into an `ExportTraceServiceRequest` (resource +
//! scope), and exports it through the transport. On shutdown the buffer is
//! drained before the task exits.
//!
//! The buffer is bounded: when it is full (an export is in progress and the
//! buffer refills to capacity), the newest finished span is dropped and a
//! dropped counter is incremented.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Once};
use std::time::Duration;

use tokio::sync::{oneshot, Notify};
use tokio_util::sync::CancellationToken;

use crate::convert::{build_resource, build_scope};
use crate::proto::opentelemetry::proto::collector::trace::v1::ExportTraceServiceRequest;
use crate::proto::opentelemetry::proto::common::v1::InstrumentationScope;
use crate::proto::opentelemetry::proto::resource::v1::Resource;
use crate::proto::opentelemetry::proto::trace::v1::{ResourceSpans, ScopeSpans, Span};
use crate::transport::client::{ExportResult, OtlpTransport};

/// Default number of finished spans that trigger an export.
pub const DEFAULT_BATCH_SIZE: usize = 512;
/// Default upper bound on buffered finished spans. New spans are dropped
/// when the buffer is full (mirrors the SDK batch processor default queue).
pub const DEFAULT_QUEUE_CAPACITY: usize = 2048;
/// Default interval at which a partially full buffer is flushed.
pub const DEFAULT_FLUSH_INTERVAL: Duration = Duration::from_secs(5);
/// Default upper bound on one export round (including transport retries).
pub const DEFAULT_EXPORT_TIMEOUT: Duration = Duration::from_secs(30);

/// Batching parameters for the trace exporter.
#[derive(Debug, Clone, Copy)]
pub struct BatchConfig {
    /// Number of finished spans that trigger a flush.
    pub batch_size: usize,
    /// Upper bound on buffered finished spans.
    pub queue_capacity: usize,
    /// Interval at which a partially full buffer is flushed.
    pub interval: Duration,
    /// Upper bound on one export round (including transport retries).
    pub export_timeout: Duration,
}

impl Default for BatchConfig {
    fn default() -> Self {
        Self {
            batch_size: DEFAULT_BATCH_SIZE,
            queue_capacity: DEFAULT_QUEUE_CAPACITY,
            interval: DEFAULT_FLUSH_INTERVAL,
            export_timeout: DEFAULT_EXPORT_TIMEOUT,
        }
    }
}

/// Bounded queue of finished spans, shared between the event loop (pusher)
/// and the exporter task (drainer). The queue is unbounded in growth only up
/// to `queue_capacity`; a new span is dropped (newest dropped) when the
/// queue is full, and the dropped counter is incremented.
#[derive(Clone)]
pub(crate) struct TraceBuffer {
    inner: Arc<TraceBufferInner>,
}

struct TraceBufferInner {
    spans: crossbeam_queue::SegQueue<Span>,
    notify: Notify,
    capacity: usize,
    dropped: AtomicU64,
}

impl TraceBuffer {
    fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(TraceBufferInner {
                spans: crossbeam_queue::SegQueue::new(),
                notify: Notify::new(),
                capacity,
                dropped: AtomicU64::new(0),
            }),
        }
    }

    /// Store a finished span, waking the exporter. Returns `false` (and
    /// increments the dropped counter) when the buffer is full.
    pub(crate) fn push(&self, span: Span) -> bool {
        let spans = &self.inner.spans;
        if spans.len() >= self.inner.capacity {
            self.record_dropped(1);
            return false;
        }
        spans.push(span);
        self.inner.notify.notify_one();
        true
    }

    /// Remove up to `max` spans from the front of the queue.
    pub(crate) fn drain_batch(&self, max: usize) -> Vec<Span> {
        let spans = &self.inner.spans;
        let n = spans.len().min(max);
        let mut batch = Vec::with_capacity(n);
        for _ in 0..n {
            if let Some(span) = spans.pop() {
                batch.push(span);
            } else {
                break;
            }
        }
        batch
    }

    /// Current number of buffered spans.
    pub(crate) fn len(&self) -> usize {
        self.inner.spans.len()
    }

    /// Total number of spans dropped (queue full or export failure).
    #[cfg(test)]
    pub(crate) fn dropped(&self) -> u64 {
        self.inner.dropped.load(Ordering::Relaxed)
    }

    fn record_dropped(&self, n: u64) {
        self.inner.dropped.fetch_add(n, Ordering::Relaxed);
        ferron_core::admin::ADMIN_METRICS
            .observability_events_dropped
            .fetch_add(n, Ordering::Relaxed);
        warn_once();
    }
}

static DROPPED_SPANS: Once = Once::new();

fn warn_once() {
    DROPPED_SPANS.call_once(|| {
        ferron_core::log_warn!(
            "OTLP trace spans dropped (`otlp` observability backend). \
        This may be caused by a full buffer or a failing OTLP receiver."
        );
    });
}

/// Export a batch of spans. Implemented by the real transport and by mocks
/// in tests.
pub(crate) trait TraceExporter: Send + Sync {
    /// Export one batch, applying retry/backoff internally.
    fn export_traces<'a>(&'a self, request: &'a ExportTraceServiceRequest) -> ExportFuture<'a>;
}

/// A pending export of one batch.
pub(crate) type ExportFuture<'a> = Pin<Box<dyn Future<Output = ExportResult> + Send + 'a>>;

impl TraceExporter for OtlpTransport {
    fn export_traces<'a>(&'a self, request: &'a ExportTraceServiceRequest) -> ExportFuture<'a> {
        Box::pin(OtlpTransport::export_traces(self, request))
    }
}

/// The exporter task state: drains the buffer, wraps batches into requests,
/// and exports them. Owned by a spawned background task.
struct BatchTraceExporter {
    buffer: TraceBuffer,
    exporter: Arc<dyn TraceExporter>,
    resource: Resource,
    scope: InstrumentationScope,
    config: BatchConfig,
}

impl BatchTraceExporter {
    /// Flush buffered spans in batches of `batch_size` until the buffer is
    /// empty. Used on interval ticks, on batch-size triggers, and on
    /// shutdown drain.
    async fn flush(&self) {
        loop {
            let spans = self.buffer.drain_batch(self.config.batch_size);
            if spans.is_empty() {
                return;
            }
            self.export(spans).await;
        }
    }

    /// Export one batch, dropping and counting it on persistent failure or
    /// when the export timeout elapses.
    async fn export(&self, spans: Vec<Span>) {
        let request = ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                resource: Some(self.resource.clone()),
                scope_spans: vec![ScopeSpans {
                    scope: Some(self.scope.clone()),
                    spans,
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        let count = request.resource_spans[0].scope_spans[0].spans.len();
        let result = tokio::time::timeout(
            self.config.export_timeout,
            self.exporter.export_traces(&request),
        )
        .await;
        match result {
            Ok(ExportResult::Success | ExportResult::PartialSuccess { .. }) => {}
            Ok(ExportResult::Failure { .. }) | Err(_) => {
                self.buffer.record_dropped(count as u64);
            }
        }
    }

    /// Run until cancellation, then drain the buffer before returning.
    async fn run(self, cancel: CancellationToken) {
        let mut interval = tokio::time::interval(self.config.interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = interval.tick() => self.flush().await,
                _ = self.buffer.inner.notify.notified() => {
                    if self.buffer.len() >= self.config.batch_size {
                        self.flush().await;
                    }
                }
            }
        }
        self.flush().await;
    }
}

/// Handle to a spawned batch trace exporter: the shared buffer (for the
/// event loop to push into) and a completion signal for shutdown draining.
pub(crate) struct TracePipeline {
    pub(crate) buffer: TraceBuffer,
    done: oneshot::Receiver<()>,
}

impl TracePipeline {
    /// Spawn the exporter background task with default batching parameters.
    ///
    /// `cancel` is the module's `CancellationToken`: when it is cancelled
    /// the exporter drains the buffer and exits.
    pub(crate) fn spawn(
        exporter: Arc<dyn TraceExporter>,
        service_name: String,
        cancel: CancellationToken,
    ) -> Self {
        Self::spawn_with_config(exporter, service_name, cancel, BatchConfig::default())
    }

    fn spawn_with_config(
        exporter: Arc<dyn TraceExporter>,
        service_name: String,
        cancel: CancellationToken,
        config: BatchConfig,
    ) -> Self {
        let buffer = TraceBuffer::new(config.queue_capacity);
        let (done_tx, done_rx) = oneshot::channel();
        let task = BatchTraceExporter {
            buffer: buffer.clone(),
            exporter,
            resource: build_resource(service_name),
            scope: build_scope("ferron"),
            config,
        };
        tokio::spawn(async move {
            task.run(cancel).await;
            let _ = done_tx.send(());
        });
        Self {
            buffer,
            done: done_rx,
        }
    }

    /// Wait for the exporter task to finish its shutdown drain.
    pub(crate) async fn wait_done(self) {
        let _ = self.done.await;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    use super::*;
    use tokio::sync::Mutex;

    fn test_config() -> BatchConfig {
        BatchConfig {
            batch_size: 16,
            queue_capacity: 64,
            interval: Duration::from_secs(3600),
            export_timeout: Duration::from_secs(5),
        }
    }

    fn span(name: &str) -> Span {
        Span {
            name: name.to_string(),
            trace_id: vec![0x11],
            span_id: vec![0x22],
            ..Default::default()
        }
    }

    /// Mock exporter: records requests, optionally fails them, and
    /// optionally blocks until released.
    #[derive(Default)]
    struct MockExporter {
        requests: Mutex<Vec<ExportTraceServiceRequest>>,
        fail: AtomicBool,
        gate: Mutex<Option<oneshot::Receiver<()>>>,
    }

    impl MockExporter {
        async fn request_count(&self) -> usize {
            self.requests.lock().await.len()
        }

        async fn span_count(&self) -> usize {
            self.requests
                .lock()
                .await
                .iter()
                .flat_map(|request| &request.resource_spans)
                .flat_map(|resource_spans| &resource_spans.scope_spans)
                .map(|scope_spans| scope_spans.spans.len())
                .sum()
        }
    }

    impl TraceExporter for MockExporter {
        fn export_traces<'a>(&'a self, request: &'a ExportTraceServiceRequest) -> ExportFuture<'a> {
            Box::pin(async move {
                self.requests.lock().await.push(request.clone());
                let gate = self.gate.lock().await.take();
                if let Some(released) = gate {
                    let _ = released.await;
                }
                if self.fail.load(Ordering::Relaxed) {
                    ExportResult::Failure {
                        retryable: true,
                        retry_after: None,
                        message: "mock failure".to_string(),
                    }
                } else {
                    ExportResult::Success
                }
            })
        }
    }

    fn spawn_test(
        exporter: Arc<MockExporter>,
        config: BatchConfig,
    ) -> (TracePipeline, CancellationToken) {
        let cancel = CancellationToken::new();
        let pipeline = TracePipeline::spawn_with_config(
            exporter,
            "test-service".into(),
            cancel.clone(),
            config,
        );
        (pipeline, cancel)
    }

    async fn wait_until<F, Fut>(mut condition: F)
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = bool>,
    {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !condition().await {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("condition not met within 5 s");
    }

    #[tokio::test]
    async fn flushes_in_batches_of_batch_size() {
        let mock = Arc::new(MockExporter::default());
        let (pipeline, cancel) = spawn_test(mock.clone(), test_config());

        for i in 0..40 {
            pipeline.buffer.push(span(&format!("s{i}")));
        }

        cancel.cancel();
        pipeline.wait_done().await;

        let requests = mock.requests.lock().await;
        // ceil(40 / 16) = 3 export calls, each at most 16 spans.
        assert_eq!(requests.len(), 3);
        let spans_per_call: Vec<usize> = requests
            .iter()
            .map(|request| request.resource_spans[0].scope_spans[0].spans.len())
            .collect();
        assert_eq!(spans_per_call.iter().sum::<usize>(), 40);
        assert!(spans_per_call.iter().all(|count| *count <= 16));
    }

    #[tokio::test]
    async fn flushes_partial_batch_on_interval() {
        let mock = Arc::new(MockExporter::default());
        let mut config = test_config();
        config.batch_size = 1024;
        config.queue_capacity = 2048;
        config.interval = Duration::from_millis(50);
        let (pipeline, cancel) = spawn_test(mock.clone(), config);

        pipeline.buffer.push(span("a"));
        pipeline.buffer.push(span("b"));
        pipeline.buffer.push(span("c"));

        wait_until(|| async { mock.request_count().await == 1 }).await;

        let requests = mock.requests.lock().await;
        let resource_spans = &requests[0].resource_spans[0];
        let service_name = resource_spans
            .resource
            .as_ref()
            .unwrap()
            .attributes
            .iter()
            .find(|attribute| attribute.key == "service.name")
            .unwrap();
        let value = service_name.value.as_ref().unwrap();
        let crate::proto::opentelemetry::proto::common::v1::any_value::Value::StringValue(text) =
            value.value.as_ref().unwrap()
        else {
            panic!("service.name is not a string");
        };
        assert_eq!(text, "test-service");
        let scope_spans = &resource_spans.scope_spans[0];
        assert_eq!(scope_spans.scope.as_ref().unwrap().name, "ferron");
        let names: Vec<&str> = scope_spans
            .spans
            .iter()
            .map(|span| span.name.as_str())
            .collect();
        assert_eq!(names, ["a", "b", "c"]);
        drop(requests);

        cancel.cancel();
        pipeline.wait_done().await;
        assert_eq!(mock.request_count().await, 1);
    }

    #[tokio::test]
    async fn drops_newest_spans_when_queue_full() {
        let mock = Arc::new(MockExporter::default());
        let mut config = test_config();
        config.batch_size = 4;
        config.queue_capacity = 8;
        let (pipeline, cancel) = spawn_test(mock.clone(), config);

        // Block the first export in flight, so the buffer refills while the
        // exporter is busy.
        let (block_tx, block_rx) = oneshot::channel();
        *mock.gate.lock().await = Some(block_rx);
        for i in 0..8 {
            pipeline.buffer.push(span(&format!("s{i}")));
        }
        wait_until(|| async { mock.request_count().await == 1 }).await;

        // 4 spans are drained into the blocked export; 4 more fill the
        // buffer, the last 2 are dropped.
        for i in 8..14 {
            pipeline.buffer.push(span(&format!("s{i}")));
        }
        assert_eq!(pipeline.buffer.dropped(), 2);

        let _ = block_tx.send(());
        cancel.cancel();
        pipeline.wait_done().await;

        // 14 pushed, 2 dropped: 12 spans exported in batches of 4.
        assert_eq!(mock.request_count().await, 3);
        assert_eq!(mock.span_count().await, 12);
    }

    #[tokio::test]
    async fn shutdown_drains_buffered_spans() {
        let mock = Arc::new(MockExporter::default());
        let (pipeline, cancel) = spawn_test(mock.clone(), test_config());

        pipeline.buffer.push(span("a"));
        pipeline.buffer.push(span("b"));
        pipeline.buffer.push(span("c"));

        cancel.cancel();
        pipeline.wait_done().await;

        assert_eq!(mock.request_count().await, 1);
        assert_eq!(mock.span_count().await, 3);
    }

    #[tokio::test]
    async fn persistent_failure_drops_batch() {
        let mock = Arc::new(MockExporter::default());
        mock.fail.store(true, Ordering::Relaxed);
        let mut config = test_config();
        config.batch_size = 4;
        let (pipeline, cancel) = spawn_test(mock.clone(), config);

        pipeline.buffer.push(span("a"));
        pipeline.buffer.push(span("b"));
        pipeline.buffer.push(span("c"));
        pipeline.buffer.push(span("d"));

        wait_until(|| async { pipeline.buffer.dropped() == 4 }).await;
        cancel.cancel();
        pipeline.wait_done().await;
        assert_eq!(mock.request_count().await, 1);
    }

    #[tokio::test]
    async fn hung_export_is_bounded_by_timeout() {
        let mock = Arc::new(MockExporter::default());
        let mut config = test_config();
        config.batch_size = 4;
        config.export_timeout = Duration::from_millis(100);
        let (pipeline, cancel) = spawn_test(mock.clone(), config);

        // Block the export forever; the pipeline must give up after the
        // export timeout and count the batch as dropped.
        let (_block_tx, block_rx) = oneshot::channel();
        *mock.gate.lock().await = Some(block_rx);
        for i in 0..4 {
            pipeline.buffer.push(span(&format!("s{i}")));
        }

        wait_until(|| async { pipeline.buffer.dropped() == 4 }).await;
        cancel.cancel();
        pipeline.wait_done().await;
        assert_eq!(mock.request_count().await, 1);
    }
}
