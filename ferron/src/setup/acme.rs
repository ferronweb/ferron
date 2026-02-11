use std::{
  collections::{HashMap, HashSet},
  error::Error,
  path::PathBuf,
  str::FromStr,
  sync::Arc,
  time::Duration,
};

use base64::Engine;
use instant_acme::{ExternalAccountKey, LetsEncrypt};
use rustls::{client::WebPkiServerVerifier, crypto::CryptoProvider, ClientConfig};
use rustls_platform_verifier::BuilderVerifierExt;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use xxhash_rust::xxh3::xxh3_128;

use crate::acme::{
  add_domain_to_cache, convert_on_demand_config, provision_certificate, AcmeConfig, AcmeOnDemandConfig,
};
use ferron_common::{get_entry, get_value, util::match_hostname};
use ferron_common::{logging::ErrorLogger, util::NoServerVerifier};

/// Builds a Rustls client configuration for ACME.
pub fn build_rustls_client_config(
  server_configuration: &ferron_common::config::ServerConfiguration,
  crypto_provider: Arc<CryptoProvider>,
) -> Result<ClientConfig, Box<dyn Error + Send + Sync>> {
  build_raw_rustls_client_config(
    get_value!("auto_tls_no_verification", server_configuration)
      .and_then(|v| v.as_bool())
      .unwrap_or(false),
    crypto_provider,
  )
}

/// Builds a raw Rustls client configuration for ACME.
fn build_raw_rustls_client_config(
  no_verification: bool,
  crypto_provider: Arc<CryptoProvider>,
) -> Result<ClientConfig, Box<dyn Error + Send + Sync>> {
  Ok(
    (if no_verification {
      ClientConfig::builder_with_provider(crypto_provider.clone())
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoServerVerifier::new()))
    } else if let Ok(client_config) = BuilderVerifierExt::with_platform_verifier(
      ClientConfig::builder_with_provider(crypto_provider.clone()).with_safe_default_protocol_versions()?,
    ) {
      client_config
    } else {
      ClientConfig::builder_with_provider(crypto_provider.clone())
        .with_safe_default_protocol_versions()?
        .with_webpki_verifier(
          WebPkiServerVerifier::builder(Arc::new(rustls::RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
          }))
          .build()?,
        )
    })
    .with_no_client_auth(),
  )
}

/// Resolves the ACME directory URL based on the server configuration.
pub fn resolve_acme_directory(server_configuration: &ferron_common::config::ServerConfiguration) -> String {
  if let Some(directory) = get_value!("auto_tls_directory", server_configuration).and_then(|v| v.as_str()) {
    directory.to_string()
  } else if get_value!("auto_tls_letsencrypt_production", server_configuration)
    .and_then(|v| v.as_bool())
    .unwrap_or(true)
  {
    LetsEncrypt::Production.url().to_string()
  } else {
    LetsEncrypt::Staging.url().to_string()
  }
}

/// Parses the External Account Binding (EAB) key and secret from the server configuration.
pub fn parse_eab(
  server_configuration: &ferron_common::config::ServerConfiguration,
) -> Result<Option<Arc<ExternalAccountKey>>, anyhow::Error> {
  Ok(
    if let Some((Some(eab_key_id), Some(eab_key_hmac))) =
      get_entry!("auto_tls_eab", server_configuration).map(|entry| {
        (
          entry.values.first().and_then(|v| v.as_str()),
          entry.values.get(1).and_then(|v| v.as_str()),
        )
      })
    {
      match base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(eab_key_hmac.trim_end_matches('=')) {
        Ok(decoded_key) => Some(Arc::new(ExternalAccountKey::new(eab_key_id.to_string(), &decoded_key))),
        Err(err) => Err(anyhow::anyhow!("Failed to decode EAB key HMAC: {}", err))?,
      }
    } else {
      None
    },
  )
}

pub fn resolve_acme_cache_path(
  server_configuration: &ferron_common::config::ServerConfiguration,
) -> Result<Option<PathBuf>, anyhow::Error> {
  let acme_default_directory = dirs::data_local_dir().and_then(|mut p| {
    p.push("ferron-acme");
    p.into_os_string().into_string().ok()
  });
  Ok(
    if let Some(acme_cache_path_str) =
      get_value!("auto_tls_cache", server_configuration).map_or(acme_default_directory.as_deref(), |v| {
        if v.is_null() {
          None
        } else if let Some(v) = v.as_str() {
          Some(v)
        } else {
          acme_default_directory.as_deref()
        }
      })
    {
      Some(PathBuf::from_str(acme_cache_path_str).map_err(|_| anyhow::anyhow!("Invalid ACME cache path"))?)
    } else {
      None
    },
  )
}

/// Resolves the paths to account and certificate caches.
pub fn resolve_cache_paths(
  server_configuration: &ferron_common::config::ServerConfiguration,
  port: u16,
  sni_hostname: &str,
) -> Result<(Option<PathBuf>, Option<PathBuf>), anyhow::Error> {
  let acme_cache_path_option = resolve_acme_cache_path(server_configuration)?;
  let (account_cache_path, cert_cache_path) = if let Some(mut pathbuf) = acme_cache_path_option {
    let base_pathbuf = pathbuf.clone();
    let append_hash = base64::engine::general_purpose::URL_SAFE_NO_PAD
      .encode(xxh3_128(format!("{port}-{sni_hostname}").as_bytes()).to_be_bytes());
    pathbuf.push(append_hash);
    (Some(base_pathbuf), Some(pathbuf))
  } else {
    (None, None)
  };
  Ok((account_cache_path, cert_cache_path))
}

/// Performs background automatic TLS tasks.
#[allow(clippy::too_many_arguments)]
pub async fn background_acme_task(
  acme_configs: Vec<AcmeConfig>,
  acme_on_demand_configs: Vec<AcmeOnDemandConfig>,
  memory_acme_account_cache_data: Arc<RwLock<HashMap<String, Vec<u8>>>>,
  acme_on_demand_rx: async_channel::Receiver<(String, u16)>,
  on_demand_tls_ask_endpoint: Option<hyper::Uri>,
  on_demand_tls_ask_endpoint_verify: bool,
  acme_logger: ErrorLogger,
  crypto_provider: Arc<CryptoProvider>,
  existing_combinations: HashSet<(String, u16)>,
  cancel_token: Option<CancellationToken>,
) {
  let acme_logger = Arc::new(acme_logger);

  // Wrap ACME configurations in a mutex
  let acme_configs_mutex = Arc::new(tokio::sync::Mutex::new(acme_configs));

  let prevent_file_race_conditions_sem = Arc::new(tokio::sync::Semaphore::new(1));

  let acme_logger_clone = acme_logger.clone();
  let acme_configs_mutex_clone = acme_configs_mutex.clone();
  if !acme_on_demand_configs.is_empty() {
    let cancel_token_clone = cancel_token.clone();
    let cancelled_future = async move {
      if let Some(token) = cancel_token_clone {
        token.cancelled().await
      } else {
        futures_util::future::pending().await
      }
    };

    // On-demand TLS
    tokio::spawn(async move {
      tokio::select! {
        biased;

        _ = cancelled_future => {},
        _ = background_on_demand_acme_task(
          existing_combinations,
          acme_on_demand_rx,
          on_demand_tls_ask_endpoint,
          on_demand_tls_ask_endpoint_verify,
          acme_logger_clone,
          crypto_provider,
          acme_configs_mutex_clone,
          acme_on_demand_configs,
          memory_acme_account_cache_data,
          prevent_file_race_conditions_sem,
        ) => {}
      }
    });
  }

  let mut cancelled_future = Box::pin(async move {
    if let Some(token) = cancel_token {
      token.cancelled().await
    } else {
      futures_util::future::pending().await
    }
  });

  loop {
    for acme_config in &mut *(tokio::select! {
        biased;
        _ = &mut cancelled_future => {
            return;
        },
        result = acme_configs_mutex.lock() => result
    }) {
      if let Err(acme_error) = tokio::select! {
        biased;
        _ = &mut cancelled_future => {
            return;
        },
        result = provision_certificate(acme_config, &acme_logger) => result
      } {
        acme_logger
          .log(&format!("Error while obtaining a TLS certificate: {acme_error}"))
          .await
      }
    }
    tokio::time::sleep(Duration::from_secs(10)).await;
  }
}

/// Performs background automatic TLS on demand tasks.
#[allow(clippy::too_many_arguments)]
#[inline]
pub async fn background_on_demand_acme_task(
  existing_combinations: HashSet<(String, u16)>,
  acme_on_demand_rx: async_channel::Receiver<(String, u16)>,
  on_demand_tls_ask_endpoint: Option<hyper::Uri>,
  on_demand_tls_ask_endpoint_verify: bool,
  acme_logger: Arc<ErrorLogger>,
  crypto_provider: Arc<CryptoProvider>,
  acme_configs_mutex: Arc<tokio::sync::Mutex<Vec<AcmeConfig>>>,
  acme_on_demand_configs: Vec<AcmeOnDemandConfig>,
  memory_acme_account_cache_data: Arc<RwLock<HashMap<String, Vec<u8>>>>,
  prevent_file_race_conditions_sem: Arc<tokio::sync::Semaphore>,
) {
  let acme_on_demand_configs = Arc::new(acme_on_demand_configs);
  let mut existing_combinations = existing_combinations;
  while let Ok(received_data) = acme_on_demand_rx.recv().await {
    let on_demand_tls_ask_endpoint = on_demand_tls_ask_endpoint.clone();
    if let Some(on_demand_tls_ask_endpoint) = on_demand_tls_ask_endpoint {
      let mut url_parts = on_demand_tls_ask_endpoint.into_parts();
      let path_and_query_str = if let Some(path_and_query) = url_parts.path_and_query {
        let query = path_and_query.query();
        let query = if let Some(query) = query {
          format!("{}&domain={}", query, urlencoding::encode(&received_data.0))
        } else {
          format!("domain={}", urlencoding::encode(&received_data.0))
        };
        format!("{}?{}", path_and_query.path(), query)
      } else {
        format!("/?domain={}", urlencoding::encode(&received_data.0))
      };
      url_parts.path_and_query = Some(match path_and_query_str.parse() {
        Ok(parsed) => parsed,
        Err(err) => {
          acme_logger
            .log(&format!(
              "Error while formatting the URL for on-demand TLS request: {err}"
            ))
            .await;
          continue;
        }
      });
      let endpoint_url = match hyper::Uri::from_parts(url_parts) {
        Ok(parsed) => parsed,
        Err(err) => {
          acme_logger
            .log(&format!(
              "Error while formatting the URL for on-demand TLS request: {err}"
            ))
            .await;
          continue;
        }
      };
      let crypto_provider = crypto_provider.clone();
      let ask_closure = async {
        let client = hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
          .build::<_, http_body_util::Empty<hyper::body::Bytes>>(
          hyper_rustls::HttpsConnectorBuilder::new()
            .with_tls_config(build_raw_rustls_client_config(
              !on_demand_tls_ask_endpoint_verify,
              crypto_provider,
            )?)
            .https_or_http()
            .enable_http1()
            .enable_http2()
            .build(),
        );
        let request = hyper::Request::builder()
          .method(hyper::Method::GET)
          .uri(endpoint_url)
          .body(http_body_util::Empty::<hyper::body::Bytes>::new())?;
        let response = client.request(request).await?;

        Ok::<_, Box<dyn Error + Send + Sync>>(response.status().is_success())
      };
      match ask_closure.await {
        Ok(true) => (),
        Ok(false) => {
          acme_logger
            .log(&format!(
              "The TLS certificate cannot be issued for \"{}\" hostname",
              &received_data.0
            ))
            .await;
          continue;
        }
        Err(err) => {
          acme_logger
            .log(&format!(
              "Error while determining if the TLS certificate can be issued for \"{}\" hostname: {err}",
              &received_data.0
            ))
            .await;
          continue;
        }
      }
    }
    if existing_combinations.contains(&received_data) {
      continue;
    } else {
      existing_combinations.insert(received_data.clone());
    }
    let (sni_hostname, port) = received_data;
    let acme_configs_mutex = acme_configs_mutex.clone();
    let acme_on_demand_configs = acme_on_demand_configs.clone();
    let memory_acme_account_cache_data = memory_acme_account_cache_data.clone();
    let prevent_file_race_conditions_sem = prevent_file_race_conditions_sem.clone();
    tokio::spawn(async move {
      for acme_on_demand_config in acme_on_demand_configs.iter() {
        if match_hostname(acme_on_demand_config.sni_hostname.as_deref(), Some(&sni_hostname))
          && acme_on_demand_config.port == port
        {
          let sem_guard = prevent_file_race_conditions_sem.acquire().await;
          add_domain_to_cache(acme_on_demand_config, &sni_hostname)
            .await
            .unwrap_or_default();
          drop(sem_guard);

          acme_configs_mutex.lock().await.push(
            convert_on_demand_config(
              acme_on_demand_config,
              sni_hostname.clone(),
              memory_acme_account_cache_data,
            )
            .await,
          );
          break;
        }
      }
    });
  }
}
