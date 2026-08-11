//! Admin API axum router builder.
//!
//! Constructs the axum `Router` with routes and middleware
//! based on the parsed `AdminConfig`.

use std::convert::Infallible;
use std::sync::Arc;

use http_body_util::Full;
use hyper::body::Bytes;
use hyper::body::Incoming;
use hyper::{Request, Response};
use subtle::ConstantTimeEq;

use crate::config::AdminConfig;
use crate::handlers::{
    admin_metrics_middleware, config_handler, health_handler, reload_get_handler, reload_handler,
    runtime_handler, status_handler, AdminState,
};

/// A middleware that enforces Bearer token authentication.
///
/// Extracts the `Authorization` header and validates it against the configured token.
/// Requests to `/health` are always exempt from authentication.
async fn bearer_auth_middleware(
    request: &Request<Incoming>,
    auth_token: Option<&str>,
) -> Option<Response<Full<Bytes>>> {
    let required_token = auth_token?;

    // Exempt /health from authentication (needed for load balancer / orchestrator probes)
    if request.uri().path() == "/health" {
        return None;
    }

    let auth_header = request
        .headers()
        .get(http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());

    match auth_header {
        Some(header_value) => {
            if let Some(token) = header_value.strip_prefix("Bearer ") {
                if token.as_bytes().ct_eq(required_token.as_bytes()).into() {
                    return None;
                }
            }
            Some(
                http::Response::builder()
                    .status(http::StatusCode::UNAUTHORIZED)
                    .header(http::header::WWW_AUTHENTICATE, "Bearer")
                    .body(Full::new(Bytes::from_static("Unauthorized".as_bytes())))
                    .expect("invalid HTTP response state"),
            )
        }
        None => Some(
            http::Response::builder()
                .status(http::StatusCode::UNAUTHORIZED)
                .header(http::header::WWW_AUTHENTICATE, "Bearer")
                .body(Full::new(Bytes::from_static("Unauthorized".as_bytes())))
                .expect("invalid HTTP response state"),
        ),
    }
}

pub async fn request_fn(
    request: Request<Incoming>,
    state: AdminState,
    config: Arc<AdminConfig>,
) -> Result<hyper::Response<Full<Bytes>>, Infallible> {
    let state2 = state.clone();
    Ok(
        admin_metrics_middleware(request, state2, |request| async move {
            if let Some(auth_res) =
                bearer_auth_middleware(&request, config.auth_token.as_deref()).await
            {
                return auth_res;
            }

            if config.health
                && request.uri().path() == "/health"
                && request.method() == hyper::Method::GET
            {
                return health_handler().await;
            }
            if config.status
                && request.uri().path() == "/status"
                && request.method() == hyper::Method::GET
            {
                return status_handler().await;
            }
            if config.config
                && request.uri().path() == "/config"
                && request.method() == hyper::Method::GET
            {
                return config_handler(state).await;
            }
            if config.reload_get
                && request.uri().path() == "/reload"
                && request.method() == hyper::Method::POST
            {
                return reload_handler(state).await;
            }
            if config.reload_get
                && request.uri().path() == "/reload"
                && request.method() == hyper::Method::GET
            {
                return reload_get_handler().await;
            }
            if config.runtime
                && request.uri().path() == "/runtime"
                && request.method() == hyper::Method::GET
            {
                return runtime_handler().await;
            }

            http::Response::builder()
                .status(http::StatusCode::NOT_FOUND)
                .body(Full::new(Bytes::from_static("Not found".as_bytes())))
                .expect("invalid HTTP response state")
        })
        .await,
    )
}
