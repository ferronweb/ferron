//! Admin API axum handlers.

mod config;
mod status;

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;
use ferron_core::config::ServerConfiguration;
use ferron_observability::{
    CompositeEventSink, Event, LogEvent, LogLevel, MetricAttributeValue, MetricEvent, MetricType,
    MetricValue,
};
use tokio_util::sync::CancellationToken;

use self::status::StatusResponse;

/// Shared state passed to all admin handlers via axum `State` extractor.
#[derive(Clone)]
pub struct AdminState {
    /// The full server configuration, used by the `/config` endpoint.
    pub full_config: std::sync::Arc<ServerConfiguration>,
    /// Observability event sink for emitting metrics and logs.
    pub events: std::sync::Arc<CompositeEventSink>,
}

/// Axum middleware that emits per-request metrics for the admin API.
///
/// Skips metrics for `/health` (high-frequency probe, low signal).
pub(crate) async fn admin_metrics_middleware(
    State(state): State<AdminState>,
    request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path().to_string();
    let method = request.method().to_string();

    let start = std::time::Instant::now();
    let response = next.run(request).await;
    let duration = start.elapsed().as_secs_f64();
    let status_code = response.status().as_u16();

    // Skip metrics for /health (frequent probe, low signal)
    if path != "/health" {
        let attrs = vec![
            (
                "http.request.method",
                MetricAttributeValue::StaticStr(method_to_label(&method)),
            ),
            ("url.path", MetricAttributeValue::String(path.clone())),
            (
                "http.response.status_code",
                MetricAttributeValue::I64(status_code as i64),
            ),
        ];

        state.events.emit(Event::Metric(MetricEvent {
            name: "ferron.admin.request.duration",
            attributes: attrs.clone(),
            ty: MetricType::Histogram(Some(
                vec![
                    0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1.0, 2.5, 5.0,
                ]
                .into(),
            )),
            value: MetricValue::F64(duration),
            unit: Some("s"),
            description: Some("Duration of admin API requests"),
            trace_context: None,
        }));

        state.events.emit(Event::Metric(MetricEvent {
            name: "ferron.admin.request.count",
            attributes: attrs,
            ty: MetricType::Counter,
            value: MetricValue::U64(1),
            unit: None,
            description: Some("Total number of admin API requests"),
            trace_context: None,
        }));
    }

    response
}

/// Map HTTP method to a bounded set of labels to prevent cardinality explosion.
fn method_to_label(method: &str) -> &'static str {
    match method {
        "GET" => "GET",
        "POST" => "POST",
        "PUT" => "PUT",
        "DELETE" => "DELETE",
        "PATCH" => "PATCH",
        "HEAD" => "HEAD",
        "OPTIONS" => "OPTIONS",
        _ => "_other",
    }
}

/// Emit a structured log event through the observability pipeline.
fn emit_log(events: &CompositeEventSink, level: LogLevel, message: String, summary: &'static str) {
    events.emit(Event::Log(LogEvent {
        level,
        message,
        summary: std::borrow::Cow::Borrowed(summary),
        target: "ferron-admin-api",
        attributes: vec![],
        trace_context: None,
    }));
}

/// `GET /health` — returns 200 OK if the server is running, or 503 during shutdown.
pub async fn health_handler(State(_state): State<AdminState>) -> (StatusCode, &'static str) {
    let shutdown_token = ferron_core::shutdown::SHUTDOWN_TOKEN.load();
    if shutdown_token.is_cancelled() {
        (StatusCode::SERVICE_UNAVAILABLE, "Service Unavailable")
    } else {
        (StatusCode::OK, "OK")
    }
}

/// `GET /status` — returns JSON with uptime, connection counts, and reload stats.
pub async fn status_handler(State(_state): State<AdminState>) -> axum::Json<serde_json::Value> {
    let metrics = StatusResponse::from_global();
    axum::Json(serde_json::json!({
        "uptime_sec": metrics.uptime_sec,
        "connections_active": metrics.connections_active,
        "requests_total": metrics.requests_total,
        "reloads": metrics.reloads,
        "observability_events_dropped": metrics.observability_events_dropped,
        "observability_event_queue_len": metrics.observability_event_queue_len,
    }))
}

/// `GET /config` — returns the current effective configuration as sanitized JSON.
pub async fn config_handler(State(state): State<AdminState>) -> axum::Json<serde_json::Value> {
    let sanitized = config::sanitize_config(&state.full_config);
    emit_log(
        &state.events,
        LogLevel::Info,
        "Admin config queried".to_string(),
        "Admin config queried",
    );
    axum::Json(sanitized)
}

/// `POST /reload` — triggers a configuration reload by cancelling the global reload token.
pub async fn reload_handler(
    State(state): State<AdminState>,
) -> (StatusCode, axum::Json<serde_json::Value>) {
    let start = std::time::Instant::now();
    {
        let previous_state = ferron_core::shutdown::RELOAD_STATE.load();
        ferron_core::shutdown::RELOAD_TOKEN
            .swap(Arc::new(CancellationToken::new()))
            .cancel();
        previous_state.0.cancelled().await;
    }
    let duration = start.elapsed().as_secs_f64();
    let current_state = ferron_core::shutdown::RELOAD_STATE.load();
    let error = current_state.1.get_state().await;
    if let Some(error) = error {
        emit_log(
            &state.events,
            LogLevel::Error,
            format!("Admin config reload failed: {error}"),
            "Admin config reload failed",
        );
        state.events.emit(Event::Metric(MetricEvent {
            name: "ferron.admin.reload.count",
            attributes: vec![("http.response.status_code", MetricAttributeValue::I64(500))],
            ty: MetricType::Counter,
            value: MetricValue::U64(1),
            unit: None,
            description: Some("Total admin config reload attempts"),
            trace_context: None,
        }));
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "status": "reload_failed", "error": error })),
        )
    } else {
        emit_log(
            &state.events,
            LogLevel::Info,
            format!("Admin config reload completed in {duration:.3}s"),
            "Admin config reload completed",
        );
        state.events.emit(Event::Metric(MetricEvent {
            name: "ferron.admin.reload.count",
            attributes: vec![("http.response.status_code", MetricAttributeValue::I64(200))],
            ty: MetricType::Counter,
            value: MetricValue::U64(1),
            unit: None,
            description: Some("Total admin config reload attempts"),
            trace_context: None,
        }));
        (
            StatusCode::OK,
            axum::Json(serde_json::json!({ "status": "reload_initiated", "error": null })),
        )
    }
}

/// `GET /reload` — returns the status of the reload operation.
pub async fn reload_get_handler(State(_state): State<AdminState>) -> axum::Json<serde_json::Value> {
    let metrics = ferron_core::admin::ADMIN_METRICS.reload_metrics.read();
    axum::Json(serde_json::json!({
        "last_reload_time": chrono::DateTime::<chrono::Utc>::from(metrics.last_reload_time).to_rfc3339(), // ISO 8601 format
        "last_reload_error": metrics.last_reload_error,
        "active_generation": metrics.active_generation,
    }))
}

/// `GET /runtime` — returns the runtime status.
pub async fn runtime_handler(State(_state): State<AdminState>) -> axum::Json<serde_json::Value> {
    let metrics = ferron_core::admin::ADMIN_METRICS.runtime_metrics.read();
    axum::Json(serde_json::json!({
        "primary_threads": metrics.primary_threads,
        "io_uring_supported": metrics.io_uring_supported,
        "io_uring_runtime_enabled": metrics.io_uring_runtime_enabled,
    }))
}
