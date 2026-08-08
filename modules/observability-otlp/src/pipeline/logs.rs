//! Batch log pipeline: finished log records flow through a bounded buffer
//! and are exported in batches by a background task.
//!
//! The event loop (see `crate::OtlpObservabilityModule::start`) pushes
//! finished log records into the [`LogBuffer`], tagged with their
//! instrumentation scope name (`"ferron"` for application logs,
//! `"ferron.access"` for access logs, matching the SDK loggers they replace).
//! The [`BatchLogExporter`] task flushes the buffer when it reaches
//! `batch_size` or when `interval` elapses, wraps each batch into an
//! `ExportLogsServiceRequest` (resource + one scope group per scope), and
//! exports it through the transport. On shutdown the buffer is drained
//! before the task exits.
//!
//! The buffer is bounded: when it is full (an export is in progress and the
//! buffer refills to capacity), the newest finished record is dropped and a
//! dropped counter is incremented.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Once};

use tokio::sync::{oneshot, Notify};
use tokio_util::sync::CancellationToken;

use crate::convert::{build_resource, build_scope};
use crate::proto::opentelemetry::proto::collector::logs::v1::ExportLogsServiceRequest;
use crate::proto::opentelemetry::proto::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
use crate::proto::opentelemetry::proto::resource::v1::Resource;
use crate::transport::client::{ExportResult, OtlpTransport};

use super::BatchConfig;

/// A finished log record tagged with the instrumentation scope it belongs
/// to. Scope names are pooled (`"ferron"`, `"ferron.access"`), so the
/// per-record `String` is a single allocation.
type TaggedRecord = (String, LogRecord);

/// Bounded queue of finished log records, shared between the event loop
/// (pusher) and the exporter task (drainer). The queue is bounded at
/// `queue_capacity`; a new record is dropped (newest dropped) when the
/// queue is full, and the dropped counter is incremented.
#[derive(Clone)]
pub(crate) struct LogBuffer {
    inner: Arc<LogBufferInner>,
}

struct LogBufferInner {
    records: crossbeam_queue::SegQueue<TaggedRecord>,
    notify: Notify,
    capacity: usize,
    dropped: AtomicU64,
}

impl LogBuffer {
    #[inline]
    fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(LogBufferInner {
                records: crossbeam_queue::SegQueue::new(),
                notify: Notify::new(),
                capacity,
                dropped: AtomicU64::new(0),
            }),
        }
    }

    /// Store a finished log record, waking the exporter. Returns `false`
    /// (and increments the dropped counter) when the buffer is full.
    #[inline]
    pub(crate) fn push(&self, scope: &str, record: LogRecord) -> bool {
        let records = &self.inner.records;
        if records.len() >= self.inner.capacity {
            self.record_dropped(1);
            return false;
        }
        records.push((scope.to_string(), record));
        self.inner.notify.notify_one();
        true
    }

    /// Remove up to `max` records from the front of the queue.
    #[inline]
    pub(crate) fn drain_batch(&self, max: usize) -> Vec<TaggedRecord> {
        let records = &self.inner.records;
        let n = records.len().min(max);
        let mut batch = Vec::with_capacity(n);
        for _ in 0..n {
            if let Some(record) = records.pop() {
                batch.push(record);
            } else {
                break;
            }
        }
        batch
    }

    /// Current number of buffered records.
    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.inner.records.len()
    }

    /// Total number of records dropped (queue full or export failure).
    #[cfg(test)]
    #[inline]
    pub(crate) fn dropped(&self) -> u64 {
        self.inner.dropped.load(Ordering::Relaxed)
    }

    #[inline]
    fn record_dropped(&self, n: u64) {
        self.inner.dropped.fetch_add(n, Ordering::Relaxed);
        ferron_core::admin::ADMIN_METRICS
            .observability_events_dropped
            .fetch_add(n, Ordering::Relaxed);
        warn_once();
    }
}

static DROPPED_RECORDS: Once = Once::new();

#[inline]
fn warn_once() {
    DROPPED_RECORDS.call_once(|| {
        ferron_core::log_warn!(
            "OTLP log records dropped (`otlp` observability backend). \
        This may be caused by a full buffer or a failing OTLP receiver."
        );
    });
}

/// Export a batch of log records. Implemented by the real transport and by
/// mocks in tests.
pub(crate) trait LogExporter: Send + Sync {
    /// Export one batch, applying retry/backoff internally.
    fn export_logs<'a>(&'a self, request: &'a ExportLogsServiceRequest) -> ExportLogsFuture<'a>;
}

/// A pending export of one batch.
pub(crate) type ExportLogsFuture<'a> = Pin<Box<dyn Future<Output = ExportResult> + Send + 'a>>;

impl LogExporter for OtlpTransport {
    #[inline]
    fn export_logs<'a>(&'a self, request: &'a ExportLogsServiceRequest) -> ExportLogsFuture<'a> {
        Box::pin(OtlpTransport::export_logs(self, request))
    }
}

/// The exporter task state: drains the buffer, wraps batches into requests,
/// and exports them. Owned by a spawned background task.
struct BatchLogExporter {
    buffer: LogBuffer,
    exporter: Arc<dyn LogExporter>,
    resource: Resource,
    config: BatchConfig,
}

impl BatchLogExporter {
    /// Flush buffered records in batches of `batch_size` until the buffer
    /// is empty. Used on interval ticks, on batch-size triggers, and on
    /// shutdown drain.
    #[inline]
    async fn flush(&self) {
        loop {
            let records = self.buffer.drain_batch(self.config.batch_size);
            if records.is_empty() {
                return;
            }
            self.export(records).await;
        }
    }

    /// Export one batch, grouped by instrumentation scope, dropping and
    /// counting it on persistent failure or when the export timeout
    /// elapses.
    #[inline]
    async fn export(&self, records: Vec<TaggedRecord>) {
        let mut grouped: BTreeMap<String, Vec<LogRecord>> = BTreeMap::new();
        let mut count = 0;
        for (scope, record) in records {
            grouped.entry(scope).or_default().push(record);
            count += 1;
        }
        let scope_logs: Vec<ScopeLogs> = grouped
            .into_iter()
            .map(|(name, records)| ScopeLogs {
                scope: Some(build_scope(&name)),
                log_records: records,
                ..Default::default()
            })
            .collect();
        let request = ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                resource: Some(self.resource.clone()),
                scope_logs,
                ..Default::default()
            }],
        };
        let result = tokio::time::timeout(
            self.config.export_timeout,
            self.exporter.export_logs(&request),
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
    #[inline]
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

/// Handle to a spawned batch log exporter: the shared buffer (for the event
/// loop to push into) and a completion signal for shutdown draining.
pub(crate) struct LogPipeline {
    pub(crate) buffer: LogBuffer,
    done: oneshot::Receiver<()>,
}

impl LogPipeline {
    #[inline]
    pub(crate) fn spawn_with_config(
        exporter: Arc<dyn LogExporter>,
        service_name: String,
        cancel: CancellationToken,
        config: BatchConfig,
    ) -> Self {
        let buffer = LogBuffer::new(config.queue_capacity);
        let (done_tx, done_rx) = oneshot::channel();
        let task = BatchLogExporter {
            buffer: buffer.clone(),
            exporter,
            resource: build_resource(service_name),
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
    #[inline]
    pub(crate) async fn wait_done(self) {
        let _ = self.done.await;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;
    use tokio::sync::Mutex;

    use crate::config::LogStyle;
    use crate::convert::{any_string, build_log_record};
    use ferron_observability::{LogAttributeValue, LogEvent, LogLevel};
    #[inline]
    fn test_config() -> BatchConfig {
        BatchConfig {
            batch_size: 16,
            queue_capacity: 64,
            interval: Duration::from_secs(3600),
            export_timeout: Duration::from_secs(5),
        }
    }

    #[inline]
    fn record(message: &str) -> LogRecord {
        LogRecord {
            body: Some(any_string(message)),
            ..Default::default()
        }
    }

    /// Mock exporter: records requests, optionally fails them, and
    /// optionally blocks until released.
    #[derive(Default)]
    struct MockExporter {
        requests: Mutex<Vec<ExportLogsServiceRequest>>,
        fail: AtomicBool,
        gate: Mutex<Option<oneshot::Receiver<()>>>,
    }

    impl MockExporter {
        #[inline]
        async fn request_count(&self) -> usize {
            self.requests.lock().await.len()
        }

        #[inline]
        async fn record_count(&self) -> usize {
            self.requests
                .lock()
                .await
                .iter()
                .flat_map(|request| &request.resource_logs)
                .flat_map(|resource_logs| &resource_logs.scope_logs)
                .map(|scope_logs| scope_logs.log_records.len())
                .sum()
        }
    }

    impl LogExporter for MockExporter {
        #[inline]
        fn export_logs<'a>(
            &'a self,
            request: &'a ExportLogsServiceRequest,
        ) -> ExportLogsFuture<'a> {
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

    #[inline]
    fn spawn_test(
        exporter: Arc<MockExporter>,
        config: BatchConfig,
    ) -> (LogPipeline, CancellationToken) {
        let cancel = CancellationToken::new();
        let pipeline =
            LogPipeline::spawn_with_config(exporter, "test-service".into(), cancel.clone(), config);
        (pipeline, cancel)
    }

    #[inline]
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

        for _ in 0..40 {
            pipeline.buffer.push("ferron", LogRecord::default());
        }

        cancel.cancel();
        pipeline.wait_done().await;

        let requests = mock.requests.lock().await;
        // ceil(40 / 16) = 3 export calls, each at most 16 records.
        assert_eq!(requests.len(), 3);
        let records_per_call: Vec<usize> = requests
            .iter()
            .map(|request| request.resource_logs[0].scope_logs[0].log_records.len())
            .collect();
        assert_eq!(records_per_call.iter().sum::<usize>(), 40);
        assert!(records_per_call.iter().all(|count| *count <= 16));
    }

    #[tokio::test]
    async fn flushes_partial_batch_on_interval() {
        let mock = Arc::new(MockExporter::default());
        let mut config = test_config();
        config.batch_size = 1024;
        config.queue_capacity = 2048;
        config.interval = Duration::from_millis(50);
        let (pipeline, cancel) = spawn_test(mock.clone(), config);

        pipeline.buffer.push("ferron", record("a"));
        pipeline.buffer.push("ferron", record("b"));
        pipeline.buffer.push("ferron.access", record("c"));

        wait_until(|| async { mock.request_count().await == 1 }).await;

        let requests = mock.requests.lock().await;
        let resource_logs = &requests[0].resource_logs[0];
        let service_name = resource_logs
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
        drop(requests);

        cancel.cancel();
        pipeline.wait_done().await;
        assert_eq!(mock.request_count().await, 1);
    }

    #[tokio::test]
    async fn groups_records_by_scope() {
        let mock = Arc::new(MockExporter::default());
        let (pipeline, cancel) = spawn_test(mock.clone(), test_config());

        pipeline.buffer.push("ferron", record("a"));
        pipeline.buffer.push("ferron", record("b"));
        pipeline.buffer.push("ferron.access", record("c"));

        cancel.cancel();
        pipeline.wait_done().await;

        let requests = mock.requests.lock().await;
        let scope_logs = &requests[0].resource_logs[0].scope_logs;
        let names: Vec<&str> = scope_logs
            .iter()
            .map(|scope_logs| scope_logs.scope.as_ref().unwrap().name.as_str())
            .collect();
        assert_eq!(names, ["ferron", "ferron.access"]);
        let counts: Vec<usize> = scope_logs
            .iter()
            .map(|scope_logs| scope_logs.log_records.len())
            .collect();
        assert_eq!(counts, [2, 1]);
    }

    #[tokio::test]
    async fn drops_newest_records_when_queue_full() {
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
            pipeline.buffer.push("ferron", record(&format!("r{i}")));
        }
        wait_until(|| async { mock.request_count().await == 1 }).await;

        // 4 records are drained into the blocked export; 4 more fill the
        // buffer, the last 2 are dropped.
        for i in 8..14 {
            pipeline.buffer.push("ferron", record(&format!("r{i}")));
        }
        assert_eq!(pipeline.buffer.dropped(), 2);

        let _ = block_tx.send(());
        cancel.cancel();
        pipeline.wait_done().await;

        // 14 pushed, 2 dropped: 12 records exported in batches of 4.
        assert_eq!(mock.request_count().await, 3);
        assert_eq!(mock.record_count().await, 12);
    }

    #[tokio::test]
    async fn shutdown_drains_buffered_records() {
        let mock = Arc::new(MockExporter::default());
        let (pipeline, cancel) = spawn_test(mock.clone(), test_config());

        pipeline.buffer.push("ferron", record("a"));
        pipeline.buffer.push("ferron", record("b"));
        pipeline.buffer.push("ferron", record("c"));

        cancel.cancel();
        pipeline.wait_done().await;

        assert_eq!(mock.request_count().await, 1);
        assert_eq!(mock.record_count().await, 3);
    }

    #[tokio::test]
    async fn persistent_failure_drops_batch() {
        let mock = Arc::new(MockExporter::default());
        mock.fail.store(true, Ordering::Relaxed);
        let mut config = test_config();
        config.batch_size = 4;
        let (pipeline, cancel) = spawn_test(mock.clone(), config);

        for i in 0..4 {
            pipeline.buffer.push("ferron", record(&format!("r{i}")));
        }

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
            pipeline.buffer.push("ferron", record(&format!("r{i}")));
        }

        wait_until(|| async { pipeline.buffer.dropped() == 4 }).await;
        cancel.cancel();
        pipeline.wait_done().await;
        assert_eq!(mock.request_count().await, 1);
    }

    #[tokio::test]
    async fn exports_log_style_bodies_and_attributes_at_wire_level() {
        let mock = Arc::new(MockExporter::default());
        let (pipeline, cancel) = spawn_test(mock.clone(), test_config());

        // Modern style: body = summary, typed attributes, Error severity.
        let modern = LogEvent {
            level: LogLevel::Error,
            message: "human message".into(),
            summary: "short summary".into(),
            target: "test.target",
            attributes: vec![
                ("http.status", LogAttributeValue::I64(500)),
                ("flag", LogAttributeValue::Bool(true)),
                ("ratio", LogAttributeValue::F64(0.5)),
                ("name", LogAttributeValue::StaticStr("x")),
            ],
            trace_context: None,
        };
        let record = build_log_record(&modern, &[], LogStyle::Modern, std::time::SystemTime::now());
        pipeline.buffer.push("ferron", record);

        // Legacy style: body = message, only log.target kept, Warn severity.
        let legacy = LogEvent {
            level: LogLevel::Warn,
            message: "plain message".into(),
            summary: "ignored summary".into(),
            target: "test.target",
            attributes: vec![("http.status", LogAttributeValue::I64(404))],
            trace_context: None,
        };
        let record = build_log_record(&legacy, &[], LogStyle::Legacy, std::time::SystemTime::now());
        pipeline.buffer.push("ferron", record);

        cancel.cancel();
        pipeline.wait_done().await;

        let requests = mock.requests.lock().await;
        let records = &requests[0].resource_logs[0].scope_logs[0].log_records;
        assert_eq!(records.len(), 2);
        #[inline]
        fn body(record: &LogRecord) -> &crate::proto::opentelemetry::proto::common::v1::AnyValue {
            record.body.as_ref().unwrap()
        }
        let crate::proto::opentelemetry::proto::common::v1::any_value::Value::StringValue(
            modern_body,
        ) = body(&records[0]).value.as_ref().unwrap()
        else {
            panic!("modern body is not a string");
        };
        assert_eq!(modern_body, "short summary");
        assert_eq!(records[0].severity_number, 17);
        assert_eq!(records[0].severity_text, "ERROR");
        let modern_attrs: std::collections::HashMap<
            &str,
            &crate::proto::opentelemetry::proto::common::v1::AnyValue,
        > = records[0]
            .attributes
            .iter()
            .map(|attribute| (attribute.key.as_str(), attribute.value.as_ref().unwrap()))
            .collect();
        assert_eq!(modern_attrs.len(), 5);
        assert_eq!(
            modern_attrs["log.target"].value,
            Some(
                crate::proto::opentelemetry::proto::common::v1::any_value::Value::StringValue(
                    "test.target".into()
                )
            )
        );
        assert_eq!(
            modern_attrs["http.status"].value,
            Some(crate::proto::opentelemetry::proto::common::v1::any_value::Value::IntValue(500))
        );
        assert_eq!(
            modern_attrs["flag"].value,
            Some(crate::proto::opentelemetry::proto::common::v1::any_value::Value::BoolValue(true))
        );
        assert_eq!(
            modern_attrs["ratio"].value,
            Some(
                crate::proto::opentelemetry::proto::common::v1::any_value::Value::DoubleValue(0.5)
            )
        );
        assert_eq!(
            modern_attrs["name"].value,
            Some(
                crate::proto::opentelemetry::proto::common::v1::any_value::Value::StringValue(
                    "x".into()
                )
            )
        );

        let crate::proto::opentelemetry::proto::common::v1::any_value::Value::StringValue(
            legacy_body,
        ) = body(&records[1]).value.as_ref().unwrap()
        else {
            panic!("legacy body is not a string");
        };
        assert_eq!(legacy_body, "plain message");
        assert_eq!(records[1].severity_number, 13);
        assert_eq!(records[1].severity_text, "WARN");
        // Legacy mode drops the typed attributes; only log.target is kept.
        assert_eq!(records[1].attributes.len(), 1);
        assert_eq!(records[1].attributes[0].key, "log.target");
    }
}
