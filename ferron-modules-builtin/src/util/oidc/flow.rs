use std::error::Error;

use base64::prelude::*;
use openidconnect::core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata};
use openidconnect::{
  AccessTokenHash, AuthorizationCode, ClientId, ClientSecret, CsrfToken, Nonce, OAuth2TokenResponse, PkceCodeChallenge,
  PkceCodeVerifier, RedirectUrl, Scope, TokenResponse,
};

use crate::util::oidc::http_client::OidcHttpClient;

/// The OIDC relying party configuration used by the authorization code flow
pub struct OidcFlowConfig {
  /// The OAuth2 client ID
  pub client_id: String,
  /// The OAuth2 client secret
  pub client_secret: Option<String>,
  /// The requested scopes
  pub scopes: Vec<String>,
  /// The redirect URL (constructed from the request and the configured redirect path)
  pub redirect_url: String,
  /// Additional accepted ID token audiences (besides the client ID)
  pub audiences: Vec<String>,
}

/// The parameters of a started OIDC login
pub struct BeginLogin {
  /// The OIDC provider authorization URL to redirect the user to
  pub authorization_url: String,
  /// The OAuth2 state parameter
  pub state: String,
  /// The OpenID Connect nonce
  pub nonce: String,
  /// The PKCE code verifier
  pub pkce_verifier: String,
}

/// The identity claims extracted from a verified ID token
pub struct SessionClaims {
  /// The OIDC subject identifier
  pub sub: String,
  /// The preferred username
  pub username: Option<String>,
  /// The email address
  pub email: Option<String>,
  /// The display name
  pub name: Option<String>,
  /// The groups the user belongs to
  pub groups: Vec<String>,
}

/// The OIDC client type constructed from the provider metadata
type ProviderCoreClient = CoreClient<
  openidconnect::EndpointSet,
  openidconnect::EndpointNotSet,
  openidconnect::EndpointNotSet,
  openidconnect::EndpointNotSet,
  openidconnect::EndpointMaybeSet,
  openidconnect::EndpointMaybeSet,
>;

/// Constructs an OIDC client from the provider metadata and the relying party configuration
fn build_client(
  metadata: CoreProviderMetadata,
  config: &OidcFlowConfig,
) -> Result<ProviderCoreClient, Box<dyn Error + Send + Sync>> {
  Ok(
    CoreClient::from_provider_metadata(
      metadata,
      ClientId::new(config.client_id.clone()),
      config.client_secret.clone().map(ClientSecret::new),
    )
    .set_redirect_uri(
      RedirectUrl::new(config.redirect_url.clone()).map_err(|e| anyhow::anyhow!("Invalid OIDC redirect URL: {e}"))?,
    ),
  )
}

/// Begins an OIDC login by generating the authorization URL with PKCE, state, and nonce
pub fn begin_login(
  metadata: CoreProviderMetadata,
  config: &OidcFlowConfig,
) -> Result<BeginLogin, Box<dyn Error + Send + Sync>> {
  let client = build_client(metadata, config)?;
  let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
  let mut authorization_request = client
    .authorize_url(
      CoreAuthenticationFlow::AuthorizationCode,
      CsrfToken::new_random,
      Nonce::new_random,
    )
    .set_pkce_challenge(pkce_challenge);
  for scope in &config.scopes {
    // The "openid" scope is always added by the OIDC library
    if scope != "openid" {
      authorization_request = authorization_request.add_scope(Scope::new(scope.clone()));
    }
  }
  let (authorization_url, state, nonce) = authorization_request.url();
  Ok(BeginLogin {
    authorization_url: authorization_url.to_string(),
    state: state.secret().clone(),
    nonce: nonce.secret().clone(),
    pkce_verifier: pkce_verifier.secret().clone(),
  })
}

/// Exchanges an authorization code for tokens and verifies the ID token, extracting the
/// identity claims
pub async fn exchange_code(
  metadata: CoreProviderMetadata,
  config: &OidcFlowConfig,
  http_client: &reqwest::Client,
  code: String,
  pkce_verifier: String,
  nonce: String,
) -> Result<SessionClaims, Box<dyn Error + Send + Sync>> {
  let client = build_client(metadata, config)?;
  let oidc_http_client = OidcHttpClient::new(http_client.clone());
  let token_response = client
    .exchange_code(AuthorizationCode::new(code))
    .map_err(|e| anyhow::anyhow!("The OIDC provider doesn't support the token endpoint: {e}"))?
    .set_pkce_verifier(PkceCodeVerifier::new(pkce_verifier))
    .request_async(&oidc_http_client)
    .await
    .map_err(|e| anyhow::anyhow!("The OIDC authorization code exchange failed: {e}"))?;

  let id_token = token_response
    .id_token()
    .ok_or_else(|| anyhow::anyhow!("The OIDC provider didn't return an ID token"))?;
  let mut id_token_verifier = client.id_token_verifier();
  if !config.audiences.is_empty() {
    id_token_verifier =
      id_token_verifier.set_other_audience_verifier_fn(|audience| config.audiences.iter().any(|a| a == &**audience));
  }
  let nonce = Nonce::new(nonce);
  let claims = id_token
    .claims(&id_token_verifier, &nonce)
    .map_err(|e| anyhow::anyhow!("The OIDC ID token verification failed: {e}"))?;

  // Verify the access token hash (if present) to ensure that the access token
  // hasn't been substituted for another user's
  if let Some(expected_access_token_hash) = claims.access_token_hash() {
    let actual_access_token_hash = AccessTokenHash::from_token(
      token_response.access_token(),
      id_token
        .signing_alg()
        .map_err(|e| anyhow::anyhow!("Cannot determine the OIDC ID token signing algorithm: {e}"))?,
      id_token
        .signing_key(&id_token_verifier)
        .map_err(|e| anyhow::anyhow!("Cannot determine the OIDC ID token signing key: {e}"))?,
    )
    .map_err(|e| anyhow::anyhow!("Cannot compute the OIDC access token hash: {e}"))?;
    if actual_access_token_hash != *expected_access_token_hash {
      Err(anyhow::anyhow!("Invalid OIDC access token hash"))?;
    }
  }

  Ok(SessionClaims {
    sub: claims.subject().to_string(),
    username: claims.preferred_username().map(|v| v.to_string()),
    email: claims.email().map(|v| v.to_string()),
    name: claims.name().and_then(|name| name.get(None)).map(|v| v.to_string()),
    groups: extract_groups(id_token),
  })
}

/// Extracts the non-standard "groups" claim from a verified ID token by decoding its payload.
/// This is safe, because the ID token signature has already been verified over this payload.
fn extract_groups<T: serde::Serialize>(id_token: &T) -> Vec<String> {
  let extract = || -> Option<Vec<String>> {
    let jwt = match serde_json::to_value(id_token).ok()? {
      serde_json::Value::String(jwt) => jwt,
      _ => return None,
    };
    let payload = jwt.split('.').nth(1)?;
    let decoded = BASE64_URL_SAFE_NO_PAD.decode(payload).ok()?;
    let payload_json: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
    Some(
      payload_json
        .get("groups")?
        .as_array()?
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect(),
    )
  };
  extract().unwrap_or_default()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_extract_groups() {
    let payload = serde_json::json!({"sub": "user", "groups": ["admins", "developers"]});
    let jwt = format!(
      "e30.{}.c2ln",
      BASE64_URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap())
    );
    assert_eq!(
      extract_groups(&jwt),
      vec!["admins".to_string(), "developers".to_string()]
    );
  }

  #[test]
  fn test_extract_groups_missing() {
    let payload = serde_json::json!({"sub": "user"});
    let jwt = format!(
      "e30.{}.c2ln",
      BASE64_URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap())
    );
    assert!(extract_groups(&jwt).is_empty());
  }
}
