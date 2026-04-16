use axum::extract::State;
use prometheus::{Encoder, Registry};
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

pub async fn endpoint_listener_fn(
    config: PrometheusBackendConfig,
    reload_token: CancellationToken,
    registry: prometheus::Registry,
) -> Result<(), Box<dyn std::error::Error>> {
    // Axum server
    let app = axum::Router::new()
        .route("/metrics", axum::routing::get(endpoint_fn))
        .with_state((registry, config.format));
    let listener = tokio::net::TcpListener::bind(config.listen).await?;
    ferron_core::log_info!("Prometheus endpoint listening on {}", config.listen);
    let server = axum::serve(listener, app.into_make_service());

    tokio::select! {
        _ = reload_token.cancelled() => {
            ferron_core::log_info!("Admin API shutting down (reload)");
        }
        result = server => {
            result?;
        }
    }

    Ok(())
}

async fn endpoint_fn(
    State((registry, format)): State<(Registry, String)>,
) -> Result<axum::response::Response, axum::http::StatusCode> {
    let encoder: AnyEncoder = match format.as_str() {
        "protobuf" => AnyEncoder::Protobuf(prometheus::ProtobufEncoder::new()),
        _ => AnyEncoder::Text(prometheus::TextEncoder::new()),
    };
    let mut buffer = Vec::new();
    if encoder.encode(&registry.gather(), &mut buffer).is_err() {
        return Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR);
    }
    axum::response::Response::builder()
        .header(axum::http::header::CONTENT_TYPE, encoder.format_type())
        .body(axum::body::Body::from(buffer))
        .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)
}
