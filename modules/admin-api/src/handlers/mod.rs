//! Admin API axum handlers.

mod config;
mod status;

use std::sync::Arc;

use ferron_core::config::ServerConfiguration;
use ferron_observability::{
    CompositeEventSink, Event, LogEvent, LogLevel, MetricAttributeValue, MetricEvent, MetricType,
    MetricValue,
};
use http_body_util::Full;
use hyper::body::Bytes;
use hyper::body::Incoming;
use hyper::{Request, Response};
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
pub(crate) async fn admin_metrics_middleware<F, Fut>(
    request: Request<Incoming>,
    state: AdminState,
    request_fn: F,
) -> Response<Full<Bytes>>
where
    F: FnOnce(Request<Incoming>) -> Fut,
    Fut: std::future::Future<Output = Response<Full<Bytes>>>,
{
    let path = request.uri().path().to_string();
    let method = request.method().to_string();

    let start = std::time::Instant::now();
    let response = request_fn(request).await;
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

/// `GET /health`: returns 200 OK if the server is running, or 503 during shutdown.
pub async fn health_handler() -> Response<Full<Bytes>> {
    let shutdown_token = ferron_core::shutdown::SHUTDOWN_TOKEN.load();
    if shutdown_token.is_cancelled() {
        http::Response::builder()
            .status(http::StatusCode::SERVICE_UNAVAILABLE)
            .body(Full::new(Bytes::from_static(
                "Service unavailable".as_bytes(),
            )))
            .expect("invalid HTTP response state")
    } else {
        http::Response::builder()
            .status(http::StatusCode::OK)
            .body(Full::new(Bytes::from_static("OK".as_bytes())))
            .expect("invalid HTTP response state")
    }
}

/// `GET /status`: returns JSON with uptime, connection counts, and reload stats.
pub async fn status_handler() -> Response<Full<Bytes>> {
    let metrics = StatusResponse::from_global();
    http::Response::builder()
        .status(http::StatusCode::OK)
        .header(http::header::CONTENT_TYPE, "application/json")
        .body(Full::new(Bytes::from_owner(
            serde_json::json!({
                "uptime_sec": metrics.uptime_sec,
                "connections_active": metrics.connections_active,
                "requests_total": metrics.requests_total,
                "reloads": metrics.reloads,
                "observability_events_dropped": metrics.observability_events_dropped,
                "observability_event_queue_len": metrics.observability_event_queue_len,
                "config_file_hash": metrics.config_file_hash,
                "config_file_mtime": metrics.config_file_mtime,
                "config_drift": metrics.config_drift,
                "config_drift_hints_enabled": metrics.config_drift_hints_enabled,
                "cache_persistence_dropped_records": metrics.cache_persistence_dropped_records,
                "cache_persistence_errors": metrics.cache_persistence_errors,
                "cache_persistence_zones_inactive": metrics.cache_persistence_zones_inactive,
            })
            .to_string(),
        )))
        .expect("invalid HTTP response state")
}

/// `GET /config`: returns the current effective configuration as sanitized JSON.
pub async fn config_handler(state: AdminState) -> Response<Full<Bytes>> {
    let sanitized = config::sanitize_config(&state.full_config);
    emit_log(
        &state.events,
        LogLevel::Info,
        "Admin config queried".to_string(),
        "Admin config queried",
    );
    http::Response::builder()
        .status(http::StatusCode::OK)
        .header(http::header::CONTENT_TYPE, "application/json")
        .body(Full::new(Bytes::from_owner(sanitized.to_string())))
        .expect("invalid HTTP response state")
}

/// `POST /reload`: triggers a configuration reload by cancelling the global reload token.
pub async fn reload_handler(state: AdminState) -> Response<Full<Bytes>> {
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
        http::Response::builder()
            .status(http::StatusCode::INTERNAL_SERVER_ERROR)
            .header(http::header::CONTENT_TYPE, "application/json")
            .body(Full::new(Bytes::from_owner(
                serde_json::json!({ "status": "reload_failed", "error": error }).to_string(),
            )))
            .expect("invalid HTTP response state")
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
        http::Response::builder()
            .status(http::StatusCode::OK)
            .header(http::header::CONTENT_TYPE, "application/json")
            .body(Full::new(Bytes::from_owner(
                serde_json::json!({ "status": "reload_initiated", "error": null }).to_string(),
            )))
            .expect("invalid HTTP response state")
    }
}

/// `GET /reload`: returns the status of the reload operation.
pub async fn reload_get_handler() -> Response<Full<Bytes>> {
    let metrics = ferron_core::admin::ADMIN_METRICS.reload_metrics.read();
    http::Response::builder()
        .status(http::StatusCode::OK)
        .header(http::header::CONTENT_TYPE, "application/json")
        .body(Full::new(Bytes::from_owner(
            serde_json::json!({
                "last_reload_time": chrono::DateTime::<chrono::Utc>::from(metrics.last_reload_time).to_rfc3339(), // ISO 8601 format
                "last_reload_error": metrics.last_reload_error,
                "active_generation": metrics.active_generation,
            })
            .to_string(),
        )))
        .expect("invalid HTTP response state")
}

/// `GET /runtime`: returns the runtime status.
pub async fn runtime_handler() -> Response<Full<Bytes>> {
    let metrics = ferron_core::admin::ADMIN_METRICS.runtime_metrics.read();
    http::Response::builder()
        .status(http::StatusCode::OK)
        .header(http::header::CONTENT_TYPE, "application/json")
        .body(Full::new(Bytes::from_owner(
            serde_json::json!({
                "primary_threads": metrics.primary_threads,
                "io_uring_supported": metrics.io_uring_supported,
                "io_uring_runtime_enabled": metrics.io_uring_runtime_enabled,
            })
            .to_string(),
        )))
        .expect("invalid HTTP response state")
}
