use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use openidconnect::{AsyncHttpClient, HttpRequest, HttpResponse};

/// The maximum accepted size of an OIDC provider response body
const MAXIMUM_RESPONSE_BODY_SIZE: u64 = 1024 * 1024;

/// The timeout for requests to the OIDC provider
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// An error occurring while sending a request to the OIDC provider
#[derive(Debug)]
pub struct OidcHttpClientError(String);

impl std::fmt::Display for OidcHttpClientError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.write_str(&self.0)
  }
}

impl std::error::Error for OidcHttpClientError {}

/// Builds an HTTP client for communication with the OIDC provider
pub fn build_http_client(no_verification: bool) -> Result<reqwest::Client, Box<dyn std::error::Error + Send + Sync>> {
  let mut builder = reqwest::Client::builder()
    .timeout(REQUEST_TIMEOUT)
    // Following redirects opens the client up to SSRF vulnerabilities
    .redirect(reqwest::redirect::Policy::none());
  if no_verification {
    builder = builder.danger_accept_invalid_certs(true);
  }
  Ok(
    builder
      .build()
      .map_err(|e| anyhow::anyhow!("Failed to build the OIDC HTTP client: {e}"))?,
  )
}

/// An `openidconnect` asynchronous HTTP client backed by a `reqwest` client
pub struct OidcHttpClient(reqwest::Client);

impl OidcHttpClient {
  /// Wraps a `reqwest` client for use with the `openidconnect` crate
  pub fn new(client: reqwest::Client) -> Self {
    Self(client)
  }
}

impl<'c> AsyncHttpClient<'c> for OidcHttpClient {
  type Error = OidcHttpClientError;
  type Future = Pin<Box<dyn Future<Output = Result<HttpResponse, OidcHttpClientError>> + Send + 'c>>;

  fn call(&'c self, request: HttpRequest) -> Self::Future {
    Box::pin(execute(&self.0, request))
  }
}

/// Executes an OIDC provider request
async fn execute(client: &reqwest::Client, request: HttpRequest) -> Result<HttpResponse, OidcHttpClientError> {
  let request = reqwest::Request::try_from(request).map_err(|e| OidcHttpClientError(e.to_string()))?;
  let response = client
    .execute(request)
    .await
    .map_err(|e| OidcHttpClientError(e.to_string()))?;
  if response.content_length().unwrap_or(0) > MAXIMUM_RESPONSE_BODY_SIZE {
    return Err(OidcHttpClientError("OIDC provider response body too large".to_string()));
  }
  let mut builder = openidconnect::http::Response::builder().status(response.status());
  if let Some(headers) = builder.headers_mut() {
    *headers = response.headers().clone();
  }
  let body = response.bytes().await.map_err(|e| OidcHttpClientError(e.to_string()))?;
  if body.len() as u64 > MAXIMUM_RESPONSE_BODY_SIZE {
    return Err(OidcHttpClientError("OIDC provider response body too large".to_string()));
  }
  builder
    .body(body.to_vec())
    .map_err(|e| OidcHttpClientError(e.to_string()))
}
