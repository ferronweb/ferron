use std::time::Duration;

use http::header::HeaderName;

pub(super) const PURGE_SOURCE_HEADER: HeaderName = HeaderName::from_static("x-purge-source");
/// Header sent in outbound purge webhooks to identify the originating edge node.
/// The external control-plane uses this to avoid broadcasting back to the origin.
#[allow(dead_code)]
pub(super) const PURGE_ORIGIN_HEADER: HeaderName = HeaderName::from_static("x-purge-origin");
pub(super) const PURGE_SECRET_HEADER: HeaderName = HeaderName::from_static("x-purge-secret");

/// Build an HTTPS client for outbound purge propagation webhooks.
pub(super) fn build_propagation_client() -> Result<
    hyper_util::client::legacy::Client<
        hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
        http_body_util::Full<bytes::Bytes>,
    >,
    Box<dyn std::error::Error + Send + Sync>,
> {
    use hyper_rustls::HttpsConnectorBuilder;

    let root_store = build_root_cert_store()?;

    let tls_config = rustls::ClientConfig::builder_with_provider(
        rustls::crypto::aws_lc_rs::default_provider().into(),
    )
    .with_safe_default_protocol_versions()?
    .with_root_certificates(root_store)
    .with_no_client_auth();

    let https = HttpsConnectorBuilder::new()
        .with_tls_config(tls_config)
        .https_or_http()
        .enable_http1()
        .enable_http2()
        .build();

    Ok(
        hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
            .build(https),
    )
}

fn build_root_cert_store() -> Result<rustls::RootCertStore, Box<dyn std::error::Error + Send + Sync>>
{
    let mut root_store = rustls::RootCertStore::empty();
    let mut found_any = false;

    match rustls_native_certs::load_native_certs() {
        cert_result if !cert_result.errors.is_empty() => {
            ferron_core::log_warn!(
                "native root CA certificate loading errors: {:?}",
                cert_result.errors
            );
        }
        cert_result if cert_result.certs.is_empty() => {
            ferron_core::log_warn!("no native root CA certificates found");
        }
        cert_result => {
            for cert in cert_result.certs {
                if let Err(err) = root_store.add(cert) {
                    ferron_core::log_warn!("native certificate parsing failed: {:?}", err);
                } else {
                    found_any = true;
                }
            }
        }
    }

    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    if !found_any {
        ferron_core::log_warn!("using webpki-roots as fallback (no native root CAs available)");
    }

    if root_store.is_empty() {
        return Err("No root certificates available".into());
    }

    Ok(root_store)
}

/// Send a purge webhook to the external control-plane service.
///
/// The webhook is a `POST` with a JSON body containing the purged path and the
/// originating node ID. The control-plane is expected to fan out `PURGE`
/// requests to all other registered edges.
pub(super) async fn propagate_purge_webhook(
    url: &str,
    shared_secret: Option<&str>,
    node_id: Option<&str>,
    path: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let client = build_propagation_client()?;

    let body = serde_json::json!({
        "path": path,
        "origin": node_id.unwrap_or("unknown"),
    });

    let mut request = http::Request::builder()
        .method(http::Method::POST)
        .uri(url)
        .header(http::header::CONTENT_TYPE, "application/json");

    if let Some(secret) = shared_secret {
        request = request.header(&PURGE_SECRET_HEADER, secret);
    }

    let request = request.body(http_body_util::Full::new(bytes::Bytes::from(
        serde_json::to_vec(&body)?,
    )))?;

    let response = tokio::time::timeout(Duration::from_secs(5), client.request(request)).await??;

    if !response.status().is_success() {
        return Err(format!("control-plane returned HTTP {}", response.status()).into());
    }

    Ok(())
}
