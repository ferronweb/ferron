use std::{
  collections::HashMap,
  error::Error,
  future::Future,
  net::IpAddr,
  ops::{Deref, Sub},
  path::PathBuf,
  pin::Pin,
  sync::Arc,
  time::{Duration, SystemTime},
};

use base64::Engine;
use bytes::Bytes;
use hyper::Request;
use hyper_util::client::legacy::Client as HyperClient;
use hyper_util::{client::legacy::connect::HttpConnector, rt::TokioExecutor};
use instant_acme::{
  Account, AccountCredentials, AuthorizationStatus, BodyWrapper, BytesResponse, CertificateIdentifier, ChallengeType,
  ExternalAccountKey, HttpClient, Identifier, NewAccount, NewOrder, OrderStatus, RenewalInfo, RetryPolicy,
};
use rcgen::{CertificateParams, CustomExtension, KeyPair};
use rustls::{
  crypto::CryptoProvider,
  server::{ClientHello, ResolvesServerCert},
  sign::CertifiedKey,
  ClientConfig,
};
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use serde::{Deserialize, Serialize};
use tokio::{sync::RwLock, time::Instant};
use x509_parser::prelude::{FromDer, X509Certificate};
use xxhash_rust::xxh3::xxh3_128;

use crate::tls_util::load_host_resolver;
use ferron_common::dns::DnsProvider;

pub const ACME_TLS_ALPN_NAME: &[u8] = b"acme-tls/1";
const SECONDS_BEFORE_RENEWAL: u64 = 86400; // 1 day before expiration

pub type TlsAlpn01DataLock = Arc<RwLock<Option<(Arc<CertifiedKey>, String)>>>;
pub type Http01DataLock = Arc<RwLock<Option<(String, String)>>>;

/// Represents the configuration for the ACME client.
pub struct AcmeConfig {
  /// The Rustls client configuration to use for ACME communication.
  pub rustls_client_config: ClientConfig,
  /// The domains for which to request certificates.
  pub domains: Vec<String>,
  /// The type of challenge to use for ACME certificate issuance.
  pub challenge_type: ChallengeType,
  /// The contact information for the ACME account.
  pub contact: Vec<String>,
  /// The directory URL for the ACME server.
  pub directory: String,
  /// The optional EAB key
  pub eab_key: Option<Arc<ExternalAccountKey>>,
  /// The optional ACME profile name
  pub profile: Option<String>,
  /// The cache for storing ACME account information.
  pub account_cache: AcmeCache,
  /// The cache for storing ACME certificate information.
  pub certificate_cache: AcmeCache,
  /// The lock for managing the certified key.
  pub certified_key_lock: Arc<RwLock<Option<Arc<CertifiedKey>>>>,
  /// The lock for managing the TLS-ALPN-01 data.
  pub tls_alpn_01_data_lock: TlsAlpn01DataLock,
  /// The lock for managing the HTTP-01 data.
  pub http_01_data_lock: Http01DataLock,
  /// The ACME DNS provider.
  pub dns_provider: Option<Arc<dyn DnsProvider + Send + Sync>>,
  /// The certificate renewal information.
  pub renewal_info: Option<(RenewalInfo, Instant)>,
  /// The ACME account information
  pub account: Option<Account>,
}

/// Represents the type of cache to use for storing ACME data.
pub enum AcmeCache {
  /// Use an in-memory cache.
  Memory(Arc<RwLock<HashMap<String, Vec<u8>>>>),
  /// Use a file-based cache.
  File(PathBuf),
}

#[derive(Serialize, Deserialize)]
struct CertificateCacheData {
  certificate_chain_pem: String,
  private_key_pem: String,
}

/// Gets data from the cache.
async fn get_from_cache(cache: &AcmeCache, key: &str) -> Option<Vec<u8>> {
  match cache {
    AcmeCache::Memory(cache) => cache.read().await.get(key).cloned(),
    AcmeCache::File(path) => tokio::fs::read(path.join(key)).await.ok(),
  }
}

/// Represents the on-demand configuration for the ACME client.
pub struct AcmeOnDemandConfig {
  /// The Rustls client configuration to use for ACME communication.
  pub rustls_client_config: ClientConfig,
  /// The type of challenge to use for ACME certificate issuance.
  pub challenge_type: ChallengeType,
  /// The contact information for the ACME account.
  pub contact: Vec<String>,
  /// The directory URL for the ACME server.
  pub directory: String,
  /// The optional EAB key
  pub eab_key: Option<Arc<ExternalAccountKey>>,
  /// The optional ACME profile name
  pub profile: Option<String>,
  /// The path to the cache directory for storing ACME information.
  pub cache_path: Option<PathBuf>,
  /// The lock for managing the SNI resolver.
  #[allow(clippy::type_complexity)]
  pub sni_resolver_lock: Arc<RwLock<Vec<(String, Arc<dyn ResolvesServerCert>)>>>,
  /// The lock for managing the TLS-ALPN-01 resolver.
  pub tls_alpn_01_resolver_lock: Arc<RwLock<Vec<TlsAlpn01DataLock>>>,
  /// The lock for managing the HTTP-01 resolver.
  pub http_01_resolver_lock: Arc<RwLock<Vec<Http01DataLock>>>,
  /// The ACME DNS provider.
  pub dns_provider: Option<Arc<dyn DnsProvider + Send + Sync>>,
  /// The SNI hostname.
  pub sni_hostname: Option<String>,
  /// The port to use for ACME communication.
  pub port: u16,
}

/// Sets data in the cache.
async fn set_in_cache(cache: &AcmeCache, key: &str, value: Vec<u8>) -> Result<(), Box<dyn Error + Send + Sync>> {
  match cache {
    AcmeCache::Memory(cache) => {
      cache.write().await.insert(key.to_string(), value);
      Ok(())
    }
    AcmeCache::File(path) => {
      tokio::fs::create_dir_all(path).await.unwrap_or_default();
      tokio::fs::write(path.join(key), value).await.map_err(Into::into)
    }
  }
}

/// Checks if the TLS certificate is valid
fn check_certificate_validity(
  certificate: &CertificateDer,
  renewal_info: Option<&RenewalInfo>,
) -> Result<bool, Box<dyn Error + Send + Sync>> {
  if let Some(renewal_info) = renewal_info {
    return Ok(SystemTime::now() < renewal_info.suggested_window.start);
  }
  let (_, x509_certificate) = X509Certificate::from_der(certificate)?;
  let validity = x509_certificate.validity();
  if let Some(time_to_expiration) = validity.time_to_expiration() {
    let time_before_expiration = if let Some(valid_duration) = validity.not_after.sub(validity.not_before) {
      (valid_duration.whole_seconds().unsigned_abs() / 2).min(SECONDS_BEFORE_RENEWAL)
    } else {
      SECONDS_BEFORE_RENEWAL
    };
    if time_to_expiration >= Duration::from_secs(time_before_expiration) {
      return Ok(true);
    }
  }
  Ok(false)
}

/// Determines the account cache key
fn get_account_cache_key(config: &AcmeConfig) -> String {
  format!(
    "account_{}",
    base64::engine::general_purpose::URL_SAFE_NO_PAD
      .encode(xxh3_128(format!("{};{}", &config.contact.join(","), &config.directory).as_bytes()).to_be_bytes())
  )
}

/// Determines the certificate cache key
fn get_certificate_cache_key(config: &AcmeConfig) -> String {
  format!(
    "certificate_{}",
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
      xxh3_128(
        format!(
          "{}{}",
          config.domains.join(","),
          config.profile.as_ref().map_or("".to_string(), |p| format!(";{p}"))
        )
        .as_bytes()
      )
      .to_be_bytes()
    )
  )
}

/// Determines the account cache key
fn get_hostname_cache_key(config: &AcmeOnDemandConfig) -> String {
  format!(
    "hostname_{}",
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
      xxh3_128(
        format!(
          "{}{}",
          &config.port,
          config.sni_hostname.as_ref().map_or("".to_string(), |h| format!(";{h}"))
        )
        .as_bytes()
      )
      .to_be_bytes()
    )
  )
}

/// Checks if the TLS certificate (cached or live) is valid. If cached certificate is valid, installs the cached certificate
pub async fn check_certificate_validity_or_install_cached(
  config: &mut AcmeConfig,
  acme_account: Option<&Account>,
) -> Result<bool, Box<dyn Error + Send + Sync>> {
  if let Some(certified_key) = config.certified_key_lock.read().await.as_deref() {
    if let Some(certificate) = certified_key.cert.first() {
      if let Some(acme_account) = acme_account {
        if config
          .renewal_info
          .as_ref()
          .is_none_or(|v| v.1.elapsed() > Duration::ZERO)
        {
          if let Ok(certificate_id) = CertificateIdentifier::try_from(certificate) {
            if let Ok(renewal_info) = acme_account.renewal_info(&certificate_id).await {
              let mut renewal_instant = Instant::now();
              renewal_instant += renewal_info.1;
              config.renewal_info = Some((renewal_info.0, renewal_instant));
            }
          }
        }
      }
      if check_certificate_validity(certificate, config.renewal_info.as_ref().map(|i| &i.0))? {
        return Ok(true);
      }
    }
  }

  let certificate_cache_key = get_certificate_cache_key(config);

  if let Some(serialized_certificate_cache_data) =
    get_from_cache(&config.certificate_cache, &certificate_cache_key).await
  {
    let certificate_data = serde_json::from_slice::<CertificateCacheData>(&serialized_certificate_cache_data)?;
    let certs = rustls_pemfile::certs(&mut std::io::Cursor::new(
      certificate_data.certificate_chain_pem.as_bytes(),
    ))
    .collect::<Result<Vec<_>, _>>()?;
    if let Some(certificate) = certs.first() {
      if let Some(acme_account) = acme_account {
        if config
          .renewal_info
          .as_ref()
          .is_none_or(|v| v.1.elapsed() > Duration::ZERO)
        {
          if let Ok(certificate_id) = CertificateIdentifier::try_from(certificate) {
            if let Ok(renewal_info) = acme_account.renewal_info(&certificate_id).await {
              let mut renewal_instant = Instant::now();
              renewal_instant -= renewal_info.1;
              config.renewal_info = Some((renewal_info.0, renewal_instant));
            }
          }
        }
      }
      if check_certificate_validity(certificate, config.renewal_info.as_ref().map(|i| &i.0))? {
        let private_key =
          (match rustls_pemfile::private_key(&mut std::io::Cursor::new(certificate_data.private_key_pem.as_bytes())) {
            Ok(Some(private_key)) => Ok(private_key),
            Ok(None) => Err(std::io::Error::new(
              std::io::ErrorKind::InvalidData,
              "Invalid private key",
            )),
            Err(err) => Err(err),
          })?;

        let signing_key = CryptoProvider::get_default()
          .ok_or(anyhow::anyhow!("Cannot get default crypto provider"))?
          .key_provider
          .load_private_key(private_key)?;

        *config.certified_key_lock.write().await = Some(Arc::new(CertifiedKey::new(certs, signing_key)));

        return Ok(true);
      }
    }
  }

  Ok(false)
}

/// Provisions TLS certificates using the ACME protocol.
pub async fn provision_certificate(config: &mut AcmeConfig) -> Result<(), Box<dyn Error + Send + Sync>> {
  let account_cache_key = get_account_cache_key(config);
  let certificate_cache_key = get_certificate_cache_key(config);

  let acme_account = if let Some(acme_account) = config.account.take() {
    acme_account
  } else {
    let acme_account_builder =
      Account::builder_with_http(Box::new(HttpsClientForAcme::new(config.rustls_client_config.clone())));

    if let Some(account_credentials_serialized) = get_from_cache(&config.account_cache, &account_cache_key).await {
      let account_credentials = serde_json::from_slice::<AccountCredentials>(&account_credentials_serialized)?;
      acme_account_builder.from_credentials(account_credentials).await?
    } else {
      let (account, account_credentials) = acme_account_builder
        .create(
          &NewAccount {
            contact: config.contact.iter().map(|s| s.deref()).collect::<Vec<_>>().as_slice(),
            terms_of_service_agreed: true,
            only_return_existing: false,
          },
          config.directory.clone(),
          config.eab_key.as_deref(),
        )
        .await?;

      set_in_cache(
        &config.account_cache,
        &account_cache_key,
        serde_json::to_vec(&account_credentials)?,
      )
      .await?;
      account
    }
  };

  if check_certificate_validity_or_install_cached(config, Some(&acme_account)).await? {
    // Certificate is still valid, no need to renew
    config.account.replace(acme_account);
    return Ok(());
  }

  let acme_identifiers_vec = config
    .domains
    .iter()
    .map(|s| {
      if let Ok(ip) = s.parse::<IpAddr>() {
        Identifier::Ip(ip)
      } else {
        Identifier::Dns(s.to_string())
      }
    })
    .collect::<Vec<_>>();

  let mut acme_new_order = NewOrder::new(&acme_identifiers_vec);
  if let Some(profile) = &config.profile {
    acme_new_order = acme_new_order.profile(profile);
  }

  let mut acme_order = acme_account.new_order(&acme_new_order).await?;
  let mut dns_01_identifiers = Vec::new();
  let mut acme_authorizations = acme_order.authorizations();
  while let Some(acme_authorization) = acme_authorizations.next().await {
    let mut acme_authorization = acme_authorization?;
    match acme_authorization.status {
      AuthorizationStatus::Pending => {}
      AuthorizationStatus::Valid => continue,
      _ => Err(anyhow::anyhow!("Invalid ACME authorization status"))?,
    }

    let mut challenge = acme_authorization
      .challenge(config.challenge_type.clone())
      .ok_or(anyhow::anyhow!(
        "The ACME server doesn't support the requested challenge type"
      ))?;

    let identifier = match challenge.identifier().identifier {
      Identifier::Dns(identifier) => identifier.to_string(),
      Identifier::Ip(ip) => ip.to_string(),
      _ => Err(anyhow::anyhow!("Unsupported ACME identifier type",))?,
    };

    let key_authorization = challenge.key_authorization();
    match config.challenge_type {
      ChallengeType::TlsAlpn01 => {
        let mut params = CertificateParams::new(vec![identifier.clone()])?;
        params.custom_extensions.push(CustomExtension::new_acme_identifier(
          key_authorization.digest().as_ref(),
        ));
        let key_pair = KeyPair::generate()?;
        let certificate = params.self_signed(&key_pair)?;
        let private_key = PrivateKeyDer::try_from(key_pair.serialize_der())?;

        let signing_key = CryptoProvider::get_default()
          .ok_or(anyhow::anyhow!("Cannot get default crypto provider"))?
          .key_provider
          .load_private_key(private_key)?;

        *config.tls_alpn_01_data_lock.write().await = Some((
          Arc::new(CertifiedKey::new(vec![certificate.der().to_owned()], signing_key)),
          identifier.clone(),
        ));
      }
      ChallengeType::Http01 => {
        let key_auth_value = key_authorization.as_str();
        *config.http_01_data_lock.write().await = Some((challenge.token.clone(), key_auth_value.to_string()));
      }
      ChallengeType::Dns01 => {
        if let Some(dns_provider) = &config.dns_provider {
          dns_provider
            .remove_acme_txt_record(&identifier)
            .await
            .unwrap_or_default();
          dns_provider
            .set_acme_txt_record(&identifier, &key_authorization.dns_value())
            .await?;
          // Wait for DNS propagation
          tokio::time::sleep(Duration::from_secs(60)).await;
          dns_01_identifiers.push(identifier.clone());
        } else {
          Err(anyhow::anyhow!("No DNS provider configured."))?;
        }
      }
      _ => (),
    }

    challenge.set_ready().await?;
  }

  let acme_order_status = acme_order.poll_ready(&RetryPolicy::default()).await?;
  if acme_order_status != OrderStatus::Ready {
    Err(anyhow::anyhow!("ACME order is not ready",))?;
  }

  let finalize_closure = async {
    let private_key_pem = acme_order.finalize().await?;
    let certificate_chain_pem = acme_order.poll_certificate(&RetryPolicy::default()).await?;

    let certificate_cache_data = CertificateCacheData {
      certificate_chain_pem: certificate_chain_pem.clone(),
      private_key_pem: private_key_pem.clone(),
    };

    set_in_cache(
      &config.certificate_cache,
      &certificate_cache_key,
      serde_json::to_vec(&certificate_cache_data)?,
    )
    .await?;

    let certs = rustls_pemfile::certs(&mut std::io::Cursor::new(certificate_chain_pem.as_bytes()))
      .collect::<Result<Vec<_>, _>>()?;
    let private_key = (match rustls_pemfile::private_key(&mut std::io::Cursor::new(private_key_pem.as_bytes())) {
      Ok(Some(private_key)) => Ok(private_key),
      Ok(None) => Err(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "Invalid private key",
      )),
      Err(err) => Err(err),
    })?;

    let signing_key = CryptoProvider::get_default()
      .ok_or(anyhow::anyhow!("Cannot get default crypto provider"))?
      .key_provider
      .load_private_key(private_key)?;

    config.account.replace(acme_account);

    *config.certified_key_lock.write().await = Some(Arc::new(CertifiedKey::new(certs, signing_key)));

    Ok::<_, Box<dyn Error + Send + Sync>>(())
  };

  let result = finalize_closure.await;

  // Cleanup
  if let Some(dns_provider) = &config.dns_provider {
    for identifier in dns_01_identifiers {
      dns_provider
        .remove_acme_txt_record(&identifier)
        .await
        .unwrap_or_default();
    }
  }

  result?;

  Ok(())
}

/// Obtains the list of domains for which `AcmeOnDemandConfig` was converted into `AcmeConfig` from cache.
pub async fn get_cached_domains(config: &AcmeOnDemandConfig) -> Vec<String> {
  if let Some(pathbuf) = config.cache_path.clone() {
    let hostname_cache_key = get_hostname_cache_key(config);
    let hostname_cache = AcmeCache::File(pathbuf);
    let cache_data = get_from_cache(&hostname_cache, &hostname_cache_key).await;
    if let Some(data) = cache_data {
      serde_json::from_slice(&data).unwrap_or_default()
    } else {
      Vec::new()
    }
  } else {
    Vec::new()
  }
}

/// Adds the domain to the cache.
pub async fn add_domain_to_cache(
  config: &AcmeOnDemandConfig,
  domain: &str,
) -> Result<(), Box<dyn Error + Send + Sync>> {
  if let Some(pathbuf) = config.cache_path.clone() {
    let hostname_cache_key = get_hostname_cache_key(config);
    let hostname_cache = AcmeCache::File(pathbuf);
    let mut cached_domains = get_cached_domains(config).await;
    cached_domains.push(domain.to_string());
    let data = serde_json::to_vec(&cached_domains)?;
    set_in_cache(&hostname_cache, &hostname_cache_key, data).await?;
  }
  Ok(())
}

/// Converts a `AcmeOnDemandConfig` into an `AcmeConfig`
pub async fn convert_on_demand_config(
  config: &AcmeOnDemandConfig,
  sni_hostname: String,
  memory_acme_account_cache_data: Arc<RwLock<HashMap<String, Vec<u8>>>>,
) -> AcmeConfig {
  let (account_cache_path, cert_cache_path) = if let Some(mut pathbuf) = config.cache_path.clone() {
    let base_pathbuf = pathbuf.clone();
    let append_hash = base64::engine::general_purpose::URL_SAFE_NO_PAD
      .encode(xxh3_128(format!("{}-{sni_hostname}", config.port).as_bytes()).to_be_bytes());
    pathbuf.push(append_hash);
    (Some(base_pathbuf), Some(pathbuf))
  } else {
    (None, None)
  };

  let certified_key_lock = Arc::new(tokio::sync::RwLock::new(None));
  let tls_alpn_01_data_lock = Arc::new(tokio::sync::RwLock::new(None));
  let http_01_data_lock = Arc::new(tokio::sync::RwLock::new(None));

  // Insert new locked data
  load_host_resolver(
    &mut *config.sni_resolver_lock.write().await,
    &sni_hostname,
    Arc::new(AcmeResolver::new(certified_key_lock.clone())),
  );
  match config.challenge_type {
    ChallengeType::TlsAlpn01 => {
      config
        .tls_alpn_01_resolver_lock
        .write()
        .await
        .push(tls_alpn_01_data_lock.clone());
    }
    ChallengeType::Http01 => {
      config
        .http_01_resolver_lock
        .write()
        .await
        .push(http_01_data_lock.clone());
    }
    _ => (),
  };

  AcmeConfig {
    rustls_client_config: config.rustls_client_config.clone(),
    domains: vec![sni_hostname],
    challenge_type: config.challenge_type.clone(),
    contact: config.contact.clone(),
    directory: config.directory.clone(),
    eab_key: config.eab_key.clone(),
    profile: config.profile.clone(),
    account_cache: if let Some(account_cache_path) = account_cache_path {
      AcmeCache::File(account_cache_path)
    } else {
      AcmeCache::Memory(memory_acme_account_cache_data.clone())
    },
    certificate_cache: if let Some(cert_cache_path) = cert_cache_path {
      AcmeCache::File(cert_cache_path)
    } else {
      AcmeCache::Memory(Arc::new(tokio::sync::RwLock::new(HashMap::new())))
    },
    certified_key_lock: certified_key_lock.clone(),
    tls_alpn_01_data_lock: tls_alpn_01_data_lock.clone(),
    http_01_data_lock: http_01_data_lock.clone(),
    dns_provider: config.dns_provider.clone(),
    renewal_info: None,
    account: None,
  }
}

/// An ACME resolver resolving one certified key
#[derive(Debug)]
pub struct AcmeResolver {
  certified_key_lock: Arc<RwLock<Option<Arc<CertifiedKey>>>>,
}

impl AcmeResolver {
  /// Creates an ACME resolver
  pub fn new(certified_key_lock: Arc<RwLock<Option<Arc<CertifiedKey>>>>) -> Self {
    Self { certified_key_lock }
  }
}

impl ResolvesServerCert for AcmeResolver {
  fn resolve(&self, _client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
    self.certified_key_lock.blocking_read().clone()
  }
}

struct HttpsClientForAcme(HyperClient<hyper_rustls::HttpsConnector<HttpConnector>, BodyWrapper<Bytes>>);

impl HttpsClientForAcme {
  fn new(tls_config: ClientConfig) -> Self {
    Self(
      HyperClient::builder(TokioExecutor::new()).build(
        hyper_rustls::HttpsConnectorBuilder::new()
          .with_tls_config(tls_config)
          .https_or_http()
          .enable_http1()
          .enable_http2()
          .build(),
      ),
    )
  }
}

impl HttpClient for HttpsClientForAcme {
  fn request(
    &self,
    req: Request<BodyWrapper<Bytes>>,
  ) -> Pin<Box<dyn Future<Output = Result<BytesResponse, instant_acme::Error>> + Send>> {
    HttpClient::request(&self.0, req)
  }
}

/// The TLS-ALPN-01 ACME challenge certificate resolver
#[derive(Debug)]
pub struct TlsAlpn01Resolver {
  resolvers: Arc<tokio::sync::RwLock<Vec<TlsAlpn01DataLock>>>,
}

impl TlsAlpn01Resolver {
  /// Creates a TLS-ALPN-01 resolver
  #[allow(dead_code)]
  pub fn new() -> Self {
    Self {
      resolvers: Arc::new(tokio::sync::RwLock::new(Vec::new())),
    }
  }

  /// Creates a TLS-ALPN-01 resolver with provided resolver list lock
  pub fn with_resolvers(resolvers: Arc<tokio::sync::RwLock<Vec<TlsAlpn01DataLock>>>) -> Self {
    Self { resolvers }
  }

  /// Loads a certificate resolver lock
  pub fn load_resolver(&self, resolver: TlsAlpn01DataLock) {
    self.resolvers.blocking_write().push(resolver);
  }
}

impl ResolvesServerCert for TlsAlpn01Resolver {
  fn resolve(&self, client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
    let hostname = client_hello.server_name();

    // If blocking_read() method is used when only Tokio is used, the program would panic on resolving a TLS certificate.
    #[cfg(feature = "runtime-monoio")]
    let resolver_locks = self.resolvers.blocking_read();
    #[cfg(feature = "runtime-tokio")]
    let resolver_locks = futures_executor::block_on(async { self.resolvers.read().await });

    for resolver_lock in &*resolver_locks {
      if let Some(hostname) = hostname {
        #[cfg(feature = "runtime-monoio")]
        let resolver_data = resolver_lock.blocking_read().clone();
        #[cfg(feature = "runtime-tokio")]
        let resolver_data = futures_executor::block_on(async { resolver_lock.read().await }).clone();
        if let Some(resolver_data) = resolver_data {
          let (cert, host) = resolver_data;
          if host == hostname {
            return Some(cert);
          }
        }
      }
    }
    None
  }
}
