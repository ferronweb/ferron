//! Admin API axum router builder.
//!
//! Constructs the axum `Router` with routes and middleware
//! based on the parsed `AdminConfig`.

use axum::routing::{get, post};
use axum::Router;

use crate::config::AdminConfig;
use crate::handlers::{config_handler, health_handler, reload_handler, status_handler, AdminState};

/// Build the admin API axum router.
///
/// Routes are registered based on endpoint enable flags in `AdminConfig`.
/// Disabled endpoints return 404.
pub fn build_admin_router(config: &AdminConfig, state: AdminState) -> Router {
    let mut router = Router::new();

    if config.health {
        router = router.route("/health", get(health_handler));
    }
    if config.status {
        router = router.route("/status", get(status_handler));
    }
    if config.config {
        router = router.route("/config", get(config_handler));
    }
    if config.reload {
        router = router.route("/reload", post(reload_handler));
    }

    // Fallback for any unmatched admin paths
    router = router.fallback(|| async { (axum::http::StatusCode::NOT_FOUND, "Not Found") });

    router.with_state(state)
}
