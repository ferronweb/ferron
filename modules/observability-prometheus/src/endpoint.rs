use std::sync::Arc;

use axum::extract::{Request, State};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use prometheus::{Encoder, Histogram, Registry};
use subtle::ConstantTimeEq;
use tokio_util::sync::CancellationToken;

use crate::PrometheusBackendConfig;

pub enum AnyEncoder {
    Text(prometheus::TextEncoder),
    Protobuf(prometheus::ProtobufEncoder),
}

impl prometheus::Encoder for AnyEncoder {
    fn encode<W: std::io::Write>(
        &self,
        metric_families: &[prometheus::proto::MetricFamily],
        writer: &mut W,
    ) -> Result<(), prometheus::Error> {
        match self {
            AnyEncoder::Text(encoder) => encoder.encode(metric_families, writer),
            AnyEncoder::Protobuf(encoder) => encoder.encode(metric_families, writer),
        }
    }

    fn format_type(&self) -> &str {
        match self {
            AnyEncoder::Text(encoder) => encoder.format_type(),
            AnyEncoder::Protobuf(encoder) => encoder.format_type(),
        }
    }
}

/// Shared state for the bearer token auth middleware.
#[derive(Clone)]
struct AuthState {
    /// The bearer token required for authentication.
    /// `None` means authentication is disabled.
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
    registry: prometheus::Registry,
) -> Result<(), Box<dyn std::error::Error>> {
    let auth_token = config.auth_token.as_deref().map(Arc::from);

    // Register self-referential scrape metrics
    let scrape_duration = prometheus::Histogram::with_opts(prometheus::HistogramOpts {
        common_opts: prometheus::Opts {
            namespace: String::new(),
            subsystem: String::new(),
            name: "ferron_prometheus_scrape_duration_seconds".to_string(),
            help: "Duration of Prometheus scrape requests in seconds".to_string(),
            const_labels: Default::default(),
            variable_labels: Vec::new(),
        },
        buckets: vec![
            0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0,
        ],
    })?;
    registry.register(Box::new(scrape_duration.clone()))?;

    let scrape_total = prometheus::IntCounter::with_opts(prometheus::Opts {
        namespace: String::new(),
        subsystem: String::new(),
        name: "ferron_prometheus_scrape_total".to_string(),
        help: "Total number of Prometheus scrape requests".to_string(),
        const_labels: Default::default(),
        variable_labels: Vec::new(),
    })?;
    registry.register(Box::new(scrape_total.clone()))?;

    let scrape_errors = prometheus::IntCounter::with_opts(prometheus::Opts {
        namespace: String::new(),
        subsystem: String::new(),
        name: "ferron_prometheus_scrape_errors_total".to_string(),
        help: "Total number of failed Prometheus scrape requests".to_string(),
        const_labels: Default::default(),
        variable_labels: Vec::new(),
    })?;
    registry.register(Box::new(scrape_errors.clone()))?;

    // Axum server
    let app = axum::Router::new()
        .route("/metrics", axum::routing::get(endpoint_fn))
        .with_state((
            registry,
            config.format,
            scrape_duration,
            scrape_total,
            scrape_errors,
        ));

    // Apply bearer token auth middleware if a token is configured
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
    State((registry, format, scrape_duration, scrape_total, scrape_errors)): State<(
        Registry,
        String,
        Histogram,
        prometheus::IntCounter,
        prometheus::IntCounter,
    )>,
) -> Result<axum::response::Response, axum::http::StatusCode> {
    let start = std::time::Instant::now();
    scrape_total.inc();

    let encoder: AnyEncoder = match format.as_str() {
        "protobuf" => AnyEncoder::Protobuf(prometheus::ProtobufEncoder::new()),
        _ => AnyEncoder::Text(prometheus::TextEncoder::new()),
    };
    let mut buffer = Vec::new();
    let result = encoder.encode(&registry.gather(), &mut buffer);

    let duration = start.elapsed().as_secs_f64();
    scrape_duration.observe(duration);

    if result.is_err() {
        scrape_errors.inc();
        return Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR);
    }

    axum::response::Response::builder()
        .header(axum::http::header::CONTENT_TYPE, encoder.format_type())
        .body(axum::body::Body::from(buffer))
        .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)
}
