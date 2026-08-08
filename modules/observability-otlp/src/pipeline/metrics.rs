//! Metric pipeline: per-series accumulation and periodic export.
//!
//! The [`MetricStore`] accumulates `MetricEvent`s into per-series aggregators
//! keyed by `(metric name, attributes)`, mirroring the SDK instrument +
//! attribute-set deduplication it replaces. A [`MetricReader`] background
//! task collects all series on a fixed interval (30 s, matching the SDK
//! `PeriodicReader`) and exports them as one `ExportMetricsServiceRequest`.
//!
//! Aggregators (parity with the SDK configuration in
//! `providers/cache.rs#build_metrics_provider`):
//!
//! - counters: monotonic running sum, cumulative from the first observation.
//! - up-down counters: non-monotonic running sum (i64/f64).
//! - gauges: the last recorded value.
//! - histograms: Base2 exponential buckets (`max_scale 20`, `max_size 160`),
//!   because the SDK view always forced that aggregation. Explicit-boundary
//!   histograms are a follow-up behind a `native_histograms false` directive
//!   (see `CUSTOM_EXPORTER_REWRITE.md` §5.6).
//!
//! Exemplars: a ring buffer of capacity 1 per series keeps the last
//! measurement that carried a trace context; its invalid-recorded IDs are
//! decoded from hex-ASCII to raw bytes and attached to the exported data
//! point.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Once, OnceLock};
use std::time::{Duration, SystemTime};

use ferron_observability::baggage::{
    extract_promoted_keys, BaggageKeyPromotion, DistinctValueTracker, SignalSet,
};
use ferron_observability::{MetricEvent, MetricType, MetricValue};
use parking_lot::Mutex;

use crate::convert::{
    any_string, build_resource, build_scope, decode_span_id, decode_trace_id, kv,
    metric_key_values, nanos,
};
use crate::proto::opentelemetry::proto::collector::metrics::v1::ExportMetricsServiceRequest;
use crate::proto::opentelemetry::proto::common::v1::{any_value, KeyValue};
use crate::proto::opentelemetry::proto::metrics::v1::{
    exemplar, exponential_histogram_data_point::Buckets, number_data_point, AggregationTemporality,
    ExponentialHistogram, ExponentialHistogramDataPoint, Gauge, Histogram, HistogramDataPoint,
    Metric, NumberDataPoint, ResourceMetrics, ScopeMetrics, Sum,
};
use crate::proto::opentelemetry::proto::resource::v1::Resource;
use crate::transport::client::{ExportResult, OtlpTransport};

/// The instrumentation scope all metrics are reported under (parity with
/// `meter("ferron")` in the SDK path).
const METRIC_SCOPE: &str = "ferron";

/// Maximum scale of the Base2 exponential histogram (parity with the SDK
/// view: `max_scale 20`).
const EXPO_MAX_SCALE: i8 = 20;
/// Minimum scale before a measurement is dropped (parity with the SDK's
/// `EXPO_MIN_SCALE`).
const EXPO_MIN_SCALE: i8 = -10;
/// Maximum number of buckets in the exponential histogram (parity with the
/// SDK view: `max_size 160`).
const EXPO_MAX_SIZE: i32 = 160;

/// A stored exemplar: one sample measurement that carried a trace context.
#[derive(Clone)]
struct StoredExemplar {
    trace_id: Vec<u8>,
    span_id: Vec<u8>,
    time: u64,
    value: Scalar,
}

/// A numeric value accumulated by a series. The variant is fixed by the
/// instrument type of the first observation.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Scalar {
    Double(f64),
    Int(i64),
}

impl Scalar {
    fn of(value: MetricValue) -> Self {
        match value {
            MetricValue::F64(v) => Scalar::Double(v),
            MetricValue::I64(v) => Scalar::Int(v),
            MetricValue::U64(v) => Scalar::Int(v as i64),
            _ => Scalar::Int(0),
        }
    }
}

/// An operation to apply to a series aggregate, derived from a metric event.
enum Op {
    AddDouble { v: f64, monotonic: bool },
    AddInt { v: i64, monotonic: bool },
    AddUint(u64),
    SetDouble(f64),
    SetInt(i64),
    SetUint(u64),
    RecordHistogram(f64),
    None,
}

/// The aggregate of one metric series.
#[derive(Debug)]
enum Aggregate {
    Sum { value: Scalar, monotonic: bool },
    Gauge { value: Scalar },
    Histogram(HistogramAgg),
}

/// The histogram layout selected for a series: the base-2 exponential
/// (native) layout, or explicit bucket boundaries.
#[derive(Debug)]
enum HistogramAgg {
    Expo(ExpoHistogram),
    Explicit(ExplicitHistogram),
}

impl HistogramAgg {
    fn record(&mut self, value: f64) {
        match self {
            HistogramAgg::Expo(histogram) => histogram.record(value),
            HistogramAgg::Explicit(histogram) => histogram.record(value),
        }
    }
}

impl Aggregate {
    /// Resolve the aggregate variant for a metric's instrument type and first
    /// value. Combinations the SDK never supported return `None` (the event
    /// is dropped, matching the old `emit_metric` fall-through).
    fn for_event(ty: &MetricType, value: MetricValue, native_histograms: bool) -> Option<Self> {
        match (ty, value) {
            (MetricType::Counter, MetricValue::F64(_)) => Some(Aggregate::Sum {
                value: Scalar::Double(0.0),
                monotonic: true,
            }),
            (MetricType::Counter, MetricValue::U64(_)) => Some(Aggregate::Sum {
                value: Scalar::Int(0),
                monotonic: true,
            }),
            (MetricType::UpDownCounter, MetricValue::F64(_)) => Some(Aggregate::Sum {
                value: Scalar::Double(0.0),
                monotonic: false,
            }),
            (MetricType::UpDownCounter, MetricValue::I64(_)) => Some(Aggregate::Sum {
                value: Scalar::Int(0),
                monotonic: false,
            }),
            (MetricType::Gauge, MetricValue::F64(_)) => Some(Aggregate::Gauge {
                value: Scalar::Double(0.0),
            }),
            (MetricType::Gauge, MetricValue::I64(_)) | (MetricType::Gauge, MetricValue::U64(_)) => {
                Some(Aggregate::Gauge {
                    value: Scalar::Int(0),
                })
            }
            (MetricType::Histogram(_), MetricValue::F64(_))
            | (MetricType::Histogram(_), MetricValue::U64(_)) => {
                let aggregate = if native_histograms {
                    HistogramAgg::Expo(ExpoHistogram::new())
                } else {
                    HistogramAgg::Explicit(ExplicitHistogram::new())
                };
                Some(Aggregate::Histogram(aggregate))
            }
            _ => None,
        }
    }
}

/// The operation for one recorded value, mirroring the old `emit_metric`
/// match arms exactly (including the instrument/value arms it never had).
fn op_for(aggregate: &Aggregate, ty: &MetricType, value: MetricValue) -> Op {
    match (ty, aggregate, value) {
        (MetricType::Counter, Aggregate::Sum { .. }, MetricValue::F64(v)) => {
            Op::AddDouble { v, monotonic: true }
        }
        (MetricType::Counter, Aggregate::Sum { .. }, MetricValue::U64(v)) => Op::AddUint(v),
        (MetricType::UpDownCounter, Aggregate::Sum { .. }, MetricValue::F64(v)) => Op::AddDouble {
            v,
            monotonic: false,
        },
        (MetricType::UpDownCounter, Aggregate::Sum { .. }, MetricValue::I64(v)) => Op::AddInt {
            v,
            monotonic: false,
        },
        (MetricType::Gauge, Aggregate::Gauge { .. }, MetricValue::F64(v)) => Op::SetDouble(v),
        (MetricType::Gauge, Aggregate::Gauge { .. }, MetricValue::I64(v)) => Op::SetInt(v),
        (MetricType::Gauge, Aggregate::Gauge { .. }, MetricValue::U64(v)) => Op::SetUint(v),
        (MetricType::Histogram(_), Aggregate::Histogram(_), MetricValue::F64(v)) => {
            Op::RecordHistogram(v)
        }
        (MetricType::Histogram(_), Aggregate::Histogram(_), MetricValue::U64(v)) => {
            Op::RecordHistogram(v as f64)
        }
        _ => Op::None,
    }
}

/// Apply an operation to a series aggregate. Returns `false` (no-op) when the
/// measured value type does not match the aggregate's numeric kind, or when a
/// monotonic counter receives a negative delta.
fn apply(aggregate: &mut Aggregate, op: Op) -> bool {
    match (aggregate, op) {
        (Aggregate::Sum { value, .. }, Op::AddDouble { v, monotonic }) => {
            let Scalar::Double(d) = value else {
                return false;
            };
            if monotonic && v < 0.0 {
                return false;
            }
            *d += v;
        }
        (Aggregate::Sum { value, .. }, Op::AddInt { v, monotonic }) => {
            let Scalar::Int(i) = value else {
                return false;
            };
            if monotonic && v < 0 {
                return false;
            }
            *i = i.wrapping_add(v);
        }
        (Aggregate::Sum { value, .. }, Op::AddUint(v)) => {
            let Scalar::Int(i) = value else {
                return false;
            };
            *i = i.wrapping_add(v as i64);
        }
        (Aggregate::Gauge { value }, Op::SetDouble(v)) => *value = Scalar::Double(v),
        (Aggregate::Gauge { value }, Op::SetInt(v)) => *value = Scalar::Int(v),
        (Aggregate::Gauge { value }, Op::SetUint(v)) => *value = Scalar::Int(v as i64),
        (Aggregate::Histogram(histogram), Op::RecordHistogram(v)) => histogram.record(v),
        _ => return false,
    }
    true
}

/// One metric series: a named aggregate over one attribute set.
struct Series {
    name: String,
    description: String,
    unit: String,
    attributes: Vec<KeyValue>,
    aggregate: Aggregate,
    /// UNIX epoch nanoseconds of the first recorded observation (cumulative
    /// temporality: this stays fixed for the life of the series).
    start_time: u64,
    /// UNIX epoch nanoseconds of the most recent observation.
    last_time: u64,
    /// The last exemplar sample (ring capacity 1).
    exemplar: Option<StoredExemplar>,
}

impl Series {
    /// Record one measurement, updating time bounds and, when `capture` is
    /// set, the exemplar ring.
    fn record(&mut self, event: &MetricEvent, now: u64, capture: bool) {
        let op = op_for(&self.aggregate, &event.ty, event.value);
        if !apply(&mut self.aggregate, op) {
            return;
        }
        if self.start_time == 0 {
            self.start_time = now;
        }
        self.last_time = now;
        if capture {
            if let Some(ctx) = &event.trace_context {
                if let (Some(trace_id), Some(span_id)) =
                    (decode_trace_id(&ctx.trace_id), decode_span_id(&ctx.span_id))
                {
                    self.exemplar = Some(StoredExemplar {
                        trace_id,
                        span_id,
                        time: now,
                        value: Scalar::of(event.value),
                    });
                }
            }
        }
    }

    /// Export this series as one data point for the OTLP `Metric` grouping.
    fn export(&self) -> (String, String, String, MetricKind, Point) {
        let attributes = self.attributes.clone();
        let exemplars = self.exemplar.iter().cloned().map(exemplar_proto).collect();
        match &self.aggregate {
            Aggregate::Sum { value, monotonic } => (
                self.name.clone(),
                self.description.clone(),
                self.unit.clone(),
                MetricKind::Sum {
                    monotonic: *monotonic,
                },
                Point::Number(NumberDataPoint {
                    attributes,
                    start_time_unix_nano: self.start_time,
                    time_unix_nano: self.last_time,
                    exemplars,
                    flags: 0,
                    value: Some(number_value(*value)),
                }),
            ),
            Aggregate::Gauge { value } => (
                self.name.clone(),
                self.description.clone(),
                self.unit.clone(),
                MetricKind::Gauge,
                Point::Number(NumberDataPoint {
                    attributes,
                    start_time_unix_nano: self.start_time,
                    time_unix_nano: self.last_time,
                    exemplars: Vec::new(),
                    flags: 0,
                    value: Some(number_value(*value)),
                }),
            ),
            Aggregate::Histogram(histogram) => match histogram {
                HistogramAgg::Expo(histogram) => {
                    let point =
                        histogram.to_proto(attributes, self.start_time, self.last_time, exemplars);
                    (
                        self.name.clone(),
                        self.description.clone(),
                        self.unit.clone(),
                        MetricKind::ExponentialHistogram,
                        Point::Exponential(point),
                    )
                }
                HistogramAgg::Explicit(histogram) => {
                    let point =
                        histogram.to_proto(attributes, self.start_time, self.last_time, exemplars);
                    (
                        self.name.clone(),
                        self.description.clone(),
                        self.unit.clone(),
                        MetricKind::Histogram,
                        Point::Explicit(point),
                    )
                }
            },
        }
    }
}

fn number_value(
    value: Scalar,
) -> crate::proto::opentelemetry::proto::metrics::v1::number_data_point::Value {
    match value {
        Scalar::Double(v) => number_data_point::Value::AsDouble(v),
        Scalar::Int(v) => number_data_point::Value::AsInt(v),
    }
}

fn exemplar_proto(
    exemplar: StoredExemplar,
) -> crate::proto::opentelemetry::proto::metrics::v1::Exemplar {
    crate::proto::opentelemetry::proto::metrics::v1::Exemplar {
        filtered_attributes: Vec::new(),
        time_unix_nano: exemplar.time,
        span_id: exemplar.span_id,
        trace_id: exemplar.trace_id,
        value: Some(match exemplar.value {
            Scalar::Double(v) => exemplar::Value::AsDouble(v),
            Scalar::Int(v) => exemplar::Value::AsInt(v),
        }),
    }
}

/// The aggregation kind of a metric; distinguishes groups that cannot share
/// one `Metric` message.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum MetricKind {
    Gauge,
    Sum { monotonic: bool },
    ExponentialHistogram,
    Histogram,
}

/// The serialized form of one data point.
enum Point {
    Number(NumberDataPoint),
    Exponential(ExponentialHistogramDataPoint),
    Explicit(HistogramDataPoint),
}

/// An in-progress group of points that share a metric name and representation.
struct MetricGroup {
    name: String,
    description: String,
    unit: String,
    kind: MetricKind,
    points: Vec<Point>,
}

impl MetricGroup {
    fn finish(self) -> Metric {
        let data = match self.kind {
            MetricKind::Gauge => {
                crate::proto::opentelemetry::proto::metrics::v1::metric::Data::Gauge(Gauge {
                    data_points: self.points.into_iter().map(point_number).collect(),
                })
            }
            MetricKind::Sum { monotonic } => {
                crate::proto::opentelemetry::proto::metrics::v1::metric::Data::Sum(Sum {
                    data_points: self.points.into_iter().map(point_number).collect(),
                    aggregation_temporality: AggregationTemporality::Cumulative as i32,
                    is_monotonic: monotonic,
                })
            }
            MetricKind::ExponentialHistogram => {
                crate::proto::opentelemetry::proto::metrics::v1::metric::Data::ExponentialHistogram(
                    ExponentialHistogram {
                        data_points: self.points.into_iter().map(point_exponential).collect(),
                        aggregation_temporality: AggregationTemporality::Cumulative as i32,
                    },
                )
            }
            MetricKind::Histogram => {
                crate::proto::opentelemetry::proto::metrics::v1::metric::Data::Histogram(
                    Histogram {
                        data_points: self.points.into_iter().map(point_explicit).collect(),
                        aggregation_temporality: AggregationTemporality::Cumulative as i32,
                    },
                )
            }
        };
        Metric {
            name: self.name,
            description: self.description,
            unit: self.unit,
            metadata: Vec::new(),
            data: Some(data),
        }
    }
}

fn point_number(point: Point) -> NumberDataPoint {
    match point {
        Point::Number(point) => point,
        Point::Exponential(_) | Point::Explicit(_) => {
            unreachable!("point kind does not match metric group")
        }
    }
}

fn point_exponential(point: Point) -> ExponentialHistogramDataPoint {
    match point {
        Point::Exponential(point) => point,
        Point::Number(_) | Point::Explicit(_) => {
            unreachable!("point kind does not match metric group")
        }
    }
}

fn point_explicit(point: Point) -> HistogramDataPoint {
    match point {
        Point::Explicit(point) => point,
        Point::Number(_) | Point::Exponential(_) => {
            unreachable!("point kind does not match metric group")
        }
    }
}

/// The per-series accumulator registry, shared by the event loop (recorder)
/// and the reader task (collector).
#[derive(Clone)]
pub(crate) struct MetricStore {
    inner: Arc<MetricStoreInner>,
}

struct MetricStoreInner {
    series: Mutex<HashMap<String, Series>>,
    dropped: AtomicU64,
    /// Whether exemplar samples are captured (`exemplars` config directive).
    capture_exemplars: bool,
    /// Whether histogram series use the exponential (native) layout
    /// (`native_histograms` config directive).
    native_histograms: bool,
}

impl MetricStore {
    fn new(capture_exemplars: bool, native_histograms: bool) -> Self {
        Self {
            inner: Arc::new(MetricStoreInner {
                series: Mutex::new(HashMap::new()),
                dropped: AtomicU64::new(0),
                capture_exemplars,
                native_histograms,
            }),
        }
    }

    /// Record one metric event into its series, creating the series on first
    /// observation.
    pub(crate) fn record(
        &self,
        event: &MetricEvent,
        promotions: &[BaggageKeyPromotion],
        tracker: &mut DistinctValueTracker,
    ) {
        let aggregate =
            match Aggregate::for_event(&event.ty, event.value, self.inner.native_histograms) {
                Some(aggregate) => aggregate,
                None => return,
            };
        let attributes = attributes_for(event, promotions, tracker);
        let key = series_key(event.name, &attributes);
        let now = nanos(SystemTime::now());
        let mut series = self.inner.series.lock();
        let entry = series.entry(key).or_insert_with(|| Series {
            name: event.name.to_string(),
            description: event.description.unwrap_or_default().to_string(),
            unit: event.unit.unwrap_or_default().to_string(),
            attributes,
            aggregate,
            start_time: 0,
            last_time: 0,
            exemplar: None,
        });
        entry.record(event, now, self.inner.capture_exemplars);
    }

    /// Collect all series into per-name metric groups. Returns `None` when
    /// there is nothing to export yet, so the reader does not emit empty
    /// envelopes. `points` counts the metric data points collected.
    fn collect(&self) -> Option<(Vec<Metric>, usize)> {
        let mut group_index: HashMap<(String, MetricKind), usize> = HashMap::new();
        let mut groups: Vec<MetricGroup> = Vec::new();
        let mut points = 0;

        let series = self.inner.series.lock();
        if series.is_empty() {
            return None;
        }
        for metric in series.values() {
            let (name, description, unit, kind, point) = metric.export();
            let key = (name.clone(), kind);
            let index = match group_index.get(&key) {
                Some(&index) => index,
                None => {
                    group_index.insert(key.clone(), groups.len());
                    groups.push(MetricGroup {
                        name,
                        description,
                        unit,
                        kind,
                        points: Vec::new(),
                    });
                    groups.len() - 1
                }
            };
            groups[index].points.push(point);
            points += 1;
        }

        Some((
            groups.into_iter().map(MetricGroup::finish).collect(),
            points,
        ))
    }

    /// Total number of data points dropped because an export failed.
    #[cfg(test)]
    fn dropped(&self) -> u64 {
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

static DROPPED_METRICS: Once = Once::new();

fn warn_once() {
    DROPPED_METRICS.call_once(|| {
        ferron_core::log_warn!(
            "OTLP metric data points dropped (`otlp` observability backend). \
        This may be caused by a failing OTLP receiver."
        );
    });
}

/// Build the attribute set for a metric event: typed event attributes plus
/// promoted baggage keys, deduplicated by the series key this feeds into.
fn attributes_for(
    event: &MetricEvent,
    promotions: &[BaggageKeyPromotion],
    tracker: &mut DistinctValueTracker,
) -> Vec<KeyValue> {
    let mut attributes = metric_key_values(&event.attributes);
    if let Some(baggage_str) = event
        .trace_context
        .as_ref()
        .and_then(|ctx| ctx.baggage.as_deref())
    {
        let extracted = extract_promoted_keys(baggage_str, promotions, SignalSet::METRICS);
        for attribute in extracted {
            let value = tracker.canonicalize(
                &attribute.attribute_name,
                &attribute.value,
                promotions
                    .iter()
                    .find(|promotion| {
                        promotion.effective_attribute_name() == attribute.attribute_name
                    })
                    .and_then(|promotion| promotion.max_distinct),
            );
            attributes.push(kv(attribute.attribute_name, any_string(value)));
        }
    }
    attributes
}

/// Deterministic string key identifying one series: metric name plus a
/// canonical rendering of its attributes.
fn series_key(name: &str, attributes: &[KeyValue]) -> String {
    let mut key = String::with_capacity(64 + attributes.len() * 16);
    key.push_str(name);
    for attribute in attributes {
        key.push('|');
        key.push_str(&attribute.key);
        key.push('=');
        if let Some(value) = &attribute.value {
            if let Some(value) = &value.value {
                match value {
                    any_value::Value::StringValue(v) => key.push_str(v),
                    any_value::Value::IntValue(v) => key.push_str(&v.to_string()),
                    any_value::Value::DoubleValue(v) => key.push_str(&v.to_string()),
                    any_value::Value::BoolValue(v) => {
                        key.push_str(if *v { "true" } else { "false" })
                    }
                    any_value::Value::ArrayValue(v) => key.push_str(&v.values.len().to_string()),
                    any_value::Value::KvlistValue(_) => {}
                    any_value::Value::BytesValue(v) => key.push_str(&v.len().to_string()),
                    _ => {}
                }
            }
        }
    }
    key
}

/// Export one batch of metrics. Implemented by the real transport and by
/// mocks in tests.
pub(crate) trait MetricExporter: Send + Sync {
    /// Export one collection round.
    fn export_metrics<'a>(
        &'a self,
        request: &'a ExportMetricsServiceRequest,
    ) -> ExportMetricsFuture<'a>;
}

/// A pending export of one metric collection.
pub(crate) type ExportMetricsFuture<'a> = Pin<Box<dyn Future<Output = ExportResult> + Send + 'a>>;

impl MetricExporter for OtlpTransport {
    fn export_metrics<'a>(
        &'a self,
        request: &'a ExportMetricsServiceRequest,
    ) -> ExportMetricsFuture<'a> {
        Box::pin(OtlpTransport::export_metrics(self, request))
    }
}

/// The reader task state: collects all series and exports them on the
/// configured interval, draining on shutdown.
struct MetricReader {
    store: MetricStore,
    exporter: Arc<dyn MetricExporter>,
    resource: Resource,
    interval: Duration,
    export_timeout: Duration,
}

impl MetricReader {
    /// Collect and export all series. Exports nothing when no series exist.
    async fn flush(&self) {
        let Some((metrics, points)) = self.store.collect() else {
            return;
        };
        let request = ExportMetricsServiceRequest {
            resource_metrics: vec![ResourceMetrics {
                resource: Some(self.resource.clone()),
                scope_metrics: vec![ScopeMetrics {
                    scope: Some(build_scope(METRIC_SCOPE)),
                    metrics,
                    schema_url: String::new(),
                }],
                schema_url: String::new(),
            }],
        };
        let result =
            tokio::time::timeout(self.export_timeout, self.exporter.export_metrics(&request)).await;
        match result {
            Ok(ExportResult::Success | ExportResult::PartialSuccess { .. }) => {}
            Ok(ExportResult::Failure { .. }) | Err(_) => {
                self.store.record_dropped(points as u64);
            }
        }
    }

    /// Run until cancellation, then perform a final flush before returning.
    async fn run(self, cancel: tokio_util::sync::CancellationToken) {
        let mut interval = tokio::time::interval(self.interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = interval.tick() => self.flush().await,
            }
        }
        self.flush().await;
    }
}

/// Handle to a spawned metric reader: the shared store (for the event loop
/// to record into) and a completion signal for shutdown.
pub(crate) struct MetricPipeline {
    pub(crate) store: MetricStore,
    done: tokio::sync::oneshot::Receiver<()>,
}

impl MetricPipeline {
    pub(crate) fn spawn_with_config(
        exporter: Arc<dyn MetricExporter>,
        service_name: String,
        cancel: tokio_util::sync::CancellationToken,
        interval: Duration,
        export_timeout: Duration,
        capture_exemplars: bool,
        native_histograms: bool,
    ) -> Self {
        let store = MetricStore::new(capture_exemplars, native_histograms);
        let (done_tx, done_rx) = tokio::sync::oneshot::channel();
        let reader = MetricReader {
            store: store.clone(),
            exporter,
            resource: build_resource(service_name),
            interval,
            export_timeout,
        };
        tokio::spawn(async move {
            reader.run(cancel).await;
            let _ = done_tx.send(());
        });
        Self {
            store,
            done: done_rx,
        }
    }

    /// Wait for the reader task to finish its final collection.
    pub(crate) async fn wait_done(self) {
        let _ = self.done.await;
    }
}

/// The upper bounds of the buckets; the final bucket has no upper bound.
const EXPLICIT_BOUNDS: &[f64] = &[
    0.0, 5.0, 10.0, 25.0, 50.0, 75.0, 100.0, 250.0, 500.0, 750.0, 1000.0, 2500.0, 5000.0, 7500.0,
    10000.0,
];

/// A histogram aggregated into fixed, explicit buckets.
#[derive(Debug)]
pub(crate) struct ExplicitHistogram {
    count: u64,
    min: f64,
    max: f64,
    sum: f64,
    bucket_counts: Vec<u64>,
}

impl ExplicitHistogram {
    fn new() -> Self {
        Self {
            count: 0,
            min: f64::MAX,
            max: f64::MIN,
            sum: 0.0,
            bucket_counts: vec![0; EXPLICIT_BOUNDS.len() + 1],
        }
    }

    /// Record one measurement into the bucket that holds it.
    fn record(&mut self, value: f64) {
        self.count += 1;
        if value < self.min {
            self.min = value;
        }
        if value > self.max {
            self.max = value;
        }
        self.sum += value;
        let index = EXPLICIT_BOUNDS.partition_point(|bound| value > *bound);
        self.bucket_counts[index] += 1;
    }

    /// Export the histogram as an OTLP data point.
    fn to_proto(
        &self,
        attributes: Vec<KeyValue>,
        start_time_unix_nano: u64,
        time_unix_nano: u64,
        exemplars: Vec<crate::proto::opentelemetry::proto::metrics::v1::Exemplar>,
    ) -> HistogramDataPoint {
        HistogramDataPoint {
            attributes,
            start_time_unix_nano,
            time_unix_nano,
            count: self.count,
            sum: Some(self.sum),
            min: Some(self.min),
            max: Some(self.max),
            explicit_bounds: EXPLICIT_BOUNDS.to_vec(),
            bucket_counts: self.bucket_counts.clone(),
            flags: 0,
            exemplars,
        }
    }
}

/// A measurement that cannot fit even at the minimum scale is silently
/// dropped (parity with the SDK, which logs a debug message instead).
#[derive(Debug)]
pub(crate) struct ExpoHistogram {
    max_size: i32,
    count: u64,
    min: f64,
    max: f64,
    sum: f64,
    scale: i8,
    positive: ExpoBuckets,
    negative: ExpoBuckets,
    zero_count: u64,
}

impl ExpoHistogram {
    fn new() -> Self {
        Self {
            max_size: EXPO_MAX_SIZE,
            count: 0,
            min: f64::MAX,
            max: f64::MIN,
            sum: 0.0,
            scale: EXPO_MAX_SCALE,
            positive: ExpoBuckets::default(),
            negative: ExpoBuckets::default(),
            zero_count: 0,
        }
    }

    /// Rescale to a smaller scale (fewer buckets; `delta` bucket rows are
    /// merged). Used when a range of bins no longer fits and to honor the
    /// "downscale" semantic of cumulative histograms.
    fn downscale(&mut self, delta: u32) {
        if delta == 0 {
            return;
        }
        self.scale -= delta as i8;
        self.positive.downscale(delta);
        self.negative.downscale(delta);
    }

    /// Record one measurement into the histogram, resizing the buckets if
    /// needed.
    fn record(&mut self, value: f64) {
        self.count += 1;
        if value < self.min {
            self.min = value;
        }
        if value > self.max {
            self.max = value;
        }
        self.sum += value;

        let abs = value.abs();
        if abs == 0.0 {
            self.zero_count += 1;
            return;
        }

        let value_negative = value < 0.0;
        let mut bin = self.get_bin(abs);

        let bucket = if value_negative {
            &self.negative
        } else {
            &self.positive
        };
        let delta = scale_delta(
            self.max_size,
            bin,
            bucket.offset,
            bucket.counts.len() as i32,
        );
        if delta > 0 {
            if (self.scale - delta as i8) < EXPO_MIN_SCALE {
                // The measurement cannot fit even at the minimum scale; drop it.
                self.count -= 1;
                return;
            }
            self.downscale(delta);
            bin = get_bin(abs, self.scale);
        }

        if value_negative {
            self.negative.record(bin);
        } else {
            self.positive.record(bin);
        }
    }

    /// The index of the bucket `value` belongs to at a given scale.
    fn get_bin(&self, value: f64) -> i32 {
        get_bin(value, self.scale)
    }

    /// Export the histogram as an OTLP data point.
    fn to_proto(
        &self,
        attributes: Vec<KeyValue>,
        start_time_unix_nano: u64,
        time_unix_nano: u64,
        exemplars: Vec<crate::proto::opentelemetry::proto::metrics::v1::Exemplar>,
    ) -> ExponentialHistogramDataPoint {
        ExponentialHistogramDataPoint {
            attributes,
            start_time_unix_nano,
            time_unix_nano,
            count: self.count,
            sum: Some(self.sum),
            scale: self.scale as i32,
            zero_count: self.zero_count,
            positive: Some(self.positive.to_proto()),
            negative: Some(self.negative.to_proto()),
            flags: 0,
            exemplars,
            min: Some(self.min),
            max: Some(self.max),
            zero_threshold: 0.0,
        }
    }
}

/// The bucket index that holds `value` at `scale`, following the OTel
/// exponential histogram mapping formula.
fn get_bin(value: f64, scale: i8) -> i32 {
    debug_assert!(value >= 0.0 && value.is_finite(), "invalid histogram value");
    let (frac, exp) = frexp(value);
    if scale <= 0 {
        // With a negative scale, `frac` is always one power of two higher
        // than desired.
        let correction = if frac == 0.5 { 2 } else { 1 };
        return (exp - correction) >> -scale;
    }
    (exp << scale) + (frac.ln() * scale_factors()[scale as usize]) as i32 - 1
}

/// The number of scale reductions needed to fit `bin` within `[start_bin,
/// start_bin + length)` buckets of size `max_size`. Returns 0 when no
/// reduction is needed.
fn scale_delta(max_size: i32, bin: i32, start_bin: i32, length: i32) -> u32 {
    if length == 0 {
        return 0;
    }
    let mut low = start_bin;
    let mut high = bin;
    if start_bin >= bin {
        low = bin;
        high = start_bin + length - 1;
    }
    let mut count = 0u32;
    while high - low >= max_size {
        low >>= 1;
        high >>= 1;
        count += 1;
        if count > (EXPO_MAX_SCALE - EXPO_MIN_SCALE) as u32 {
            return count;
        }
    }
    count
}

static SCALE_FACTORS: OnceLock<[f64; 21]> = OnceLock::new();

/// Precomputed `LOG2_E * 2^scale` factors used by the bin formula.
fn scale_factors() -> &'static [f64; 21] {
    SCALE_FACTORS
        .get_or_init(|| std::array::from_fn(|i| std::f64::consts::LOG2_E * 2f64.powi(i as i32)))
}

/// Break a positive float into a normalized fraction and base-2 exponent
/// (libc `frexp`, reimplemented because Rust removed it from std).
fn frexp(value: f64) -> (f64, i32) {
    let mut bits = value.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32;

    if exponent == 0 {
        if value != 0.0 {
            let two_pow_64 = f64::from_bits(0x43f0_0000_0000_0000);
            let (frac, exp) = frexp(value * two_pow_64);
            return (frac, exp - 64);
        }
        // value is ±0.0; return the zero representation as-is.
        return (value, 0);
    }
    if exponent == 0x7ff {
        // NaN / infinity; clamp the fraction to 1.0 (cannot hold any bucket).
        return (1.0, 0);
    }

    let exponent = exponent - 0x3fe;
    bits &= 0x800f_ffff_ffff_ffff;
    bits |= 0x3fe0_0000_0000_0000;
    (f64::from_bits(bits), exponent)
}

/// A set of buckets of an exponential histogram.
#[derive(Debug, Default)]
struct ExpoBuckets {
    /// Index of the first bucket in `counts`.
    offset: i32,
    /// Bucket counts, bucket `i` of the histogram lives at `offset + i`.
    counts: Vec<u64>,
}

impl ExpoBuckets {
    /// Increment the count for the given bin, expanding the counts if needed.
    fn record(&mut self, bin: i32) {
        if self.counts.is_empty() {
            self.counts = vec![1];
            self.offset = bin;
            return;
        }

        let end_bin = self.offset + self.counts.len() as i32 - 1;

        // Inside the current range.
        if bin >= self.offset && bin <= end_bin {
            self.counts[(bin - self.offset) as usize] += 1;
            return;
        }

        // Before the current start: prepend zero buckets.
        if bin < self.offset {
            let mut new_counts = vec![0; (end_bin - bin + 1) as usize];
            let shift = (self.offset - bin) as usize;
            new_counts[shift..].copy_from_slice(&self.counts);
            new_counts[0] = 1;
            self.counts = new_counts;
            self.offset = bin;
        } else if bin > end_bin {
            // After the current end: append zero buckets and set the count.
            if ((bin - self.offset) as usize) < self.counts.capacity() {
                self.counts.resize((bin - self.offset + 1) as usize, 0);
                self.counts[(bin - self.offset) as usize] = 1;
                return;
            }
            self.counts.extend(std::iter::repeat_n(
                0,
                (bin - self.offset) as usize - self.counts.len() + 1,
            ));
            self.counts[(bin - self.offset) as usize] = 1;
        }
    }

    /// Shrink the buckets by a factor of `2^delta`, summing the merged
    /// counts.
    fn downscale(&mut self, delta: u32) {
        if self.counts.len() <= 1 || delta < 1 {
            self.offset >>= delta;
            return;
        }
        let steps = 1 << delta;
        let mut offset = self.offset % steps;
        offset = (offset + steps) % steps;
        for index in 1..self.counts.len() {
            let merged = index + offset as usize;
            if merged.is_multiple_of(steps as usize) {
                self.counts[merged / steps as usize] = self.counts[index];
            } else {
                self.counts[merged / steps as usize] += self.counts[index];
            }
        }
        let last_index = (self.counts.len() as i32 - 1 + offset) / steps;
        self.counts = self.counts[..last_index as usize + 1].to_vec();
        self.offset >>= delta;
    }

    fn to_proto(&self) -> Buckets {
        Buckets {
            offset: self.offset,
            bucket_counts: self.counts.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
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
        let crate::proto::opentelemetry::proto::metrics::v1::metric::Data::ExponentialHistogram(
            data,
        ) = histogram.data.as_ref().unwrap()
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
        let crate::proto::opentelemetry::proto::metrics::v1::metric::Data::ExponentialHistogram(
            data,
        ) = metrics[0].data.as_ref().unwrap()
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

    #[test]
    fn bucket_indexes_follow_the_otlp_mapping() {
        // At scale 0 the buckets are unit-width powers of two: bucket -2 =
        // (0.25, 0.5], bucket -1 = (0.5, 1], bucket 0 = (1, 2], bucket 1 =
        // (2, 4].
        assert_eq!(get_bin(1.0, 0), -1);
        assert_eq!(get_bin(2.0, 0), 0);
        assert_eq!(get_bin(0.5, 0), -2);
        assert_eq!(get_bin(3.0, 0), 1);
        assert_eq!(get_bin(1.5, 0), 0);
        // At the maximum scale, 1 maps to bucket -1 and 2 to bucket 2^20 - 1.
        assert_eq!(get_bin(1.0, EXPO_MAX_SCALE), -1);
        assert_eq!(get_bin(2.0, EXPO_MAX_SCALE), (1 << 20) - 1);
        assert_eq!(get_bin(0.5, EXPO_MAX_SCALE), -1_048_577);
    }

    #[test]
    fn buckets_prepend_append_and_downscale() {
        // Append: bucket 0 = 1, append bucket 2 -> counts [1, 0, 1].
        let mut buckets = ExpoBuckets::default();
        buckets.record(0);
        buckets.record(2);
        assert_eq!(buckets.offset, 0);
        assert_eq!(buckets.counts, vec![1, 0, 1]);
        // Prepend: bucket -1 shifts everything right.
        buckets.record(-1);
        assert_eq!(buckets.offset, -1);
        assert_eq!(buckets.counts, vec![1, 1, 0, 1]);
        // Downscale merges adjacent bins into the half as many buckets.
        let mut buckets = ExpoBuckets::default();
        for bin in 0..4 {
            buckets.record(bin);
        }
        assert_eq!(buckets.counts, vec![1, 1, 1, 1]);
        buckets.downscale(1);
        assert_eq!(buckets.offset, 0);
        assert_eq!(buckets.counts, vec![2, 2]);
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
}
