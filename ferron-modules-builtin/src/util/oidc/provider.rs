use std::error::Error;
use std::time::{Duration, Instant};

use openidconnect::core::CoreProviderMetadata;
use openidconnect::IssuerUrl;
use tokio::sync::RwLock;

use crate::util::oidc::http_client::OidcHttpClient;

/// The time after which the cached OIDC provider metadata is refreshed
const METADATA_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// The time during which OIDC provider metadata fetches aren't retried after a failure
const FAILURE_BACKOFF: Duration = Duration::from_secs(10);

/// A cache for OIDC provider metadata obtained through OpenID Connect Discovery
pub struct ProviderCache {
  state: RwLock<ProviderCacheState>,
}

#[derive(Default)]
struct ProviderCacheState {
  metadata: Option<(CoreProviderMetadata, Instant)>,
  last_failure: Option<Instant>,
}

impl Default for ProviderCache {
  fn default() -> Self {
    Self::new()
  }
}

impl ProviderCache {
  /// Creates an empty OIDC provider metadata cache
  pub fn new() -> Self {
    Self {
      state: RwLock::new(ProviderCacheState::default()),
    }
  }

  /// Obtains the OIDC provider metadata, fetching it through OpenID Connect Discovery if
  /// it's not cached or the cached metadata expired
  pub async fn get_metadata(
    &self,
    http_client: &reqwest::Client,
    issuer_url: &str,
  ) -> Result<CoreProviderMetadata, Box<dyn Error + Send + Sync>> {
    {
      let state = self.state.read().await;
      if let Some((metadata, fetched_at)) = &state.metadata {
        if fetched_at.elapsed() < METADATA_TTL {
          return Ok(metadata.clone());
        }
      }
    }
    self.refresh_metadata(http_client, issuer_url).await
  }

  /// Fetches the OIDC provider metadata through OpenID Connect Discovery, bypassing
  /// the cached metadata (used for OIDC provider signing key rotation)
  pub async fn refresh_metadata(
    &self,
    http_client: &reqwest::Client,
    issuer_url: &str,
  ) -> Result<CoreProviderMetadata, Box<dyn Error + Send + Sync>> {
    let mut state = self.state.write().await;

    if let Some(last_failure) = state.last_failure {
      if last_failure.elapsed() < FAILURE_BACKOFF {
        // Serve stale metadata if available instead of hammering a failing OIDC provider
        if let Some((metadata, _)) = &state.metadata {
          return Ok(metadata.clone());
        }
        Err(anyhow::anyhow!("The OIDC provider metadata cannot be fetched"))?;
      }
    }

    let issuer_url_parsed =
      IssuerUrl::new(issuer_url.to_string()).map_err(|e| anyhow::anyhow!("Invalid OIDC issuer URL: {e}"))?;
    let oidc_http_client = OidcHttpClient::new(http_client.clone());
    match CoreProviderMetadata::discover_async(issuer_url_parsed, &oidc_http_client).await {
      Ok(metadata) => {
        state.metadata = Some((metadata.clone(), Instant::now()));
        state.last_failure = None;
        Ok(metadata)
      }
      Err(e) => {
        state.last_failure = Some(Instant::now());
        // Serve stale metadata if available
        if let Some((metadata, _)) = &state.metadata {
          return Ok(metadata.clone());
        }
        Err(anyhow::anyhow!("Failed to fetch the OIDC provider metadata: {e}"))?
      }
    }
  }
}
