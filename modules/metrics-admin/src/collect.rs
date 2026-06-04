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
        unit: Some("{connection}"),
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
        unit: Some("{request}"),
        description: Some("Total HTTP requests served across all listeners."),
    }));

    event_sink.emit(Event::Metric(MetricEvent {
        name: "ferron.admin.reloads",
        attributes: vec![],
        ty: MetricType::Gauge,
        value: MetricValue::U64(metrics.reloads.load(std::sync::atomic::Ordering::Relaxed)),
        unit: Some("{reload}"),
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
        unit: Some("{event}"),
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
        unit: Some("{event}"),
        description: Some("Approximate current length of the observability event queue."),
    }));

    // --- Reload metrics ---
    {
        let reload_metrics = metrics.reload_metrics.read();

        event_sink.emit(Event::Metric(MetricEvent {
            name: "ferron.admin.reload.successful",
            attributes: vec![],
            ty: MetricType::Gauge,
            value: MetricValue::U64(if reload_metrics.last_reload_error.is_some() {
                0
            } else {
                1
            }),
            unit: Some("{enabled}"),
            description: Some("Whether the last configuration reload was successful."),
        }));

        event_sink.emit(Event::Metric(MetricEvent {
            name: "ferron.admin.reload.active_generation",
            attributes: vec![],
            ty: MetricType::Gauge,
            value: MetricValue::U64(reload_metrics.active_generation),
            unit: Some("{generation}"),
            description: Some("Active generation of the configuration being reloaded."),
        }));
    }

    // --- Runtime metrics ---
    {
        let runtime_metrics = metrics.runtime_metrics.read();

        event_sink.emit(Event::Metric(MetricEvent {
            name: "ferron.admin.runtime.primary_threads",
            attributes: vec![],
            ty: MetricType::Gauge,
            value: MetricValue::U64(runtime_metrics.primary_threads as u64),
            unit: Some("{thread}"),
            description: Some("Number of primary threads."),
        }));

        event_sink.emit(Event::Metric(MetricEvent {
            name: "ferron.admin.runtime.io_uring_supported",
            attributes: vec![],
            ty: MetricType::Gauge,
            value: MetricValue::U64(if runtime_metrics.io_uring_supported {
                1
            } else {
                0
            }),
            unit: Some("{enabled}"),
            description: Some("Whether io_uring is supported."),
        }));

        event_sink.emit(Event::Metric(MetricEvent {
            name: "ferron.admin.runtime.io_uring_runtime_enabled",
            attributes: vec![],
            ty: MetricType::Gauge,
            value: MetricValue::U64(if runtime_metrics.io_uring_runtime_enabled {
                1
            } else {
                0
            }),
            unit: Some("{enabled}"),
            description: Some("Whether io_uring is enabled at runtime."),
        }));
    }
}
