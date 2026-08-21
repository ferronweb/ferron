use std::error::Error;

use base64::prelude::*;
use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::Sha256;

/// The sealed OIDC session cookie payload
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SessionPayload {
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
  /// The Unix timestamp at which the session was created
  pub iat: u64,
  /// The Unix timestamp at which the session expires
  pub exp: u64,
}

/// The sealed OIDC state cookie payload, used during the authorization code flow
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StatePayload {
  /// The OAuth2 state parameter
  pub state: String,
  /// The PKCE code verifier
  pub pkce_verifier: String,
  /// The OpenID Connect nonce
  pub nonce: String,
  /// The URL (path and query string) to redirect to after a successful login
  pub original_url: String,
  /// The Unix timestamp at which the state was created
  pub iat: u64,
}

/// Keys used to seal and unseal OIDC cookies, derived from the configured cookie secret
pub struct CookieKeys {
  session_key: [u8; 32],
  state_key: [u8; 32],
}

impl CookieKeys {
  /// Derives the cookie keys from a secret using HKDF-SHA256
  pub fn derive(secret: &[u8]) -> Self {
    let hkdf = Hkdf::<Sha256>::new(None, secret);
    let mut session_key = [0u8; 32];
    let mut state_key = [0u8; 32];
    // 32-byte outputs never exceed the HKDF-SHA256 output limit
    hkdf.expand(b"ferron-oidc-session", &mut session_key).unwrap();
    hkdf.expand(b"ferron-oidc-state", &mut state_key).unwrap();
    Self { session_key, state_key }
  }

  /// Seals the session cookie payload
  pub fn seal_session(&self, payload: &SessionPayload) -> Result<String, Box<dyn Error + Send + Sync>> {
    seal(&self.session_key, payload)
  }

  /// Unseals the session cookie payload
  pub fn unseal_session(&self, sealed: &str) -> Result<SessionPayload, Box<dyn Error + Send + Sync>> {
    unseal(&self.session_key, sealed)
  }

  /// Seals the state cookie payload
  pub fn seal_state(&self, payload: &StatePayload) -> Result<String, Box<dyn Error + Send + Sync>> {
    seal(&self.state_key, payload)
  }

  /// Unseals the state cookie payload
  pub fn unseal_state(&self, sealed: &str) -> Result<StatePayload, Box<dyn Error + Send + Sync>> {
    unseal(&self.state_key, sealed)
  }
}

/// Seals a payload into a URL-safe Base64 string (random 24-byte nonce followed by the ciphertext)
fn seal<T: Serialize>(key: &[u8; 32], payload: &T) -> Result<String, Box<dyn Error + Send + Sync>> {
  let cipher = XChaCha20Poly1305::new(key.into());
  let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
  let plaintext = serde_json::to_vec(payload)?;
  let ciphertext = cipher
    .encrypt(&nonce, plaintext.as_slice())
    .map_err(|_| anyhow::anyhow!("Failed to encrypt the cookie payload"))?;
  let mut sealed = Vec::with_capacity(nonce.len() + ciphertext.len());
  sealed.extend_from_slice(&nonce);
  sealed.extend_from_slice(&ciphertext);
  Ok(BASE64_URL_SAFE_NO_PAD.encode(sealed))
}

/// Unseals a payload sealed with the `seal` function
fn unseal<T: DeserializeOwned>(key: &[u8; 32], sealed: &str) -> Result<T, Box<dyn Error + Send + Sync>> {
  let sealed = BASE64_URL_SAFE_NO_PAD
    .decode(sealed)
    .map_err(|_| anyhow::anyhow!("Invalid cookie encoding"))?;
  if sealed.len() < 24 {
    Err(anyhow::anyhow!("Invalid cookie length"))?;
  }
  let (nonce, ciphertext) = sealed.split_at(24);
  let cipher = XChaCha20Poly1305::new(key.into());
  let plaintext = cipher
    .decrypt(XNonce::from_slice(nonce), ciphertext)
    .map_err(|_| anyhow::anyhow!("Failed to decrypt the cookie payload"))?;
  Ok(serde_json::from_slice(&plaintext)?)
}

/// Extracts a cookie value from `Cookie` request header values
pub fn get_cookie<'a>(cookie_headers: impl Iterator<Item = &'a [u8]>, cookie_name: &str) -> Option<String> {
  for header_value in cookie_headers {
    let header_value = std::str::from_utf8(header_value).ok()?;
    for cookie in header_value.split(';') {
      let cookie = cookie.trim();
      if let Some((name, value)) = cookie.split_once('=') {
        if name == cookie_name {
          return Some(value.to_string());
        }
      }
    }
  }
  None
}

/// Builds a `Set-Cookie` header value setting a cookie
pub fn set_cookie_header(cookie_name: &str, value: &str, max_age: u64, secure: bool) -> String {
  format!(
    "{cookie_name}={value}; Max-Age={max_age}; Path=/; HttpOnly; SameSite=Lax{}",
    if secure { "; Secure" } else { "" }
  )
}

/// Builds a `Set-Cookie` header value expiring a cookie
pub fn expire_cookie_header(cookie_name: &str, secure: bool) -> String {
  format!(
    "{cookie_name}=; Max-Age=0; Path=/; HttpOnly; SameSite=Lax{}",
    if secure { "; Secure" } else { "" }
  )
}

/// Sanitizes a post-login or post-logout redirect target, allowing only local absolute paths
pub fn sanitize_redirect_target(target: &str) -> &str {
  if target.starts_with('/') && !target.starts_with("//") && !target.starts_with("/\\") {
    target
  } else {
    "/"
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn keys() -> CookieKeys {
    CookieKeys::derive(b"a test secret that is long enough")
  }

  fn session_payload() -> SessionPayload {
    SessionPayload {
      sub: "user-1".to_string(),
      username: Some("someone".to_string()),
      email: Some("someone@example.com".to_string()),
      name: None,
      groups: vec!["admins".to_string()],
      iat: 1000,
      exp: 2000,
    }
  }

  #[test]
  fn test_session_roundtrip() {
    let keys = keys();
    let payload = session_payload();
    let sealed = keys.seal_session(&payload).unwrap();
    assert_eq!(keys.unseal_session(&sealed).unwrap(), payload);
  }

  #[test]
  fn test_state_roundtrip() {
    let keys = keys();
    let payload = StatePayload {
      state: "state".to_string(),
      pkce_verifier: "verifier".to_string(),
      nonce: "nonce".to_string(),
      original_url: "/protected?x=1".to_string(),
      iat: 1000,
    };
    let sealed = keys.seal_state(&payload).unwrap();
    assert_eq!(keys.unseal_state(&sealed).unwrap(), payload);
  }

  #[test]
  fn test_tampered_cookie_rejected() {
    let keys = keys();
    let sealed = keys.seal_session(&session_payload()).unwrap();
    let mut tampered = sealed.into_bytes();
    let last = tampered.len() - 1;
    tampered[last] = if tampered[last] == b'A' { b'B' } else { b'A' };
    assert!(keys.unseal_session(&String::from_utf8(tampered).unwrap()).is_err());
  }

  #[test]
  fn test_wrong_key_rejected() {
    let sealed = keys().seal_session(&session_payload()).unwrap();
    let other_keys = CookieKeys::derive(b"a different secret that is long enough");
    assert!(other_keys.unseal_session(&sealed).is_err());
  }

  #[test]
  fn test_session_cookie_not_valid_as_state_cookie() {
    let keys = keys();
    let sealed = keys.seal_session(&session_payload()).unwrap();
    assert!(keys.unseal_state(&sealed).is_err());
  }

  #[test]
  fn test_get_cookie() {
    let headers: Vec<&[u8]> = vec![b"foo=bar; ferron_oidc_session=abc123; baz=qux"];
    assert_eq!(
      get_cookie(headers.clone().into_iter(), "ferron_oidc_session"),
      Some("abc123".to_string())
    );
    assert_eq!(get_cookie(headers.into_iter(), "missing"), None);
  }

  #[test]
  fn test_set_cookie_headers() {
    assert_eq!(
      set_cookie_header("c", "v", 3600, true),
      "c=v; Max-Age=3600; Path=/; HttpOnly; SameSite=Lax; Secure"
    );
    assert_eq!(
      expire_cookie_header("c", false),
      "c=; Max-Age=0; Path=/; HttpOnly; SameSite=Lax"
    );
  }

  #[test]
  fn test_sanitize_redirect_target() {
    assert_eq!(sanitize_redirect_target("/some/path?a=b"), "/some/path?a=b");
    assert_eq!(sanitize_redirect_target("//evil.example.com"), "/");
    assert_eq!(sanitize_redirect_target("/\\evil.example.com"), "/");
    assert_eq!(sanitize_redirect_target("https://evil.example.com"), "/");
    assert_eq!(sanitize_redirect_target(""), "/");
  }
}
