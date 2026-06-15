use std::sync::Arc;
use std::time::Duration;

use ferron_core::shutdown::RELOAD_STATE;
use ferron_observability::{
    CompositeEventSink, Event, LogAttributeValue, LogEvent, MetricAttributeValue, MetricEvent,
    MetricType, MetricValue,
};

/// Runs the background reload API metrics collection loop.
pub async fn collect_reload_metrics(
    event_sink: Arc<CompositeEventSink>,
    cancel_token: tokio_util::sync::CancellationToken,
) {
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut reload_state = RELOAD_STATE.load_full();

    loop {
        tokio::select! {
            _ = cancel_token.cancelled() => break,
            _ = reload_state.0.cancelled() => {}
        }

        event_sink.emit(Event::Log(LogEvent {
            level: ferron_observability::LogLevel::Info,
            message: "Reloading configuration...".to_string(),
            summary: "Configuration reload".into(),
            target: "ferron-metrics-reload",
            attributes: vec![],
            trace_context: None,
        }));

        reload_state = RELOAD_STATE.load_full();
        let error = reload_state.1.get_state().await;

        if let Some(error) = &error {
            event_sink.emit(Event::Log(LogEvent {
                level: ferron_observability::LogLevel::Warn,
                message: format!("Can't reload the server, continuing to run with the previous configuration: {error}"),
                summary: "Configuration reload error".into(),
                target: "ferron-metrics-reload",
                attributes: vec![
                    ("error.message", LogAttributeValue::String(error.to_string()))
                ],
                trace_context: None,
            }));
        }

        let mut attributes = vec![
            ((
                "ferron.reload.successful",
                MetricAttributeValue::Bool(error.is_none()),
            )),
        ];
        if let Some(error) = error {
            attributes.push(("error.message", MetricAttributeValue::String(error)));
        }
        event_sink.emit(Event::Metric(MetricEvent {
            name: "ferron.reloads",
            attributes,
            ty: MetricType::Counter,
            value: MetricValue::U64(1),
            unit: Some("{reload}"),
            description: Some("Number of configuration reloads performed."),
            trace_context: None,
        }));
    }
}
