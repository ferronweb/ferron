use std::sync::Arc;

use axum::extract::{Request, State};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::histogram::Histogram;
use subtle::ConstantTimeEq;
use tokio_util::sync::CancellationToken;

use crate::PrometheusBackendConfig;

type EndpointState = (
    Arc<tokio::sync::RwLock<prometheus_client::registry::Registry>>,
    String,
    Histogram,
    Counter,
    Counter,
);

/// Shared state for the bearer token auth middleware.
#[derive(Clone)]
struct AuthState {
    auth_token: Option<Arc<str>>,
}

/// Axum middleware that enforces Bearer token authentication for the metrics endpoint.
async fn bearer_auth_middleware(
    State(auth_state): State<AuthState>,
    request: Request,
    next: Next,
) -> Response {
    let Some(required_token) = &auth_state.auth_token else {
        return next.run(request).await;
    };

    let auth_header = request
        .headers()
        .get(http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());

    match auth_header {
        Some(header_value) => {
            if let Some(token) = header_value.strip_prefix("Bearer ") {
                if token.as_bytes().ct_eq(required_token.as_bytes()).into() {
                    return next.run(request).await;
                }
            }
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

pub async fn endpoint_listener_fn(
    config: PrometheusBackendConfig,
    reload_token: CancellationToken,
    registry: Arc<tokio::sync::RwLock<prometheus_client::registry::Registry>>,
    scrape_duration: Histogram,
    scrape_total: Counter,
    scrape_errors: Counter,
) -> Result<(), Box<dyn std::error::Error>> {
    let auth_token = config.auth_token.as_deref().map(Arc::from);

    let app = axum::Router::new()
        .route("/metrics", axum::routing::get(endpoint_fn))
        .with_state((
            registry,
            config.format,
            scrape_duration,
            scrape_total,
            scrape_errors,
        ));

    let app = if let Some(token) = auth_token {
        let auth_state = AuthState {
            auth_token: Some(token),
        };
        app.layer(middleware::from_fn_with_state(
            auth_state,
            bearer_auth_middleware,
        ))
    } else {
        app
    };

    let listener = tokio::net::TcpListener::bind(config.listen).await?;
    ferron_core::log_info!("Prometheus endpoint listening on {}", config.listen);
    let server = axum::serve(listener, app.into_make_service());

    tokio::select! {
        _ = reload_token.cancelled() => {
            ferron_core::log_info!("Prometheus endpoint shutting down (reload)");
        }
        result = server => {
            result?;
        }
    }

    Ok(())
}

async fn endpoint_fn(
    State((registry, format, scrape_duration, scrape_total, _scrape_errors)): State<EndpointState>,
) -> Result<axum::response::Response, axum::http::StatusCode> {
    let start = std::time::Instant::now();
    scrape_total.inc();

    match format.as_str() {
        "protobuf" => {
            let buffer = prometheus_client::encoding::prometheus_protobuf::encode_to_vec(
                &*registry.read().await,
            )
            .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;

            let duration = start.elapsed().as_secs_f64();
            scrape_duration.observe(duration);

            axum::response::Response::builder()
                .header(
                    axum::http::header::CONTENT_TYPE,
                    "application/vnd.google.protobuf; proto=io.prometheus.client.MetricFamily; encode=delimited",
                )
                .body(axum::body::Body::from(buffer))
                .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)
        }
        _ => {
            let mut buffer = String::new();
            prometheus_client::encoding::text::encode(&mut buffer, &*registry.read().await)
                .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;

            let duration = start.elapsed().as_secs_f64();
            scrape_duration.observe(duration);

            axum::response::Response::builder()
                .header(
                    axum::http::header::CONTENT_TYPE,
                    "application/openmetrics-text; version=1.0.0; charset=utf-8",
                )
                .body(axum::body::Body::from(buffer))
                .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}
