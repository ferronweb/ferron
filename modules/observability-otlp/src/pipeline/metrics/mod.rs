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

#[cfg(any(test, feature = "fuzz"))]
pub mod expo_histogram;
#[cfg(not(any(test, feature = "fuzz")))]
mod expo_histogram;
mod tests;

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Once};
use std::time::{Duration, SystemTime};

use dashmap::DashMap;
use ferron_observability::baggage::{
    extract_promoted_keys, BaggageKeyPromotion, DistinctValueTracker, SignalSet,
};
use ferron_observability::{MetricEvent, MetricType, MetricValue};

use self::expo_histogram::*;

use crate::convert::{
    any_string, build_resource, build_scope, decode_span_id, decode_trace_id, kv,
    metric_key_values, nanos,
};
use crate::proto::opentelemetry::proto::collector::metrics::v1::ExportMetricsServiceRequest;
use crate::proto::opentelemetry::proto::common::v1::{any_value, KeyValue};
use crate::proto::opentelemetry::proto::metrics::v1::{
    exemplar, number_data_point, AggregationTemporality, ExponentialHistogram,
    ExponentialHistogramDataPoint, Gauge, Histogram, HistogramDataPoint, Metric, NumberDataPoint,
    ResourceMetrics, ScopeMetrics, Sum,
};
use crate::proto::opentelemetry::proto::resource::v1::Resource;
use crate::transport::client::{ExportResult, OtlpTransport};

/// The instrumentation scope all metrics are reported under (parity with
/// `meter("ferron")` in the SDK path).
const METRIC_SCOPE: &str = "ferron";

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
            (MetricType::Histogram(buckets), MetricValue::F64(_))
            | (MetricType::Histogram(buckets), MetricValue::U64(_)) => {
                let aggregate = if native_histograms {
                    HistogramAgg::Expo(ExpoHistogram::new())
                } else {
                    if let Some(buckets) = buckets.to_owned() {
                        HistogramAgg::Explicit(ExplicitHistogram::with_buckets(buckets))
                    } else {
                        // Use implicit default buckets
                        HistogramAgg::Explicit(ExplicitHistogram::new())
                    }
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
    series: DashMap<String, Series>,
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
                series: DashMap::new(),
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
        let mut entry = self.inner.series.entry(key).or_insert_with(|| Series {
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

        let mut is_empty = true;

        for metric in self.inner.series.iter() {
            is_empty = false;
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

        if is_empty {
            return None;
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
