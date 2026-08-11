use std::convert::Infallible;
use std::sync::Arc;

use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::service::service_fn;
use hyper::{Request, Response};
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

/// A middleware that enforces Bearer token authentication for the metrics endpoint.
async fn bearer_auth_middleware(
    request: &Request<Incoming>,
    auth_token: Option<&str>,
) -> Option<Response<Full<Bytes>>> {
    let Some(required_token) = auth_token else {
        return None;
    };

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

pub async fn endpoint_listener_fn(
    config: PrometheusBackendConfig,
    reload_token: CancellationToken,
    registry: Arc<tokio::sync::RwLock<prometheus_client::registry::Registry>>,
    scrape_duration: Histogram,
    scrape_total: Counter,
    scrape_errors: Counter,
) -> Result<(), Box<dyn std::error::Error>> {
    let auth_token = config.auth_token.as_deref().map(Arc::from);
    let endpoint_state = (
        registry,
        config.format,
        scrape_duration,
        scrape_total,
        scrape_errors,
    );

    let listener = tokio::net::TcpListener::bind(config.listen).await?;
    ferron_core::log_info!("Prometheus endpoint listening on {}", config.listen);
    let server = async {
        loop {
            let Ok((sock, _)) = listener.accept().await else {
                continue;
            };

            let _ = sock.set_nodelay(true);

            let auth_token = auth_token.clone();
            let endpoint_state = endpoint_state.clone();

            tokio::spawn(async move {
                let _ = hyper::server::conn::http1::Builder::new()
                    .timer(hyper_util::rt::TokioTimer::default())
                    .serve_connection(
                        hyper_util::rt::TokioIo::new(sock),
                        service_fn(|request| {
                            request_fn(request, auth_token.clone(), endpoint_state.clone())
                        }),
                    )
                    .await;
            });
        }
    };

    tokio::select! {
        _ = reload_token.cancelled() => {
            ferron_core::log_info!("Prometheus endpoint shutting down (reload)");
        }
        _ = server => {}
    }

    Ok(())
}

async fn request_fn(
    request: Request<Incoming>,
    auth_token: Option<Arc<str>>,
    endpoint_state: EndpointState,
) -> Result<hyper::Response<Full<Bytes>>, Infallible> {
    if let Some(auth_res) = bearer_auth_middleware(&request, auth_token.as_deref()).await {
        return Ok(auth_res);
    }

    if request.uri().path() == "/metrics" {
        return Ok(endpoint_fn(endpoint_state).await.unwrap_or_else(|_| {
            http::Response::builder()
                .status(http::StatusCode::INTERNAL_SERVER_ERROR)
                .body(Full::new(Bytes::from_static(
                    "Internal server error".as_bytes(),
                )))
                .expect("invalid HTTP response state")
        }));
    }

    Ok(http::Response::builder()
        .status(http::StatusCode::NOT_FOUND)
        .body(Full::new(Bytes::from_static("Not found".as_bytes())))
        .expect("invalid HTTP response state"))
}

async fn endpoint_fn(
    (registry, format, scrape_duration, scrape_total, _scrape_errors): EndpointState,
) -> anyhow::Result<hyper::Response<Full<Bytes>>> {
    let start = std::time::Instant::now();
    scrape_total.inc();

    match format.as_str() {
        "protobuf" => {
            let buffer = prometheus_client::encoding::prometheus_protobuf::encode_to_vec(
                &*registry.read().await,
            )?;

            let duration = start.elapsed().as_secs_f64();
            scrape_duration.observe(duration);

            Ok(http::Response::builder()
                    .status(http::StatusCode::OK)
                    .header(
                        http::header::CONTENT_TYPE,
                        "application/vnd.google.protobuf; proto=io.prometheus.client.MetricFamily; encode=delimited",
                    )
                    .body(Full::new(Bytes::from_owner(buffer)))?)
        }
        _ => {
            let mut buffer = String::new();
            prometheus_client::encoding::text::encode(&mut buffer, &*registry.read().await)?;

            let duration = start.elapsed().as_secs_f64();
            scrape_duration.observe(duration);

            Ok(http::Response::builder()
                .status(http::StatusCode::OK)
                .header(
                    http::header::CONTENT_TYPE,
                    "application/openmetrics-text; version=1.0.0; charset=utf-8",
                )
                .body(Full::new(Bytes::from_owner(buffer)))?)
        }
    }
}
