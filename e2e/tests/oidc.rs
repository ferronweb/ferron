#[cfg(unix)]
use std::{fs::Permissions, os::unix::fs::PermissionsExt};
use std::{io::Write, path::Path};

use testcontainers::{
  ContainerAsync, GenericImage, ImageExt, TestcontainersError,
  core::{ContainerPort, Mount, WaitFor, wait::HttpWaitStrategy},
  runners::AsyncRunner,
};

mod common;

// 32 bytes, Base64-encoded
const COOKIE_SECRET: &str = "QUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUE=";

async fn create_mock_idp_container(network: &str) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
  GenericImage::new("ghcr.io/navikt/mock-oauth2-server", "2.1.10")
    .with_exposed_port(ContainerPort::Tcp(8080))
    .with_wait_for(WaitFor::Http(Box::new(
      HttpWaitStrategy::new("/default/.well-known/openid-configuration")
        .with_port(ContainerPort::Tcp(8080))
        .with_response_matcher(|response| response.status().is_success()),
    )))
    // Non-interactive login: the authorization endpoint redirects back with a code immediately
    .with_env_var("JSON_CONFIG", r#"{"interactiveLogin": false}"#)
    .with_network(network)
    .with_hostname("mockidp")
    .start()
    .await
}

async fn create_backend_container(network: &str) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
  let backend_image = self::common::build_backend_image().await?;
  backend_image
    .with_exposed_port(ContainerPort::Tcp(3000))
    .with_wait_for(WaitFor::Http(Box::new(
      HttpWaitStrategy::new("/")
        .with_port(ContainerPort::Tcp(3000))
        .with_response_matcher(|_| true),
    )))
    .with_network(network)
    .with_hostname("backend")
    .start()
    .await
}

async fn create_ferron_container(
  network: &str,
  config_file: &Path,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
  let ferron_image = self::common::build_ferron_image().await?;
  ferron_image
    .with_exposed_port(ContainerPort::Tcp(80))
    .with_wait_for(WaitFor::Http(Box::new(
      // The header makes Ferron respond with 401 instead of redirecting to the OIDC
      // provider, which the wait strategy's HTTP client would try to follow.
      HttpWaitStrategy::new("/")
        .with_port(ContainerPort::Tcp(80))
        .with_header(
          "x-requested-with",
          reqwest::header::HeaderValue::from_static("XMLHttpRequest"),
        )
        .with_response_matcher(|response| response.status() == reqwest::StatusCode::UNAUTHORIZED),
    )))
    .with_network(network)
    .with_hostname("ferron")
    .with_mount(Mount::bind_mount(
      config_file.to_string_lossy().to_string(),
      "/etc/ferron.kdl",
    ))
    .start()
    .await
}

struct OidcTestContext {
  _mock_idp: ContainerAsync<GenericImage>,
  _backend: ContainerAsync<GenericImage>,
  _ferron: ContainerAsync<GenericImage>,
  base_url: String,
  idp_base_url: String,
  client: reqwest::Client,
  _config_file: tempfile::NamedTempFile,
}

async fn setup(test_name: &str) -> OidcTestContext {
  let _ = rustls::crypto::ring::default_provider().install_default();

  let network = format!("e2e-test-oidc-{}", test_name);

  let mock_idp = create_mock_idp_container(&network).await.unwrap();
  let backend = create_backend_container(&network).await.unwrap();

  #[cfg(unix)]
  nix::sys::stat::umask(nix::sys::stat::Mode::from_bits(0o000).unwrap());

  #[cfg(unix)]
  let mut config_file = tempfile::Builder::new()
    .permissions(Permissions::from_mode(0o666))
    .tempfile()
    .unwrap();
  #[cfg(not(unix))]
  let mut config_file = tempfile::NamedTempFile::new().unwrap();

  config_file
    .as_file_mut()
    .write_all(
      format!(
        r#"
:80 {{
  auth_oidc "http://mockidp:8080/default"
  auth_oidc_client_id "ferron-e2e"
  auth_oidc_client_secret "e2e-secret"
  auth_oidc_cookie_secret "{COOKIE_SECRET}"
  auth_oidc_logout_path "/oidc-logout"

  proxy "http://backend:3000"
}}
"#
      )
      .as_bytes(),
    )
    .unwrap();

  let ferron = create_ferron_container(&network, config_file.path()).await.unwrap();

  let ferron_port = ferron.get_host_port_ipv4(ContainerPort::Tcp(80)).await.unwrap();
  let idp_port = mock_idp.get_host_port_ipv4(ContainerPort::Tcp(8080)).await.unwrap();

  let client = reqwest::Client::builder()
    .redirect(reqwest::redirect::Policy::none())
    .build()
    .unwrap();

  OidcTestContext {
    _mock_idp: mock_idp,
    _backend: backend,
    _ferron: ferron,
    base_url: format!("http://127.0.0.1:{}", ferron_port),
    idp_base_url: format!("http://127.0.0.1:{}", idp_port),
    client,
    _config_file: config_file,
  }
}

/// Extracts a cookie value from the `Set-Cookie` response headers
fn get_set_cookie(response: &reqwest::Response, cookie_name: &str) -> Option<String> {
  response
    .headers()
    .get_all(reqwest::header::SET_COOKIE)
    .iter()
    .filter_map(|v| v.to_str().ok())
    .find_map(|v| {
      let (name_value, _) = v.split_once(';')?;
      let (name, value) = name_value.split_once('=')?;
      (name == cookie_name).then(|| value.to_string())
    })
}

#[tokio::test]
async fn test_oidc_full_flow() {
  let ctx = setup("full-flow").await;

  // Step 1: an unauthenticated navigation request is redirected to the OIDC provider
  let response = ctx.client.get(format!("{}/", ctx.base_url)).send().await.unwrap();
  assert_eq!(response.status(), reqwest::StatusCode::FOUND);
  let authorization_url = response
    .headers()
    .get(reqwest::header::LOCATION)
    .unwrap()
    .to_str()
    .unwrap()
    .to_string();
  assert!(
    authorization_url.starts_with("http://mockidp:8080/default/authorize"),
    "unexpected authorization URL: {authorization_url}"
  );
  let state_cookie = get_set_cookie(&response, "ferron_oidc_session_state").expect("state cookie not set");

  // Step 2: the OIDC provider (reached through its host-mapped port) redirects back with a code
  let authorization_url_from_host = authorization_url.replace("http://mockidp:8080", &ctx.idp_base_url);
  let response = ctx.client.get(authorization_url_from_host).send().await.unwrap();
  assert_eq!(response.status(), reqwest::StatusCode::FOUND);
  let callback_url = response
    .headers()
    .get(reqwest::header::LOCATION)
    .unwrap()
    .to_str()
    .unwrap()
    .to_string();
  assert!(
    callback_url.contains("/.ferron/oidc/callback"),
    "unexpected callback URL: {callback_url}"
  );
  assert!(callback_url.contains("code="), "no authorization code: {callback_url}");

  // Step 3: the callback exchanges the code and sets the session cookie
  let response = ctx
    .client
    .get(&callback_url)
    .header(
      reqwest::header::COOKIE,
      format!("ferron_oidc_session_state={state_cookie}"),
    )
    .send()
    .await
    .unwrap();
  assert_eq!(response.status(), reqwest::StatusCode::FOUND);
  assert_eq!(
    response
      .headers()
      .get(reqwest::header::LOCATION)
      .unwrap()
      .to_str()
      .unwrap(),
    "/"
  );
  let session_cookie = get_set_cookie(&response, "ferron_oidc_session").expect("session cookie not set");
  assert!(!session_cookie.is_empty());

  // Step 4: the session cookie authenticates the request, and the backend
  // receives the Remote-User header
  let response = ctx
    .client
    .get(format!("{}/remote-user", ctx.base_url))
    .header(reqwest::header::COOKIE, format!("ferron_oidc_session={session_cookie}"))
    .send()
    .await
    .unwrap();
  assert_eq!(response.status(), reqwest::StatusCode::OK);
  let remote_user = response.text().await.unwrap();
  assert!(!remote_user.is_empty(), "no Remote-User header received by the backend");

  // Step 5: logging out expires the session cookie
  let response = ctx
    .client
    .get(format!("{}/oidc-logout", ctx.base_url))
    .header(reqwest::header::COOKIE, format!("ferron_oidc_session={session_cookie}"))
    .send()
    .await
    .unwrap();
  assert_eq!(response.status(), reqwest::StatusCode::FOUND);
  let expired_cookie = get_set_cookie(&response, "ferron_oidc_session").expect("session cookie not expired");
  assert!(expired_cookie.is_empty());
}

#[tokio::test]
async fn test_oidc_invalid_session_redirects_to_login() {
  let ctx = setup("invalid-session").await;

  let response = ctx
    .client
    .get(format!("{}/", ctx.base_url))
    .header(reqwest::header::COOKIE, "ferron_oidc_session=garbage")
    .send()
    .await
    .unwrap();
  assert_eq!(response.status(), reqwest::StatusCode::FOUND);
  let location = response
    .headers()
    .get(reqwest::header::LOCATION)
    .unwrap()
    .to_str()
    .unwrap();
  assert!(location.starts_with("http://mockidp:8080/default/authorize"));
}

#[tokio::test]
async fn test_oidc_api_request_unauthorized() {
  let ctx = setup("api-request").await;

  let response = ctx
    .client
    .get(format!("{}/", ctx.base_url))
    .header("x-requested-with", "XMLHttpRequest")
    .send()
    .await
    .unwrap();
  assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_oidc_callback_without_state_rejected() {
  let ctx = setup("callback-no-state").await;

  let response = ctx
    .client
    .get(format!("{}/.ferron/oidc/callback?code=x&state=y", ctx.base_url))
    .send()
    .await
    .unwrap();
  assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
}
