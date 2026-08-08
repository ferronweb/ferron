#![cfg(test)]

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use ferron_observability::{MetricAttributeValue, MetricEvent};
use tokio::sync::Mutex;

use super::*;

fn event(name: &'static str, ty: MetricType, value: MetricValue) -> MetricEvent {
    MetricEvent {
        name,
        attributes: Vec::new(),
        ty,
        value,
        unit: None,
        description: None,
        trace_context: None,
    }
}

fn mock_exporter() -> Arc<MockExporter> {
    Arc::new(MockExporter::default())
}

/// Mock exporter: records requests, optionally fails them.
#[derive(Default)]
struct MockExporter {
    requests: Mutex<Vec<ExportMetricsServiceRequest>>,
    fail: AtomicBool,
}

impl MockExporter {
    async fn request_count(&self) -> usize {
        self.requests.lock().await.len()
    }
}

impl MetricExporter for MockExporter {
    fn export_metrics<'a>(
        &'a self,
        request: &'a ExportMetricsServiceRequest,
    ) -> ExportMetricsFuture<'a> {
        Box::pin(async move {
            self.requests.lock().await.push(request.clone());
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

fn spawn_pipeline(
    exporter: Arc<MockExporter>,
    interval: Duration,
) -> (MetricPipeline, tokio_util::sync::CancellationToken) {
    spawn_pipeline_with(exporter, interval, true, true)
}

fn spawn_pipeline_with(
    exporter: Arc<MockExporter>,
    interval: Duration,
    capture_exemplars: bool,
    native_histograms: bool,
) -> (MetricPipeline, tokio_util::sync::CancellationToken) {
    let cancel = tokio_util::sync::CancellationToken::new();
    let pipeline = MetricPipeline::spawn_with_config(
        exporter,
        "test-service".into(),
        cancel.clone(),
        interval,
        Duration::from_secs(5),
        capture_exemplars,
        native_histograms,
    );
    (pipeline, cancel)
}

#[tokio::test]
async fn sums_and_gauges_export_cumulative_points() {
    let mock = mock_exporter();
    let (pipeline, cancel) = spawn_pipeline(mock.clone(), Duration::from_secs(3600));

    pipeline.store.record(
        &event("req.count", MetricType::Counter, MetricValue::U64(10)),
        &[],
        &mut DistinctValueTracker::new(),
    );
    pipeline.store.record(
        &event("req.count", MetricType::Counter, MetricValue::U64(5)),
        &[],
        &mut DistinctValueTracker::new(),
    );
    pipeline.store.record(
        &event("mem.bytes", MetricType::Gauge, MetricValue::I64(7)),
        &[],
        &mut DistinctValueTracker::new(),
    );

    cancel.cancel();
    pipeline.wait_done().await;

    let requests = mock.requests.lock().await;
    let metrics = &requests[0].resource_metrics[0].scope_metrics[0].metrics;
    let count = metrics
        .iter()
        .find(|metric| metric.name == "req.count")
        .unwrap();
    let crate::proto::opentelemetry::proto::metrics::v1::metric::Data::Sum(sum) =
        count.data.as_ref().unwrap()
    else {
        panic!("req.count is not a sum");
    };
    assert!(sum.is_monotonic);
    assert_eq!(
        sum.aggregation_temporality,
        AggregationTemporality::Cumulative as i32
    );
    assert_eq!(
        sum.data_points[0].value,
        Some(number_data_point::Value::AsInt(15))
    );

    let gauge = metrics
        .iter()
        .find(|metric| metric.name == "mem.bytes")
        .unwrap();
    let crate::proto::opentelemetry::proto::metrics::v1::metric::Data::Gauge(gauge_data) =
        gauge.data.as_ref().unwrap()
    else {
        panic!("mem.bytes is not a gauge");
    };
    assert_eq!(
        gauge_data.data_points[0].value,
        Some(number_data_point::Value::AsInt(7))
    );
}

#[tokio::test]
async fn monotonic_counter_drops_negative_deltas() {
    let mock = mock_exporter();
    let (pipeline, cancel) = spawn_pipeline(mock.clone(), Duration::from_secs(3600));

    pipeline.store.record(
        &event("calls", MetricType::Counter, MetricValue::F64(3.0)),
        &[],
        &mut DistinctValueTracker::new(),
    );
    pipeline.store.record(
        &event("calls", MetricType::Counter, MetricValue::F64(-2.0)),
        &[],
        &mut DistinctValueTracker::new(),
    );
    pipeline.store.record(
        &event("calls", MetricType::Counter, MetricValue::F64(1.5)),
        &[],
        &mut DistinctValueTracker::new(),
    );

    cancel.cancel();
    pipeline.wait_done().await;

    let requests = mock.requests.lock().await;
    let metrics = &requests[0].resource_metrics[0].scope_metrics[0].metrics;
    let crate::proto::opentelemetry::proto::metrics::v1::metric::Data::Sum(sum) =
        metrics[0].data.as_ref().unwrap()
    else {
        panic!("calls is not a sum");
    };
    assert_eq!(
        sum.data_points[0].value,
        Some(number_data_point::Value::AsDouble(4.5))
    );
}

#[tokio::test]
async fn rejected_first_sample_does_not_create_phantom_series() {
    let mock = mock_exporter();
    let (pipeline, cancel) = spawn_pipeline(mock.clone(), Duration::from_secs(3600));

    let mut tracker = DistinctValueTracker::new();
    // Every sample is rejected: negative deltas on a monotonic counter. A
    // rejected first observation must not create the series, or a zero-valued
    // point with no start time would be exported on every interval forever.
    for delta in [-1.0, -2.0, -3.0] {
        pipeline.store.record(
            &event("calls", MetricType::Counter, MetricValue::F64(delta)),
            &[],
            &mut tracker,
        );
    }

    cancel.cancel();
    pipeline.wait_done().await;

    let requests = mock.requests.lock().await;
    assert!(
        requests.is_empty(),
        "a series whose samples were all rejected must not be exported"
    );
}

#[tokio::test]
async fn start_time_is_first_observation_and_stamps_advance() {
    let mock = mock_exporter();
    let (pipeline, cancel) = spawn_pipeline(mock.clone(), Duration::from_secs(3600));

    pipeline.store.record(
        &event("t", MetricType::Counter, MetricValue::U64(1)),
        &[],
        &mut DistinctValueTracker::new(),
    );
    let (metrics, _) = pipeline.store.collect().unwrap();
    let (metrics2, _) = pipeline.store.collect().unwrap();

    let point = first_sum_point(&metrics, "t");
    let point2 = first_sum_point(&metrics2, "t");
    assert!(point.start_time_unix_nano > 0);
    // Cumulative: the start time is fixed from the first export.
    assert_eq!(point.start_time_unix_nano, point2.start_time_unix_nano);
    // The measurement timestamp is at or after the start, and does not go
    // backwards across exports.
    assert!(point2.time_unix_nano >= point.time_unix_nano);
    assert!(point.time_unix_nano >= point.start_time_unix_nano);

    cancel.cancel();
    pipeline.wait_done().await;
}

#[tokio::test]
async fn exponential_histogram_accumulates_magnitude_statistics() {
    let mock = mock_exporter();
    let (pipeline, cancel) = spawn_pipeline(mock.clone(), Duration::from_secs(3600));

    let values = [20u64, 200, 10, 1_000_000_000, 2, 5, 0, 0];
    let mut tracker = DistinctValueTracker::new();
    for value in values {
        pipeline.store.record(
            &event(
                "latency",
                MetricType::Histogram(None),
                MetricValue::U64(value),
            ),
            &[],
            &mut tracker,
        );
    }

    cancel.cancel();
    pipeline.wait_done().await;

    let requests = mock.requests.lock().await;
    let metrics = &requests[0].resource_metrics[0].scope_metrics[0].metrics;
    let histogram = metrics
        .iter()
        .find(|metric| metric.name == "latency")
        .unwrap();
    let crate::proto::opentelemetry::proto::metrics::v1::metric::Data::ExponentialHistogram(data) =
        histogram.data.as_ref().unwrap()
    else {
        panic!("latency is not an exponential histogram");
    };
    assert_eq!(
        data.aggregation_temporality,
        AggregationTemporality::Cumulative as i32
    );
    let point = &data.data_points[0];
    assert_eq!(point.count, 8);
    assert_eq!(point.zero_count, 2);
    assert_eq!(point.sum, Some(1_000_000_237.0));
    // min/max include the zero-valued samples (the SDK records them too).
    assert_eq!(point.min, Some(0.0));
    assert_eq!(point.max, Some(1_000_000_000.0));
    assert!(point.scale > 0 && point.scale <= EXPO_MAX_SCALE as i32);
    // count must equal the zero-count plus the union of the two bucket sets.
    let bucket_total = point
        .positive
        .as_ref()
        .unwrap()
        .bucket_counts
        .iter()
        .sum::<u64>()
        + point
            .negative
            .as_ref()
            .unwrap()
            .bucket_counts
            .iter()
            .sum::<u64>();
    assert_eq!(point.count, point.zero_count + bucket_total);
    // All values are positive here, so the negative side has no counts.
    assert_eq!(
        point
            .negative
            .as_ref()
            .unwrap()
            .bucket_counts
            .iter()
            .sum::<u64>(),
        0
    );
}

#[tokio::test]
async fn explicit_histogram_exports_fixed_buckets() {
    let mock = mock_exporter();
    let (pipeline, cancel) =
        spawn_pipeline_with(mock.clone(), Duration::from_secs(3600), true, false);

    let values = [1u64, 60, 0, 5000, 20_000];
    let mut tracker = DistinctValueTracker::new();
    for value in values {
        pipeline.store.record(
            &event(
                "latency",
                MetricType::Histogram(None),
                MetricValue::U64(value),
            ),
            &[],
            &mut tracker,
        );
    }

    cancel.cancel();
    pipeline.wait_done().await;

    let requests = mock.requests.lock().await;
    let metrics = &requests[0].resource_metrics[0].scope_metrics[0].metrics;
    let histogram = metrics
        .iter()
        .find(|metric| metric.name == "latency")
        .unwrap();
    let crate::proto::opentelemetry::proto::metrics::v1::metric::Data::Histogram(data) =
        histogram.data.as_ref().unwrap()
    else {
        panic!("latency is not an explicit histogram");
    };
    assert_eq!(
        data.aggregation_temporality,
        AggregationTemporality::Cumulative as i32
    );
    let point = &data.data_points[0];
    assert_eq!(point.count, 5);
    assert_eq!(point.sum, Some(25_061.0));
    assert_eq!(point.min, Some(0.0));
    assert_eq!(point.max, Some(20_000.0));
    // One bucket count per bound, plus the implicit +Inf bucket.
    assert_eq!(point.explicit_bounds.len() + 1, point.bucket_counts.len());
    assert_eq!(point.bucket_counts.iter().sum::<u64>(), 5);
    // 1 falls in the 0-5 bucket, 60 in the 50-75 bucket, 5000 in the
    // 2500-5000 bucket, and 20000 beyond the last bound.
    assert_eq!(point.bucket_counts[0], 1);
    assert_eq!(point.bucket_counts[1], 1);
    assert_eq!(point.bucket_counts[5], 1);
    assert_eq!(point.bucket_counts[12], 1);
    assert_eq!(point.bucket_counts[15], 1);
}

#[tokio::test]
async fn negative_and_positive_buckets_are_kept_separate() {
    let mock = mock_exporter();
    let (pipeline, cancel) = spawn_pipeline(mock.clone(), Duration::from_secs(3600));

    let values = [-100, -1, 1, 50];
    let mut tracker = DistinctValueTracker::new();
    for value in values {
        pipeline.store.record(
            &event(
                "signed",
                MetricType::Histogram(None),
                MetricValue::F64(f64::from(value)),
            ),
            &[],
            &mut tracker,
        );
    }
    cancel.cancel();
    pipeline.wait_done().await;

    let requests = mock.requests.lock().await;
    let metrics = &requests[0].resource_metrics[0].scope_metrics[0].metrics;
    let crate::proto::opentelemetry::proto::metrics::v1::metric::Data::ExponentialHistogram(data) =
        metrics[0].data.as_ref().unwrap()
    else {
        panic!("signed is not an exponential histogram");
    };
    let point = &data.data_points[0];
    let positive = point
        .positive
        .as_ref()
        .unwrap()
        .bucket_counts
        .iter()
        .sum::<u64>();
    let negative = point
        .negative
        .as_ref()
        .unwrap()
        .bucket_counts
        .iter()
        .sum::<u64>();
    assert_eq!(positive, 2);
    assert_eq!(negative, 2);
    assert_eq!(point.count, 4);
    assert_eq!(point.sum, Some(-50.0));
}

#[tokio::test]
async fn exemplar_ring_overwrites_and_attaches_last_sample() {
    let mock = mock_exporter();
    let (pipeline, cancel) = spawn_pipeline(mock.clone(), Duration::from_secs(3600));

    let ctx = |trace: &str, span: &str| ::ferron_observability::EventTraceContext {
        trace_id: trace.as_bytes().try_into().unwrap(),
        span_id: span.as_bytes().try_into().unwrap(),
        baggage: None,
        sampled: None,
    };
    let mut tracker = DistinctValueTracker::new();
    let mut first = event("hits", MetricType::Counter, MetricValue::U64(2));
    first.trace_context = Some(ctx("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "bbbbbbbbbbbbbbbb"));
    pipeline.store.record(&first, &[], &mut tracker);
    // Second sample overwrites the ring.
    let mut second = event("hits", MetricType::Counter, MetricValue::U64(3));
    second.trace_context = Some(ctx("cccccccccccccccccccccccccccccccc", "dddddddddddddddd"));
    pipeline.store.record(&second, &[], &mut tracker);

    cancel.cancel();
    pipeline.wait_done().await;

    let requests = mock.requests.lock().await;
    let metrics = &requests[0].resource_metrics[0].scope_metrics[0].metrics;
    let sum_point = first_sum_point(metrics, "hits");
    assert_eq!(sum_point.exemplars.len(), 1);
    let exemplar = &sum_point.exemplars[0];
    assert_eq!(
        hex::encode(&exemplar.trace_id),
        "cccccccccccccccccccccccccccccccc"
    );
    assert_eq!(hex::encode(&exemplar.span_id), "dddddddddddddddd");
    assert_eq!(exemplar.value, Some(exemplar::Value::AsInt(3)));
}

#[tokio::test]
async fn exemplars_disabled_do_not_attach_samples() {
    let mock = mock_exporter();
    let (pipeline, cancel) =
        spawn_pipeline_with(mock.clone(), Duration::from_secs(3600), false, true);

    let mut event = event("hits", MetricType::Counter, MetricValue::U64(2));
    event.trace_context = Some(::ferron_observability::EventTraceContext {
        trace_id: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            .as_bytes()
            .try_into()
            .unwrap(),
        span_id: "bbbbbbbbbbbbbbbb".as_bytes().try_into().unwrap(),
        baggage: None,
        sampled: None,
    });
    let mut tracker = DistinctValueTracker::new();
    pipeline.store.record(&event, &[], &mut tracker);

    cancel.cancel();
    pipeline.wait_done().await;

    let requests = mock.requests.lock().await;
    let metrics = &requests[0].resource_metrics[0].scope_metrics[0].metrics;
    let sum_point = first_sum_point(metrics, "hits");
    assert!(sum_point.exemplars.is_empty());
}

#[tokio::test]
async fn zero_trace_ids_do_not_produce_exemplars() {
    let mock = mock_exporter();
    let (pipeline, cancel) = spawn_pipeline(mock.clone(), Duration::from_secs(3600));

    let mut e = event("hits", MetricType::Counter, MetricValue::U64(2));
    e.trace_context = Some(ferron_observability::EventTraceContext {
        trace_id: [b'0'; 32],
        span_id: [b'0'; 16],
        baggage: None,
        sampled: None,
    });
    pipeline
        .store
        .record(&e, &[], &mut DistinctValueTracker::new());

    cancel.cancel();
    pipeline.wait_done().await;

    let requests = mock.requests.lock().await;
    let metrics = &requests[0].resource_metrics[0].scope_metrics[0].metrics;
    let point = first_sum_point(metrics, "hits");
    assert!(point.exemplars.is_empty());
}

#[tokio::test]
async fn sanitizes_string_attributes_on_the_wire() {
    let mock = mock_exporter();
    let (pipeline, cancel) = spawn_pipeline(mock.clone(), Duration::from_secs(3600));

    let mut e = event("g", MetricType::Gauge, MetricValue::F64(1.0));
    e.attributes = vec![
        (
            "user_agent",
            MetricAttributeValue::String("GET\r\nX".to_string()),
        ),
        ("long", MetricAttributeValue::String("A".repeat(200))),
    ];
    pipeline
        .store
        .record(&e, &[], &mut DistinctValueTracker::new());

    cancel.cancel();
    pipeline.wait_done().await;

    let requests = mock.requests.lock().await;
    let metrics = &requests[0].resource_metrics[0].scope_metrics[0].metrics;
    let crate::proto::opentelemetry::proto::metrics::v1::metric::Data::Gauge(gauge) =
        metrics[0].data.as_ref().unwrap()
    else {
        panic!("g is not a gauge");
    };
    let attrs: HashMap<_, _> = gauge.data_points[0]
        .attributes
        .iter()
        .map(|attribute| (attribute.key.as_str(), attribute.value.as_ref().unwrap()))
        .collect();
    assert_eq!(
        attrs["user_agent"].value,
        Some(any_value::Value::StringValue("GET??X".into()))
    );
    let crate::proto::opentelemetry::proto::common::v1::any_value::Value::StringValue(long) =
        attrs["long"].value.clone().unwrap()
    else {
        panic!("long is not a string");
    };
    assert!(long.starts_with("hash_"));
}

#[tokio::test]
async fn no_request_when_no_series_and_no_empty_flush() {
    let mock = mock_exporter();
    let (pipeline, cancel) = spawn_pipeline(mock.clone(), Duration::from_millis(20));

    // Wait well past the interval: nothing recorded, nothing exported.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(mock.request_count().await, 0);

    cancel.cancel();
    pipeline.wait_done().await;
    assert_eq!(mock.request_count().await, 0);
}

#[tokio::test]
async fn reader_flushes_on_interval_and_drains_on_shutdown() {
    let mock = mock_exporter();
    let (pipeline, cancel) = spawn_pipeline(mock.clone(), Duration::from_millis(50));

    pipeline.store.record(
        &event("a", MetricType::Counter, MetricValue::U64(1)),
        &[],
        &mut DistinctValueTracker::new(),
    );
    wait_until(|| async { mock.request_count().await == 1 }).await;

    pipeline.store.record(
        &event("b", MetricType::Gauge, MetricValue::F64(2.0)),
        &[],
        &mut DistinctValueTracker::new(),
    );
    cancel.cancel();
    pipeline.wait_done().await;

    // One for the interval flush, one for the shutdown flush.
    assert_eq!(mock.request_count().await, 2);
}

#[tokio::test]
async fn persistent_failure_counts_dropped_points() {
    let mock = mock_exporter();
    mock.fail.store(true, Ordering::Relaxed);
    let (pipeline, cancel) = spawn_pipeline(mock.clone(), Duration::from_millis(20));

    pipeline.store.record(
        &event("a", MetricType::Counter, MetricValue::U64(1)),
        &[],
        &mut DistinctValueTracker::new(),
    );
    pipeline.store.record(
        &event("b", MetricType::Gauge, MetricValue::F64(2.0)),
        &[],
        &mut DistinctValueTracker::new(),
    );

    wait_until(|| async { pipeline.store.dropped() == 2 }).await;
    cancel.cancel();
    pipeline.wait_done().await;
}

/// Extract the (only) data point of a sum metric.
fn first_sum_point(metrics: &[Metric], name: &str) -> NumberDataPoint {
    let metric = metrics.iter().find(|metric| metric.name == name).unwrap();
    let crate::proto::opentelemetry::proto::metrics::v1::metric::Data::Sum(sum) =
        metric.data.as_ref().unwrap()
    else {
        panic!("{name} is not a sum");
    };
    sum.data_points[0].clone()
}
