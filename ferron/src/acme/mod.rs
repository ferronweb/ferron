pub mod dns;

use std::{
  collections::HashMap,
  error::Error,
  future::Future,
  net::IpAddr,
  ops::{Deref, Sub},
  path::PathBuf,
  pin::Pin,
  sync::Arc,
  time::Duration,
};

use base64::Engine;
use bytes::Bytes;
use hyper::Request;
use hyper_util::client::legacy::Client as HyperClient;
use hyper_util::{client::legacy::connect::HttpConnector, rt::TokioExecutor};
use instant_acme::{
  Account, AccountCredentials, AuthorizationStatus, BodyWrapper, BytesResponse, ChallengeType,
  HttpClient, Identifier, NewAccount, NewOrder, OrderStatus, RetryPolicy,
};
use rcgen::{CertificateParams, CustomExtension, KeyPair};
use rustls::{
  crypto::CryptoProvider,
  server::{ClientHello, ResolvesServerCert},
  sign::CertifiedKey,
  ClientConfig,
};
use rustls_pki_types::PrivateKeyDer;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use x509_parser::prelude::{FromDer, X509Certificate};
use xxhash_rust::xxh3::xxh3_128;

use crate::acme::dns::DnsProvider;

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

/// Sets data in the cache.
async fn set_in_cache(
  cache: &AcmeCache,
  key: &str,
  value: Vec<u8>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
  match cache {
    AcmeCache::Memory(cache) => {
      cache.write().await.insert(key.to_string(), value);
      Ok(())
    }
    AcmeCache::File(path) => {
      tokio::fs::create_dir_all(path).await.unwrap_or_default();
      tokio::fs::write(path.join(key), value)
        .await
        .map_err(Into::into)
    }
  }
}

/// Checks if the TLS certificate is valid
fn check_certificate_validity(x509_certificate: &X509Certificate) -> bool {
  let validity = x509_certificate.validity();
  if let Some(time_to_expiration) = validity.time_to_expiration() {
    let time_before_expiration =
      if let Some(valid_duration) = validity.not_after.sub(validity.not_before) {
        (valid_duration.whole_seconds().unsigned_abs() / 2).min(SECONDS_BEFORE_RENEWAL)
      } else {
        SECONDS_BEFORE_RENEWAL
      };
    if time_to_expiration > Duration::from_secs(time_before_expiration) {
      return true;
    }
  }
  false
}

/// Determines the account cache key
fn get_account_cache_key(config: &AcmeConfig) -> String {
  format!(
    "account_{}",
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
      xxh3_128(format!("{};{}", &config.contact.join(","), &config.directory).as_bytes())
        .to_be_bytes()
    )
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
          config
            .profile
            .as_ref()
            .map_or("".to_string(), |p| format!(";{p}"))
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
) -> Result<bool, Box<dyn Error + Send + Sync>> {
  if let Some(certified_key) = config.certified_key_lock.read().await.as_deref() {
    if let Some(certificate) = certified_key.cert.first() {
      let (_, x509_certificate) = X509Certificate::from_der(certificate)?;
      if check_certificate_validity(&x509_certificate) {
        return Ok(true);
      }
    }
  }

  let certificate_cache_key = get_certificate_cache_key(config);

  if let Some(serialized_certificate_cache_data) =
    get_from_cache(&config.certificate_cache, &certificate_cache_key).await
  {
    let certificate_data =
      serde_json::from_slice::<CertificateCacheData>(&serialized_certificate_cache_data)?;
    let certs = rustls_pemfile::certs(&mut std::io::Cursor::new(
      certificate_data.certificate_chain_pem.as_bytes(),
    ))
    .collect::<Result<Vec<_>, _>>()?;
    if let Some(certificate) = certs.first() {
      let (_, x509_certificate) = X509Certificate::from_der(certificate)?;
      if check_certificate_validity(&x509_certificate) {
        let private_key = (match rustls_pemfile::private_key(&mut std::io::Cursor::new(
          certificate_data.private_key_pem.as_bytes(),
        )) {
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

        *config.certified_key_lock.write().await =
          Some(Arc::new(CertifiedKey::new(certs, signing_key)));

        return Ok(true);
      }
    }
  }

  Ok(false)
}

/// Provisions TLS certificates using the ACME protocol.
pub async fn provision_certificate(
  config: &mut AcmeConfig,
) -> Result<(), Box<dyn Error + Send + Sync>> {
  if check_certificate_validity_or_install_cached(config).await? {
    // Certificate is still valid, no need to renew
    return Ok(());
  }

  let account_cache_key = get_account_cache_key(config);
  let certificate_cache_key = get_certificate_cache_key(config);

  let acme_account_builder = Account::builder_with_http(Box::new(HttpsClientForAcme::new(
    config.rustls_client_config.clone(),
  )));

  let acme_account = if let Some(account_credentials_serialized) =
    get_from_cache(&config.account_cache, &account_cache_key).await
  {
    let account_credentials =
      serde_json::from_slice::<AccountCredentials>(&account_credentials_serialized)?;
    acme_account_builder
      .from_credentials(account_credentials)
      .await?
  } else {
    let (account, account_credentials) = acme_account_builder
      .create(
        &NewAccount {
          contact: config
            .contact
            .iter()
            .map(|s| s.deref())
            .collect::<Vec<_>>()
            .as_slice(),
          terms_of_service_agreed: true,
          only_return_existing: false,
        },
        config.directory.clone(),
        None,
      )
      .await?;

    set_in_cache(
      &config.account_cache,
      &account_cache_key,
      serde_json::to_vec(&account_credentials)?,
    )
    .await?;
    account
  };

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
        params
          .custom_extensions
          .push(CustomExtension::new_acme_identifier(
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
          Arc::new(CertifiedKey::new(
            vec![certificate.der().to_owned()],
            signing_key,
          )),
          identifier.clone(),
        ));
      }
      ChallengeType::Http01 => {
        let key_auth_value = key_authorization.as_str();
        *config.http_01_data_lock.write().await =
          Some((challenge.token.clone(), key_auth_value.to_string()));
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
    let private_key =
      (match rustls_pemfile::private_key(&mut std::io::Cursor::new(private_key_pem.as_bytes())) {
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

    *config.certified_key_lock.write().await =
      Some(Arc::new(CertifiedKey::new(certs, signing_key)));

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

struct HttpsClientForAcme(
  HyperClient<hyper_rustls::HttpsConnector<HttpConnector>, BodyWrapper<Bytes>>,
);

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
  resolvers: Vec<TlsAlpn01DataLock>,
}

impl TlsAlpn01Resolver {
  /// Creates a TLS-ALPN-01 resolver
  pub fn new() -> Self {
    Self {
      resolvers: Vec::new(),
    }
  }

  /// Loads a certificate resolver lock
  pub fn load_resolver(&mut self, resolver: TlsAlpn01DataLock) {
    self.resolvers.push(resolver);
  }
}

impl ResolvesServerCert for TlsAlpn01Resolver {
  fn resolve(&self, client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
    let hostname = client_hello.server_name();
    for resolver_lock in &self.resolvers {
      if let Some(hostname) = hostname {
        let resolver_data = resolver_lock.blocking_read().clone();
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
