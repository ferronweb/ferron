//! Certificate provisioning using the ACME protocol.
//!
//! Handles account creation/loading, order placement, challenge solving,
//! certificate finalization, and caching.

use std::{
    future::Future,
    net::IpAddr,
    ops::Sub,
    pin::Pin,
    sync::Arc,
    time::{Duration, SystemTime},
};

use bytes::Bytes;
use ferron_core::{log_debug, log_error, log_info, log_warn};
use hyper::Request;
use hyper_util::client::legacy::Client as HyperClient;
use hyper_util::{client::legacy::connect::HttpConnector, rt::TokioExecutor};
use instant_acme::{
    Account, AccountCredentials, AuthorizationStatus, BodyWrapper, BytesResponse,
    CertificateIdentifier, HttpClient, Identifier, NewAccount, NewOrder, OrderStatus, RetryPolicy,
};
use rustls::{sign::CertifiedKey, ClientConfig};
use rustls_pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
use tokio::io::AsyncWriteExt;
use x509_parser::prelude::{FromDer, X509Certificate};

use crate::cache::{get_account_cache_key, get_certificate_cache_key, CertificateCacheData};
use crate::challenge::tlsalpn01::TlsAlpn01Resolver;
use crate::config::AcmeConfig;

const SECONDS_BEFORE_RENEWAL: u64 = 86400; // 1 day before expiration

/// Checks if a TLS certificate is still valid (not needing renewal).
pub fn check_certificate_validity(
    certificate: &CertificateDer,
    renewal_info: Option<&instant_acme::RenewalInfo>,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    if let Some(renewal_info) = renewal_info {
        return Ok(SystemTime::now() < renewal_info.suggested_window.start);
    }
    let (_, x509_certificate) = X509Certificate::from_der(certificate)?;
    let validity = x509_certificate.validity();
    if let Some(time_to_expiration) = validity.time_to_expiration() {
        let time_before_expiration =
            if let Some(valid_duration) = validity.not_after.sub(validity.not_before) {
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

/// Checks if the current certificate is valid. If a cached cert is valid, installs it.
pub async fn check_certificate_validity_or_install_cached(
    config: &mut AcmeConfig,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    // Check if currently loaded cert is still valid
    if let Some(certified_key) = config.certified_key_lock.read().await.as_deref() {
        if let Some(certificate) = certified_key.cert.first() {
            if let Some(acme_account) = &config.account {
                if let Ok(certificate_id) = CertificateIdentifier::try_from(certificate) {
                    if let Ok(renewal_info) = acme_account.renewal_info(&certificate_id).await {
                        if SystemTime::now() < renewal_info.0.suggested_window.start {
                            return Ok(true);
                        }
                    }
                }
            } else if check_certificate_validity(certificate, None)? {
                return Ok(true);
            }
        }
    }

    // Check cache
    let certificate_cache_key =
        get_certificate_cache_key(&config.domains, config.profile.as_deref());
    if let Some(serialized_data) = config.certificate_cache.get(&certificate_cache_key).await {
        if let Ok(data) = serde_json::from_slice::<CertificateCacheData>(&serialized_data) {
            if let Ok(certs) = CertificateDer::pem_slice_iter(data.certificate_chain_pem.as_bytes())
                .collect::<Result<Vec<_>, _>>()
            {
                if let Some(certificate) = certs.first() {
                    let is_valid = if let Some(acme_account) = &config.account {
                        if let Ok(certificate_id) = CertificateIdentifier::try_from(certificate) {
                            if let Ok(renewal_info) =
                                acme_account.renewal_info(&certificate_id).await
                            {
                                SystemTime::now() < renewal_info.0.suggested_window.start
                            } else {
                                check_certificate_validity(certificate, None).unwrap_or(false)
                            }
                        } else {
                            check_certificate_validity(certificate, None).unwrap_or(false)
                        }
                    } else {
                        check_certificate_validity(certificate, None).unwrap_or(false)
                    };

                    if is_valid {
                        if let Ok(private_key) =
                            PrivateKeyDer::from_pem_slice(data.private_key_pem.as_bytes())
                        {
                            install_certified_key(config, certs, private_key, &data).await?;
                            return Ok(true);
                        }
                    }
                }
            }
        }
    }

    Ok(false)
}

/// Installs a certified key into the config and optionally saves to disk.
async fn install_certified_key(
    config: &AcmeConfig,
    certs: Vec<CertificateDer<'static>>,
    private_key: PrivateKeyDer<'static>,
    cache_data: &CertificateCacheData,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let domains = config.domains.join(", ");
    let chain_len = certs.len();

    let signing_key = rustls::crypto::aws_lc_rs::default_provider()
        .key_provider
        .load_private_key(private_key)?;

    *config.certified_key_lock.write().await =
        Some(Arc::new(CertifiedKey::new(certs, signing_key)));

    log_debug!("Certificate installed for {domains}, chain length: {chain_len}");

    // Save to files if configured
    if let Some((cert_path, key_path)) = &config.save_paths {
        tokio::fs::write(cert_path, &cache_data.certificate_chain_pem).await?;

        let mut open_options = tokio::fs::OpenOptions::new();
        open_options.write(true).create(true).truncate(true);

        #[cfg(unix)]
        open_options.mode(0o600);

        let mut file = open_options.open(key_path).await?;
        file.write_all(cache_data.private_key_pem.as_bytes())
            .await?;
        file.flush().await.unwrap_or_default();

        if let Some(command) = &config.post_obtain_command {
            log_info!("Post-obtain command started for {domains}: {command}");
            match tokio::process::Command::new(command)
                .env("FERRON_ACME_DOMAIN", config.domains.join(","))
                .env("FERRON_ACME_CERT_PATH", cert_path)
                .env("FERRON_ACME_KEY_PATH", key_path)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
            {
                Ok(mut child) => {
                    let _ = child.wait().await;
                }
                Err(e) => {
                    log_warn!("Post-obtain command failed for {domains}: {e}");
                }
            }
        }
    }

    Ok(())
}

/// Provisions a TLS certificate using ACME for the given config.
pub async fn provision_certificate(
    config: &mut AcmeConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let domains = config.domains.join(", ");
    let account_cache_key = get_account_cache_key(&config.contact, &config.directory);
    let certificate_cache_key =
        get_certificate_cache_key(&config.domains, config.profile.as_deref());

    // Step 1: Load or create ACME account
    let acme_account = if let Some(account) = config.account.take() {
        account
    } else {
        let account_builder = Account::builder_with_http(Box::new(HttpsClientForAcme::new(
            config.rustls_client_config.clone(),
        )));

        if let Some(credentials_bytes) = config.account_cache.get(&account_cache_key).await {
            if let Ok(credentials) =
                serde_json::from_slice::<AccountCredentials>(&credentials_bytes)
            {
                log_debug!("ACME account loaded from cache for {domains}");
                account_builder.from_credentials(credentials).await?
            } else {
                create_new_account(config, account_builder, &account_cache_key).await?
            }
        } else {
            create_new_account(config, account_builder, &account_cache_key).await?
        }
    };

    config.account.replace(acme_account.clone());

    // Step 2: Check if current cert is still valid or cached
    if check_certificate_validity_or_install_cached(config).await? {
        log_debug!("ACME certificate still valid or loaded from cache for {domains}");
        return Ok(());
    }

    // Step 3: Create a new ACME order
    let acme_identifiers: Vec<Identifier> = config
        .domains
        .iter()
        .map(|s| {
            if let Ok(ip) = s.parse::<IpAddr>() {
                Identifier::Ip(ip)
            } else {
                Identifier::Dns(s.clone())
            }
        })
        .collect();

    let mut new_order = NewOrder::new(&acme_identifiers);
    if let Some(profile) = &config.profile {
        new_order = new_order.profile(profile);
    }

    log_debug!("ACME order created for domains: {domains}");
    let mut order = match acme_account.new_order(&new_order).await {
        Ok(o) => o,
        Err(instant_acme::Error::Api(ref problem))
            if problem.r#type.as_deref()
                == Some("urn:ietf:params:acme:error:accountDoesNotExist") =>
        {
            log_warn!(
                "ACME account not found on server for {directory}, recreating",
                directory = config.directory
            );
            config.account_cache.remove(&account_cache_key).await;
            let account_builder = Account::builder_with_http(Box::new(HttpsClientForAcme::new(
                config.rustls_client_config.clone(),
            )));
            let new_account =
                create_new_account(config, account_builder, &account_cache_key).await?;
            new_account.new_order(&new_order).await?
        }
        Err(e) => {
            log_error!("Failed to create ACME order for {domains}: {e}");
            return Err(Box::new(e));
        }
    };

    // Step 4: Solve challenges
    let mut dns_01_domains = Vec::new();
    let mut authorizations = order.authorizations();
    while let Some(auth) = authorizations.next().await {
        let mut auth = auth?;
        match auth.status {
            AuthorizationStatus::Pending => {}
            AuthorizationStatus::Valid => continue,
            _ => {
                log_error!(
                    "ACME authorization failed — status: {:?}, domains: {domains}",
                    auth.status,
                );
                return Err(anyhow::anyhow!("Invalid ACME authorization status").into());
            }
        }

        let mut challenge = auth
            .challenge(config.challenge_type.clone())
            .ok_or_else(|| {
                log_error!(
                    "ACME server doesn't support the requested challenge type {:?} for {domains}",
                    config.challenge_type
                );
                anyhow::anyhow!("The ACME server doesn't support the requested challenge type")
            })?;

        let identifier = match &challenge.identifier().identifier {
            Identifier::Dns(name) => name.to_string(),
            Identifier::Ip(ip) => ip.to_string(),
            _ => {
                log_error!(
                    "Unsupported ACME identifier type for {domains}: {:?}",
                    challenge.identifier().identifier
                );
                return Err(anyhow::anyhow!("Unsupported ACME identifier type").into());
            }
        };

        let key_authorization = challenge.key_authorization();

        log_debug!(
            "ACME {:?} challenge initiated for {domains}",
            config.challenge_type
        );

        match config.challenge_type {
            instant_acme::ChallengeType::TlsAlpn01 => {
                let (certified_key, _ident) =
                    TlsAlpn01Resolver::generate_challenge_cert(&identifier, &key_authorization)?;
                *config.tls_alpn_01_data_lock.write().await =
                    Some((certified_key, identifier.clone()));
            }
            instant_acme::ChallengeType::Http01 => {
                *config.http_01_data_lock.write().await = Some((
                    challenge.token.clone(),
                    key_authorization.as_str().to_string(),
                ));
            }
            instant_acme::ChallengeType::Dns01 => {
                if let Some(ref dns_client) = config.dns_client {
                    // Remove any existing challenge record first
                    let challenge_domain = format!("_acme-challenge.{identifier}");
                    let _ = dns_client.delete_record(&challenge_domain, "TXT").await;

                    let dns_value = key_authorization.dns_value();
                    let ttl = dns_client.minimum_ttl().max(60);
                    let challenge_domain_log = challenge_domain.clone();
                    dns_client
                        .update_record(&ferron_dns::DnsRecord {
                            name: challenge_domain,
                            record_type: ferron_dns::DnsRecordType::TXT,
                            value: dns_value,
                            ttl,
                        })
                        .await?;

                    log_debug!("DNS-01 record created for {challenge_domain_log}, TTL {ttl}");

                    // Wait for DNS propagation
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    dns_01_domains.push(identifier.clone());
                } else {
                    return Err(
                        anyhow::anyhow!("No DNS client configured for DNS-01 challenge").into(),
                    );
                }
            }
            _ => {}
        }

        if let Err(err) = challenge.set_ready().await {
            log_error!("Failed to set ACME challenge ready for {domains}: {err}");
            return Err(Box::new(err));
        };
        log_debug!(
            "ACME {:?} challenge solved for {domains}",
            config.challenge_type
        );
    }

    // Step 5: Wait for order to be ready
    let order_status = match order.poll_ready(&RetryPolicy::default()).await {
        Ok(status) => status,
        Err(e) => {
            log_error!("Failed to finalize ACME order for {domains}: {e}");
            return Err(Box::new(e));
        }
    };
    match order_status {
        OrderStatus::Ready => {}
        OrderStatus::Invalid => {
            log_error!(
                "ACME order failed — status: invalid, domains: {domains}, reason: {}",
                order.refresh().await.map_or_else(
                    |e| e.to_string(),
                    |s| s.error.as_ref().map_or(
                        "unknown (failed ACME challenge verification?)".to_string(),
                        |s| s.to_string()
                    )
                )
            );
            return Err(anyhow::anyhow!("ACME order is invalid").into());
        }
        _ => {
            log_error!("ACME order failed — status: {order_status:?}, domains: {domains}");
            return Err(anyhow::anyhow!("ACME order is not ready").into());
        }
    }

    // Step 6: Finalize and obtain certificate
    let private_key_pem = match order.finalize().await {
        Ok(pem) => pem,
        Err(e) => {
            log_error!("Failed to finalize ACME order for {domains}: {e}");
            return Err(Box::new(e));
        }
    };
    let certificate_chain_pem = match order.poll_certificate(&RetryPolicy::default()).await {
        Ok(pem) => pem,
        Err(e) => {
            log_error!("Failed to obtain ACME certificate for {domains}: {e}");
            return Err(Box::new(e));
        }
    };

    let certs = CertificateDer::pem_slice_iter(certificate_chain_pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| match e {
            rustls_pki_types::pem::Error::Io(err) => err,
            err => std::io::Error::other(err),
        })?;
    let private_key = match PrivateKeyDer::from_pem_slice(private_key_pem.as_bytes()) {
        Ok(k) => k,
        Err(rustls_pki_types::pem::Error::Io(err)) => return Err(Box::new(err)),
        Err(err) => return Err(Box::new(std::io::Error::other(err))),
    };

    let cache_data = CertificateCacheData {
        certificate_chain_pem: certificate_chain_pem.clone(),
        private_key_pem: private_key_pem.clone(),
    };

    // Store in cache
    config
        .certificate_cache
        .set(&certificate_cache_key, serde_json::to_vec(&cache_data)?)
        .await?;

    // Install the cert
    install_certified_key(config, certs, private_key, &cache_data).await?;

    config.account.replace(acme_account);

    // Step 7: Cleanup challenge data
    cleanup_challenge_data(config, &dns_01_domains).await;

    Ok(())
}

/// Cleans up challenge data after certificate issuance.
async fn cleanup_challenge_data(config: &AcmeConfig, dns_01_domains: &[String]) {
    match config.challenge_type {
        instant_acme::ChallengeType::TlsAlpn01 => {
            *config.tls_alpn_01_data_lock.write().await = None;
        }
        instant_acme::ChallengeType::Http01 => {
            *config.http_01_data_lock.write().await = None;
        }
        instant_acme::ChallengeType::Dns01 => {
            if let Some(ref dns_client) = config.dns_client {
                for domain in dns_01_domains {
                    let challenge_domain = format!("_acme-challenge.{domain}");
                    let _ = dns_client.delete_record(&challenge_domain, "TXT").await;
                    log_debug!("DNS-01 record cleanup completed for {challenge_domain}");
                }
            }
        }
        _ => {}
    }
}

/// Creates a new ACME account and caches it.
async fn create_new_account(
    config: &AcmeConfig,
    builder: instant_acme::AccountBuilder,
    account_cache_key: &str,
) -> Result<Account, Box<dyn std::error::Error + Send + Sync>> {
    let contact_refs: Vec<&str> = config.contact.iter().map(|s| s.as_str()).collect();
    let (account, credentials) = builder
        .create(
            &NewAccount {
                contact: &contact_refs,
                terms_of_service_agreed: true,
                only_return_existing: false,
            },
            config.directory.clone(),
            config.eab_key.as_deref(),
        )
        .await?;

    config
        .account_cache
        .set(account_cache_key, serde_json::to_vec(&credentials)?)
        .await?;

    log_info!(
        "ACME account created for directory {}, contact: {}",
        config.directory,
        config.contact.first().map(|s| s.as_str()).unwrap_or("none")
    );

    Ok(account)
}

/// HTTPS client wrapper for instant-acme's HttpClient trait.
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
