use std::sync::Arc;
use std::time::Duration;

use ferron_core::admin::AdminMetrics;
use ferron_core::admin::ADMIN_METRICS;
use ferron_observability::{CompositeEventSink, Event, MetricEvent, MetricType, MetricValue};

/// Runs the background admin API metrics collection loop.
///
/// Collects metrics every 1 second and emits them through the composite event sink.
pub async fn collect_admin_metrics(
    event_sink: Arc<CompositeEventSink>,
    cancel_token: tokio_util::sync::CancellationToken,
) {
    loop {
        tokio::select! {
            _ = cancel_token.cancelled() => break,
            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
        }

        emit_metrics(&event_sink, &ADMIN_METRICS);
    }
}

fn emit_metrics(event_sink: &CompositeEventSink, metrics: &AdminMetrics) {
    event_sink.emit(Event::Metric(MetricEvent {
        name: "ferron.admin.uptime",
        attributes: vec![],
        ty: MetricType::Gauge,
        value: MetricValue::F64(metrics.start_time.elapsed().as_secs_f64()),
        unit: Some("s"),
        description: Some("Time since the server started."),
    }));

    event_sink.emit(Event::Metric(MetricEvent {
        name: "ferron.admin.connections_active",
        attributes: vec![],
        ty: MetricType::Gauge,
        value: MetricValue::U64(
            metrics
                .connections_active
                .load(std::sync::atomic::Ordering::Relaxed),
        ),
        unit: Some("1"),
        description: Some("Currently open TCP connections across all HTTP listeners."),
    }));

    event_sink.emit(Event::Metric(MetricEvent {
        name: "ferron.admin.requests_total",
        attributes: vec![],
        ty: MetricType::Gauge,
        value: MetricValue::U64(
            metrics
                .requests_total
                .load(std::sync::atomic::Ordering::Relaxed),
        ),
        unit: Some("1"),
        description: Some("Total HTTP requests served across all listeners."),
    }));

    event_sink.emit(Event::Metric(MetricEvent {
        name: "ferron.admin.reloads",
        attributes: vec![],
        ty: MetricType::Gauge,
        value: MetricValue::U64(metrics.reloads.load(std::sync::atomic::Ordering::Relaxed)),
        unit: Some("1"),
        description: Some("Number of configuration reloads performed."),
    }));

    event_sink.emit(Event::Metric(MetricEvent {
        name: "ferron.admin.observability_events_dropped",
        attributes: vec![],
        ty: MetricType::Gauge,
        value: MetricValue::U64(
            metrics
                .observability_events_dropped
                .load(std::sync::atomic::Ordering::Relaxed),
        ),
        unit: Some("1"),
        description: Some("Total number of observability events dropped due to backpressure."),
    }));

    event_sink.emit(Event::Metric(MetricEvent {
        name: "ferron.admin.observability_event_queue_len",
        attributes: vec![],
        ty: MetricType::Gauge,
        value: MetricValue::U64(
            metrics
                .observability_event_queue_len
                .load(std::sync::atomic::Ordering::Relaxed),
        ),
        unit: Some("1"),
        description: Some("Approximate current length of the observability event queue."),
    }));
}
