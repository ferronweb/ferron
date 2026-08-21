use std::error::Error;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use base64::prelude::*;
use bytes::Bytes;
use chacha20poly1305::aead::{KeyInit, OsRng};
use chacha20poly1305::XChaCha20Poly1305;
use ferron_common::logging::ErrorLogger;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty};
use hyper::header::{HeaderName, HeaderValue};
use hyper::{header, Request, Response, StatusCode};

use ferron_common::modules::{Module, ModuleHandlers, ModuleLoader, RequestData, ResponseData, SocketData};
use ferron_common::{config::ServerConfiguration, util::ModuleCache};
use ferron_common::{get_entries_for_validation, get_value, get_values};

use crate::util::oidc::cookie::{
  expire_cookie_header, get_cookie, sanitize_redirect_target, set_cookie_header, CookieKeys, SessionPayload,
  StatePayload,
};
use crate::util::oidc::flow::{begin_login, exchange_code, OidcFlowConfig, SessionClaims};
use crate::util::oidc::http_client::build_http_client;
use crate::util::oidc::provider::ProviderCache;

/// The default OIDC session time-to-live (in seconds)
const DEFAULT_SESSION_TTL: u64 = 86400;

/// The time-to-live of the OIDC state cookie (in seconds)
const STATE_TTL: u64 = 600;

/// The default OIDC session cookie name
const DEFAULT_COOKIE_NAME: &str = "ferron_oidc_session";

/// The default OIDC redirect (callback) path
const DEFAULT_REDIRECT_PATH: &str = "/.ferron/oidc/callback";

/// Identity request headers, stripped from incoming requests and injected for
/// authenticated users (compatible with the headers used by Authelia)
const IDENTITY_HEADERS: [HeaderName; 4] = [
  HeaderName::from_static("remote-user"),
  HeaderName::from_static("remote-groups"),
  HeaderName::from_static("remote-email"),
  HeaderName::from_static("remote-name"),
];

/// An OIDC authentication module loader
pub struct OidcModuleLoader {
  cache: ModuleCache<OidcModule>,
}

impl Default for OidcModuleLoader {
  fn default() -> Self {
    Self::new()
  }
}

impl OidcModuleLoader {
  /// Creates a new module loader
  pub fn new() -> Self {
    Self {
      cache: ModuleCache::new(vec![
        "auth_oidc",
        "auth_oidc_client_id",
        "auth_oidc_client_secret",
        "auth_oidc_scopes",
        "auth_oidc_cookie_secret",
        "auth_oidc_cookie_name",
        "auth_oidc_session_ttl",
        "auth_oidc_redirect_path",
        "auth_oidc_logout_path",
        "auth_oidc_post_logout_redirect",
        "auth_oidc_headers",
        "auth_oidc_user_claim",
        "auth_oidc_allowed_groups",
        "auth_oidc_audience",
        "auth_oidc_no_verification",
      ]),
    }
  }
}

impl ModuleLoader for OidcModuleLoader {
  fn load_module(
    &mut self,
    config: &ServerConfiguration,
    _global_config: Option<&ServerConfiguration>,
    secondary_runtime: &tokio::runtime::Runtime,
  ) -> Result<Arc<dyn Module + Send + Sync>, Box<dyn Error + Send + Sync>> {
    let runtime_handle = secondary_runtime.handle().to_owned();
    Ok(
      self
        .cache
        .get_or_init::<_, Box<dyn std::error::Error + Send + Sync>>(config, move |config| {
          let issuer_url = get_value!("auth_oidc", config)
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("The OIDC issuer URL is not specified"))?
            .to_string();
          let client_id = get_value!("auth_oidc_client_id", config)
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("The OIDC client ID is not specified"))?
            .to_string();
          let client_secret = get_value!("auth_oidc_client_secret", config)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
          let scopes = {
            let scopes = get_values!("auth_oidc_scopes", config)
              .into_iter()
              .filter_map(|v| v.as_str().map(|s| s.to_string()))
              .collect::<Vec<_>>();
            if scopes.is_empty() {
              vec!["openid".to_string(), "profile".to_string(), "email".to_string()]
            } else {
              scopes
            }
          };
          let cookie_keys = match get_value!("auth_oidc_cookie_secret", config).and_then(|v| v.as_str()) {
            Some(secret) => {
              let secret = BASE64_STANDARD
                .decode(secret)
                .map_err(|_| anyhow::anyhow!("The OIDC cookie secret is not valid Base64"))?;
              if secret.len() < 32 {
                Err(anyhow::anyhow!(
                  "The OIDC cookie secret must be at least 32 bytes long after decoding"
                ))?;
              }
              CookieKeys::derive(&secret)
            }
            None => {
              // Without a configured cookie secret, sessions don't survive server restarts
              // and can't be shared between multiple server instances.
              CookieKeys::derive(&XChaCha20Poly1305::generate_key(&mut OsRng))
            }
          };
          let cookie_name = get_value!("auth_oidc_cookie_name", config)
            .and_then(|v| v.as_str())
            .unwrap_or(DEFAULT_COOKIE_NAME)
            .to_string();
          let session_ttl = get_value!("auth_oidc_session_ttl", config)
            .and_then(|v| v.as_i128())
            .map(|v| v as u64)
            .unwrap_or(DEFAULT_SESSION_TTL);
          let redirect_path = get_value!("auth_oidc_redirect_path", config)
            .and_then(|v| v.as_str())
            .unwrap_or(DEFAULT_REDIRECT_PATH)
            .to_string();
          let logout_path = get_value!("auth_oidc_logout_path", config)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
          let post_logout_redirect = get_value!("auth_oidc_post_logout_redirect", config)
            .and_then(|v| v.as_str())
            .unwrap_or("/")
            .to_string();
          let inject_headers = get_value!("auth_oidc_headers", config)
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
          let user_claim = get_value!("auth_oidc_user_claim", config)
            .and_then(|v| v.as_str())
            .unwrap_or("preferred_username")
            .to_string();
          let allowed_groups = get_values!("auth_oidc_allowed_groups", config)
            .into_iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect::<Vec<_>>();
          let audiences = get_values!("auth_oidc_audience", config)
            .into_iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect::<Vec<_>>();
          let no_verification = get_value!("auth_oidc_no_verification", config)
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

          let http_client = build_http_client(no_verification)?;

          Ok(Arc::new(OidcModule {
            inner: Arc::new(OidcModuleInner {
              issuer_url,
              client_id,
              client_secret,
              scopes,
              cookie_keys,
              cookie_name,
              session_ttl,
              redirect_path,
              logout_path,
              post_logout_redirect,
              inject_headers,
              user_claim,
              allowed_groups,
              audiences,
              http_client,
              provider_cache: ProviderCache::new(),
              runtime_handle,
            }),
          }))
        })?,
    )
  }

  fn get_requirements(&self) -> Vec<&'static str> {
    vec!["auth_oidc"]
  }

  fn validate_configuration(
    &self,
    config: &ServerConfiguration,
    used_properties: &mut std::collections::HashSet<String>,
  ) -> Result<(), Box<dyn Error + Send + Sync>> {
    macro_rules! validate_single_string {
      ($name:literal, $description:literal) => {
        if let Some(entries) = get_entries_for_validation!($name, config, used_properties) {
          for entry in &entries.inner {
            if entry.values.len() != 1 {
              Err(anyhow::anyhow!(concat!(
                "The `",
                $name,
                "` configuration property must have exactly one value"
              )))?
            } else if !entry.values[0].is_string() {
              Err(anyhow::anyhow!(concat!("Invalid ", $description)))?
            }
          }
        }
      };
    }
    validate_single_string!("auth_oidc", "OIDC issuer URL");
    validate_single_string!("auth_oidc_client_id", "OIDC client ID");
    validate_single_string!("auth_oidc_client_secret", "OIDC client secret");
    validate_single_string!("auth_oidc_cookie_secret", "OIDC cookie secret");
    validate_single_string!("auth_oidc_cookie_name", "OIDC cookie name");
    validate_single_string!("auth_oidc_user_claim", "OIDC user claim");
    validate_single_string!("auth_oidc_post_logout_redirect", "OIDC post-logout redirect target");

    macro_rules! validate_path {
      ($name:literal, $description:literal, $nullable:literal) => {
        if let Some(entries) = get_entries_for_validation!($name, config, used_properties) {
          for entry in &entries.inner {
            if entry.values.len() != 1 {
              Err(anyhow::anyhow!(concat!(
                "The `",
                $name,
                "` configuration property must have exactly one value"
              )))?
            } else if $nullable && entry.values[0].is_null() {
              continue;
            } else if !entry.values[0].as_str().is_some_and(|v| v.starts_with('/')) {
              Err(anyhow::anyhow!(concat!(
                "Invalid ",
                $description,
                "; it must begin with \"/\""
              )))?
            }
          }
        }
      };
    }
    validate_path!("auth_oidc_redirect_path", "OIDC redirect path", false);
    validate_path!("auth_oidc_logout_path", "OIDC logout path", true);

    macro_rules! validate_boolean {
      ($name:literal, $description:literal) => {
        if let Some(entries) = get_entries_for_validation!($name, config, used_properties) {
          for entry in &entries.inner {
            if entry.values.len() != 1 {
              Err(anyhow::anyhow!(concat!(
                "The `",
                $name,
                "` configuration property must have exactly one value"
              )))?
            } else if !entry.values[0].is_bool() {
              Err(anyhow::anyhow!(concat!("Invalid ", $description)))?
            }
          }
        }
      };
    }
    validate_boolean!("auth_oidc_headers", "OIDC identity header injection option");
    validate_boolean!(
      "auth_oidc_no_verification",
      "OIDC provider certificate verification option"
    );

    macro_rules! validate_string_list {
      ($name:literal, $description:literal) => {
        if let Some(entries) = get_entries_for_validation!($name, config, used_properties) {
          for entry in &entries.inner {
            for value in &entry.values {
              if !value.is_string() {
                Err(anyhow::anyhow!(concat!("Invalid ", $description)))?
              }
            }
          }
        }
      };
    }
    validate_string_list!("auth_oidc_scopes", "OIDC scopes");
    validate_string_list!("auth_oidc_allowed_groups", "OIDC allowed groups");
    validate_string_list!("auth_oidc_audience", "OIDC audiences");

    if let Some(entries) = get_entries_for_validation!("auth_oidc_session_ttl", config, used_properties) {
      for entry in &entries.inner {
        if entry.values.len() != 1 {
          Err(anyhow::anyhow!(
            "The `auth_oidc_session_ttl` configuration property must have exactly one value"
          ))?
        } else if !entry.values[0].is_integer() || entry.values[0].as_i128().is_some_and(|v| v < 1) {
          Err(anyhow::anyhow!("Invalid OIDC session time-to-live"))?
        }
      }
    }

    Ok(())
  }
}

/// The OIDC authentication module configuration and shared state
struct OidcModuleInner {
  issuer_url: String,
  client_id: String,
  client_secret: Option<String>,
  scopes: Vec<String>,
  cookie_keys: CookieKeys,
  cookie_name: String,
  session_ttl: u64,
  redirect_path: String,
  logout_path: Option<String>,
  post_logout_redirect: String,
  inject_headers: bool,
  user_claim: String,
  allowed_groups: Vec<String>,
  audiences: Vec<String>,
  http_client: reqwest::Client,
  provider_cache: ProviderCache,
  runtime_handle: tokio::runtime::Handle,
}

impl OidcModuleInner {
  /// The name of the OIDC state cookie
  fn state_cookie_name(&self) -> String {
    format!("{}_state", self.cookie_name)
  }

  /// Constructs the OIDC flow configuration for the specified redirect URL
  fn flow_config(&self, redirect_url: String) -> OidcFlowConfig {
    OidcFlowConfig {
      client_id: self.client_id.clone(),
      client_secret: self.client_secret.clone(),
      scopes: self.scopes.clone(),
      redirect_url,
      audiences: self.audiences.clone(),
    }
  }

  /// Determines the authenticated username from the identity claims
  fn pick_user(&self, username: Option<&str>, email: Option<&str>, sub: &str) -> String {
    match &self.user_claim as &str {
      "email" => email.or(username),
      "sub" => None,
      _ => username.or(email),
    }
    .unwrap_or(sub)
    .to_string()
  }
}

/// An OIDC authentication module
struct OidcModule {
  inner: Arc<OidcModuleInner>,
}

impl Module for OidcModule {
  fn get_module_handlers(&self) -> Box<dyn ModuleHandlers> {
    Box::new(OidcModuleHandlers {
      inner: self.inner.clone(),
    })
  }
}

/// OIDC authentication module handlers
struct OidcModuleHandlers {
  inner: Arc<OidcModuleInner>,
}

#[async_trait(?Send)]
impl ModuleHandlers for OidcModuleHandlers {
  async fn request_handler(
    &mut self,
    mut request: Request<BoxBody<Bytes, std::io::Error>>,
    _config: &ServerConfiguration,
    socket_data: &SocketData,
    error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    let inner = &self.inner;
    let secure = socket_data.encrypted;

    // Always strip client-supplied identity headers
    for identity_header in IDENTITY_HEADERS.iter() {
      while request.headers_mut().remove(identity_header).is_some() {}
    }

    let request_path = request.uri().path().to_string();

    // Local logout: expire the session cookie and redirect
    if inner.logout_path.as_deref() == Some(&request_path) {
      return Ok(short_circuit(redirect_response(
        sanitize_redirect_target(&inner.post_logout_redirect),
        vec![expire_cookie_header(&inner.cookie_name, secure)],
      )?));
    }

    // OIDC callback: exchange the authorization code for a verified session
    if request_path == inner.redirect_path {
      return self.handle_callback(request, secure, error_logger).await;
    }

    // Check for an existing session
    let session_cookie = get_cookie(
      request.headers().get_all(header::COOKIE).iter().map(|v| v.as_bytes()),
      &inner.cookie_name,
    );
    if let Some(session_cookie) = session_cookie {
      if let Ok(session) = inner.cookie_keys.unseal_session(&session_cookie) {
        if session.exp > unix_timestamp() {
          return self.handle_authenticated(request, session);
        }
      }
    }

    // Unauthenticated: redirect navigation requests to the OIDC provider, and
    // respond with 401 Unauthorized to API-style requests
    let is_navigation = (request.method() == hyper::Method::GET || request.method() == hyper::Method::HEAD)
      && request
        .headers()
        .get("sec-fetch-mode")
        .is_none_or(|v| v.as_bytes() != b"cors")
      && !request.headers().contains_key("x-requested-with");
    if !is_navigation {
      return Ok(status_response(StatusCode::UNAUTHORIZED, None));
    }

    self.handle_login_redirect(request, secure, error_logger).await
  }
}

impl OidcModuleHandlers {
  /// Redirects an unauthenticated request to the OIDC provider's authorization endpoint
  async fn handle_login_redirect(
    &self,
    request: Request<BoxBody<Bytes, std::io::Error>>,
    secure: bool,
    error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    let inner = self.inner.clone();
    let redirect_url = match construct_redirect_url(&request, secure, &inner.redirect_path) {
      Some(redirect_url) => redirect_url,
      None => return Ok(status_response(StatusCode::BAD_REQUEST, None)),
    };
    let original_url = format!(
      "{}{}",
      request.uri().path(),
      match request.uri().query() {
        Some(query) => format!("?{query}"),
        None => String::new(),
      }
    );

    let inner_spawned = inner.clone();
    let begin = inner
      .runtime_handle
      .spawn(async move {
        let metadata = inner_spawned
          .provider_cache
          .get_metadata(&inner_spawned.http_client, &inner_spawned.issuer_url)
          .await?;
        begin_login(metadata, &inner_spawned.flow_config(redirect_url))
      })
      .await?;
    let begin = match begin {
      Ok(begin) => begin,
      Err(e) => {
        error_logger.log(&format!("OIDC login initiation failed: {e}")).await;
        return Ok(status_response(StatusCode::SERVICE_UNAVAILABLE, None));
      }
    };

    let state_payload = StatePayload {
      state: begin.state,
      pkce_verifier: begin.pkce_verifier,
      nonce: begin.nonce,
      original_url,
      iat: unix_timestamp(),
    };
    let state_cookie = inner.cookie_keys.seal_state(&state_payload)?;

    Ok(short_circuit(redirect_response(
      &begin.authorization_url,
      vec![set_cookie_header(
        &inner.state_cookie_name(),
        &state_cookie,
        STATE_TTL,
        secure,
      )],
    )?))
  }

  /// Handles the OIDC callback request
  async fn handle_callback(
    &self,
    request: Request<BoxBody<Bytes, std::io::Error>>,
    secure: bool,
    error_logger: &ErrorLogger,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    let inner = self.inner.clone();
    let expire_state = vec![expire_cookie_header(&inner.state_cookie_name(), secure)];

    let query_params = parse_query(request.uri().query().unwrap_or(""));
    let state_param = query_params.iter().find(|(k, _)| k == "state").map(|(_, v)| v.clone());
    let code_param = query_params.iter().find(|(k, _)| k == "code").map(|(_, v)| v.clone());
    let error_param = query_params.iter().find(|(k, _)| k == "error").map(|(_, v)| v.clone());

    if let Some(error_param) = error_param {
      error_logger
        .log(&format!("The OIDC provider returned an error: {error_param}"))
        .await;
      return Ok(status_response(StatusCode::FORBIDDEN, Some(expire_state)));
    }

    let state_cookie = get_cookie(
      request.headers().get_all(header::COOKIE).iter().map(|v| v.as_bytes()),
      &inner.state_cookie_name(),
    );
    let state_payload = match state_cookie.and_then(|c| inner.cookie_keys.unseal_state(&c).ok()) {
      Some(state_payload) => state_payload,
      None => {
        error_logger
          .log("OIDC callback request without a valid state cookie")
          .await;
        return Ok(status_response(StatusCode::BAD_REQUEST, Some(expire_state)));
      }
    };
    if state_payload.iat + STATE_TTL < unix_timestamp() || state_param.as_deref() != Some(&state_payload.state as &str)
    {
      error_logger
        .log("OIDC callback request with an expired or mismatched state")
        .await;
      return Ok(status_response(StatusCode::BAD_REQUEST, Some(expire_state)));
    }
    let code = match code_param {
      Some(code) => code,
      None => {
        return Ok(status_response(StatusCode::BAD_REQUEST, Some(expire_state)));
      }
    };

    let redirect_url = match construct_redirect_url(&request, secure, &inner.redirect_path) {
      Some(redirect_url) => redirect_url,
      None => return Ok(status_response(StatusCode::BAD_REQUEST, Some(expire_state))),
    };

    let inner_spawned = inner.clone();
    let pkce_verifier = state_payload.pkce_verifier.clone();
    let nonce = state_payload.nonce.clone();
    let claims = inner
      .runtime_handle
      .spawn(async move {
        let metadata = inner_spawned
          .provider_cache
          .get_metadata(&inner_spawned.http_client, &inner_spawned.issuer_url)
          .await?;
        let flow_config = inner_spawned.flow_config(redirect_url);
        let result = exchange_code(
          metadata,
          &flow_config,
          &inner_spawned.http_client,
          code,
          pkce_verifier,
          nonce,
        )
        .await;
        if result.is_err() {
          // The failure might be caused by rotated OIDC provider signing keys;
          // refresh the cached metadata, so that the next login attempt succeeds.
          let _ = inner_spawned
            .provider_cache
            .refresh_metadata(&inner_spawned.http_client, &inner_spawned.issuer_url)
            .await;
        }
        result
      })
      .await?;
    let claims: SessionClaims = match claims {
      Ok(claims) => claims,
      Err(e) => {
        error_logger.log(&format!("OIDC authentication failed: {e}")).await;
        return Ok(status_response(StatusCode::UNAUTHORIZED, Some(expire_state)));
      }
    };

    let now = unix_timestamp();
    let session_payload = SessionPayload {
      sub: claims.sub,
      username: claims.username,
      email: claims.email,
      name: claims.name,
      groups: claims.groups,
      iat: now,
      exp: now + inner.session_ttl,
    };
    let session_cookie = inner.cookie_keys.seal_session(&session_payload)?;

    Ok(short_circuit(redirect_response(
      sanitize_redirect_target(&state_payload.original_url),
      vec![
        set_cookie_header(&inner.cookie_name, &session_cookie, inner.session_ttl, secure),
        expire_cookie_header(&inner.state_cookie_name(), secure),
      ],
    )?))
  }

  /// Handles a request with a valid session
  fn handle_authenticated(
    &self,
    mut request: Request<BoxBody<Bytes, std::io::Error>>,
    session: SessionPayload,
  ) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
    let inner = &self.inner;

    if !inner.allowed_groups.is_empty() && !session.groups.iter().any(|group| inner.allowed_groups.contains(group)) {
      return Ok(status_response(StatusCode::FORBIDDEN, None));
    }

    let auth_user = inner.pick_user(session.username.as_deref(), session.email.as_deref(), &session.sub);
    if let Some(request_data) = request.extensions_mut().get_mut::<RequestData>() {
      request_data.auth_user = Some(auth_user.clone());
    }

    if inner.inject_headers {
      let headers = request.headers_mut();
      headers.insert(&IDENTITY_HEADERS[0], HeaderValue::from_str(&auth_user)?);
      if !session.groups.is_empty() {
        headers.insert(&IDENTITY_HEADERS[1], HeaderValue::from_str(&session.groups.join(","))?);
      }
      if let Some(email) = &session.email {
        if let Ok(header_value) = HeaderValue::from_str(email) {
          headers.insert(&IDENTITY_HEADERS[2], header_value);
        }
      }
      if let Some(name) = &session.name {
        if let Ok(header_value) = HeaderValue::from_str(name) {
          headers.insert(&IDENTITY_HEADERS[3], header_value);
        }
      }
    }

    Ok(ResponseData {
      request: Some(request),
      response: None,
      response_status: None,
      response_headers: None,
      new_remote_address: None,
    })
  }
}

/// The current Unix timestamp (in seconds)
fn unix_timestamp() -> u64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|d| d.as_secs())
    .unwrap_or(0)
}

/// Constructs the OIDC redirect URL from the request's host and the configured redirect path
fn construct_redirect_url(
  request: &Request<BoxBody<Bytes, std::io::Error>>,
  secure: bool,
  redirect_path: &str,
) -> Option<String> {
  let host = request
    .uri()
    .authority()
    .map(|authority| authority.to_string())
    .or_else(|| {
      request
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_string())
    })?;
  let scheme = if secure { "https" } else { "http" };
  Some(format!("{scheme}://{host}{redirect_path}"))
}

/// Parses a query string into key-value pairs
fn parse_query(query: &str) -> Vec<(String, String)> {
  query
    .split('&')
    .filter_map(|pair| {
      let (key, value) = pair.split_once('=')?;
      Some((
        urlencoding::decode(key).ok()?.into_owned(),
        urlencoding::decode(value).ok()?.into_owned(),
      ))
    })
    .collect()
}

/// Constructs a redirect response with the specified cookies
fn redirect_response(
  location: &str,
  set_cookies: Vec<String>,
) -> Result<Response<BoxBody<Bytes, std::io::Error>>, Box<dyn Error + Send + Sync>> {
  let mut builder = Response::builder()
    .status(StatusCode::FOUND)
    .header(header::LOCATION, location);
  for set_cookie in set_cookies {
    builder = builder.header(header::SET_COOKIE, set_cookie);
  }
  Ok(builder.body(Empty::new().map_err(|e| match e {}).boxed())?)
}

/// Constructs a response data structure short-circuiting with the specified response
fn short_circuit(response: Response<BoxBody<Bytes, std::io::Error>>) -> ResponseData {
  ResponseData {
    request: None,
    response: Some(response),
    response_status: None,
    response_headers: None,
    new_remote_address: None,
  }
}

/// Constructs a response data structure with a status code and optional cookies,
/// letting the error page handlers render the response body
fn status_response(status: StatusCode, set_cookies: Option<Vec<String>>) -> ResponseData {
  let response_headers = set_cookies.map(|set_cookies| {
    let mut headers = hyper::HeaderMap::new();
    for set_cookie in set_cookies {
      if let Ok(header_value) = HeaderValue::from_str(&set_cookie) {
        headers.append(header::SET_COOKIE, header_value);
      }
    }
    headers
  });
  ResponseData {
    request: None,
    response: None,
    response_status: Some(status),
    response_headers,
    new_remote_address: None,
  }
}
