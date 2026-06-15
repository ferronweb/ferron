//! Admin API axum handlers.

mod config;
mod status;

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use ferron_core::config::ServerConfiguration;
use tokio_util::sync::CancellationToken;

use self::status::StatusResponse;

/// Shared state passed to all admin handlers via axum `State` extractor.
#[derive(Clone)]
pub struct AdminState {
    /// The full server configuration, used by the `/config` endpoint.
    pub full_config: std::sync::Arc<ServerConfiguration>,
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
    axum::Json(sanitized)
}

/// `POST /reload` — triggers a configuration reload by cancelling the global reload token.
pub async fn reload_handler(
    State(_state): State<AdminState>,
) -> (StatusCode, axum::Json<serde_json::Value>) {
    {
        let previous_state = ferron_core::shutdown::RELOAD_STATE.load();
        ferron_core::shutdown::RELOAD_TOKEN
            .swap(Arc::new(CancellationToken::new()))
            .cancel();
        previous_state.0.cancelled().await;
    }
    let current_state = ferron_core::shutdown::RELOAD_STATE.load();
    let error = current_state.1.get_state().await;
    if let Some(error) = error {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "status": "reload_failed", "error": error })),
        )
    } else {
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
