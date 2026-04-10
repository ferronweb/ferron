//! Admin API axum handlers.

mod config;
mod status;

use axum::extract::State;
use axum::http::StatusCode;
use ferron_core::config::ServerConfiguration;

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
    ferron_core::shutdown::RELOAD_TOKEN.load().cancel();
    (
        StatusCode::OK,
        axum::Json(serde_json::json!({ "status": "reload_initiated" })),
    )
}
