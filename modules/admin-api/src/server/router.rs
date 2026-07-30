//! Admin API axum router builder.
//!
//! Constructs the axum `Router` with routes and middleware
//! based on the parsed `AdminConfig`.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use subtle::ConstantTimeEq;

use crate::config::AdminConfig;
use crate::handlers::{
    admin_metrics_middleware, config_handler, health_handler, reload_get_handler, reload_handler,
    runtime_handler, status_handler, AdminState,
};

/// Shared state for the bearer token auth middleware.
#[derive(Clone)]
pub struct AuthState {
    /// The bearer token required for authentication.
    /// `None` means authentication is disabled.
    pub auth_token: Option<Arc<str>>,
}

/// Axum middleware that enforces Bearer token authentication.
///
/// Extracts the `Authorization` header and validates it against the configured token.
/// Requests to `/health` are always exempt from authentication.
async fn bearer_auth_middleware(
    State(auth_state): State<AuthState>,
    request: Request,
    next: Next,
) -> Response {
    // If no token is configured, allow all requests
    let Some(required_token) = &auth_state.auth_token else {
        return next.run(request).await;
    };

    // Exempt /health from authentication (needed for load balancer / orchestrator probes)
    if request.uri().path() == "/health" {
        return next.run(request).await;
    }

    // Extract Authorization header
    let auth_header = request
        .headers()
        .get(http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());

    match auth_header {
        Some(header_value) => {
            // Validate "Bearer <token>" format
            if let Some(token) = header_value.strip_prefix("Bearer ") {
                // Constant-time comparison to prevent timing attacks
                if token.as_bytes().ct_eq(required_token.as_bytes()).into() {
                    return next.run(request).await;
                }
            }
            // Invalid token or format
            (
                axum::http::StatusCode::UNAUTHORIZED,
                [(http::header::WWW_AUTHENTICATE, "Bearer")],
                "Unauthorized",
            )
                .into_response()
        }
        None => (
            axum::http::StatusCode::UNAUTHORIZED,
            [(http::header::WWW_AUTHENTICATE, "Bearer")],
            "Unauthorized",
        )
            .into_response(),
    }
}

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
    if config.reload_get {
        router = router.route("/reload", get(reload_get_handler));
    }
    if config.runtime {
        router = router.route("/runtime", get(runtime_handler));
    }

    // Fallback for any unmatched admin paths
    router = router.fallback(|| async { (axum::http::StatusCode::NOT_FOUND, "Not Found") });

    // Apply bearer token auth middleware
    let auth_state = AuthState {
        auth_token: config.auth_token.as_deref().map(Arc::from),
    };
    router = router.layer(middleware::from_fn_with_state(
        auth_state,
        bearer_auth_middleware,
    ));

    // Apply admin metrics middleware
    router = router.layer(middleware::from_fn_with_state(
        state.clone(),
        admin_metrics_middleware,
    ));

    router.with_state(state)
}
