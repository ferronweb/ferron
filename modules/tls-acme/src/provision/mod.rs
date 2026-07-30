//! Certificate provisioning using the ACME protocol.
//!
//! Handles account creation/loading, order placement, challenge solving,
//! certificate finalization, and caching.

mod account;
mod cert_install;
mod challenge;
mod validity;

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use instant_acme::{Account, AuthorizationStatus, Identifier, NewOrder, OrderStatus, RetryPolicy};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};

use crate::cache::{get_account_cache_key, get_certificate_cache_key, CertificateCacheData};
use crate::challenge::tlsalpn01::TlsAlpn01Resolver;
use crate::config::{build_rustls_client_config, AcmeConfig};
use crate::emit_log;
use crate::errors::acme_error_to_string;

use self::account::{create_new_account, HttpsClientForAcme};
use self::cert_install::install_certified_key;
use self::challenge::cleanup_challenge_data;
use self::validity::check_certificate_validity_or_install_cached;

/// Provisions a TLS certificate using ACME for the given config.
/// Returns `true` if a certificate was provisioned, `false` otherwise.
pub async fn provision_certificate(
    config: &mut AcmeConfig,
    event_sink: &Arc<ferron_observability::CompositeEventSink>,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let domains = config.domains.join(", ");

    // Get the provider list and extract all providers (primary + fallbacks)
    let provider_list = config.provider_list.read().await;
    let mut providers = vec![provider_list.primary.clone()];
    providers.extend(provider_list.fallbacks.iter().cloned());
    drop(provider_list);

    if check_certificate_validity_or_install_cached(config, event_sink).await? {
        return Ok(false);
    }

    let mut acme_account: Option<Account> = None;
    let mut selected_directory: Option<String> = None;
    let mut selected_contact: Option<Vec<String>> = None;
    let mut selected_eab_key: Option<Option<Arc<instant_acme::ExternalAccountKey>>> = None;
    let mut selected_profile: Option<Option<String>> = None;
    let mut selected_account_cache_key: Option<String> = None;
    let mut selected_certificate_cache_key: Option<String> = None;

    for (idx, provider) in providers.iter().enumerate() {
        let provider_name = if idx == 0 { "primary" } else { "fallback" };

        let directory = provider.directory.clone();
        let contact = provider.contact.clone();
        let eab_key = provider.eab_key.clone();
        let profile = provider.profile.clone();

        let client_config = config.rustls_client_config.clone();

        let account_cache_key = get_account_cache_key(&contact, &directory);
        let certificate_cache_key = get_certificate_cache_key(&config.domains, profile.as_deref());

        let account_builder = instant_acme::Account::builder_with_http(Box::new(
            HttpsClientForAcme::new(client_config),
        ));

        let account_result =
            if let Some(credentials_bytes) = config.account_cache.get(&account_cache_key).await {
                if let Ok(credentials) =
                    serde_json::from_slice::<instant_acme::AccountCredentials>(&credentials_bytes)
                {
                    emit_log(
                        event_sink,
                        ferron_observability::LogLevel::Debug,
                        "ACME account loaded from cache",
                        &format!(
                            "ACME account loaded from cache for {domains} (provider: {})",
                            provider_name
                        ),
                        "ferron-tls-acme",
                        vec![
                            (
                                "ferron.acme.domains",
                                ferron_observability::LogAttributeValue::String(domains.clone()),
                            ),
                            (
                                "ferron.acme.provider",
                                ferron_observability::LogAttributeValue::String(
                                    provider_name.to_string(),
                                ),
                            ),
                        ],
                    );
                    account_builder
                        .from_credentials(credentials)
                        .await
                        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
                } else {
                    create_new_account(
                        config,
                        &directory,
                        &contact,
                        eab_key.as_ref(),
                        profile.as_deref(),
                        account_builder,
                        &account_cache_key,
                        event_sink,
                    )
                    .await
                }
            } else {
                create_new_account(
                    config,
                    &directory,
                    &contact,
                    eab_key.as_ref(),
                    profile.as_deref(),
                    account_builder,
                    &account_cache_key,
                    event_sink,
                )
                .await
            };

        match account_result {
            Ok(account) => {
                acme_account = Some(account);
                selected_contact = Some(contact);
                selected_eab_key = Some(eab_key);
                selected_profile = Some(profile);
                selected_directory = Some(directory);
                selected_account_cache_key = Some(account_cache_key);
                selected_certificate_cache_key = Some(certificate_cache_key);
                break;
            }
            Err(e) => {
                emit_log(
                    event_sink,
                    ferron_observability::LogLevel::Warn,
                    "ACME account creation failed",
                    &format!(
                        "Failed to create account on {} provider: {}",
                        provider_name, e
                    ),
                    "ferron-tls-acme",
                    vec![
                        (
                            "ferron.acme.domains",
                            ferron_observability::LogAttributeValue::String(domains.clone()),
                        ),
                        (
                            "ferron.acme.provider",
                            ferron_observability::LogAttributeValue::String(
                                provider_name.to_string(),
                            ),
                        ),
                        (
                            "error.message",
                            ferron_observability::LogAttributeValue::String(e.to_string()),
                        ),
                    ],
                );

                // Try the next provider
                if idx == providers.len() - 1 {
                    return Err(e);
                }
            }
        }
    }

    // At this point, we have a valid account (or failed all providers)
    let acme_account = acme_account.ok_or_else(|| {
        anyhow::anyhow!("No valid ACME account obtained after trying all providers")
    })?;

    let directory = selected_directory.ok_or_else(|| {
        anyhow::anyhow!("No valid ACME directory obtained after trying all providers")
    })?;
    let account_cache_key = selected_account_cache_key.ok_or_else(|| {
        anyhow::anyhow!("No valid ACME account cache key obtained after trying all providers")
    })?;
    let certificate_cache_key = selected_certificate_cache_key.ok_or_else(|| {
        anyhow::anyhow!("No valid ACME certificate cache key obtained after trying all providers")
    })?;

    config.account.replace(acme_account.clone());
    if let Some(contact) = selected_contact {
        config.contact = contact;
    }
    if let Some(eab_key) = selected_eab_key {
        config.eab_key = eab_key;
    }
    if let Some(profile) = selected_profile {
        config.profile = profile;
    }

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

    emit_log(
        event_sink,
        ferron_observability::LogLevel::Debug,
        "ACME order created",
        &format!("ACME order created for domains: {domains}"),
        "ferron-tls-acme",
        vec![(
            "ferron.acme.domains",
            ferron_observability::LogAttributeValue::String(domains.clone()),
        )],
    );
    let mut order = match acme_account.new_order(&new_order).await {
        Ok(o) => o,
        Err(instant_acme::Error::Api(ref problem))
            if problem.r#type.as_deref()
                == Some("urn:ietf:params:acme:error:accountDoesNotExist") =>
        {
            emit_log(
                event_sink,
                ferron_observability::LogLevel::Warn,
                "ACME account recreated",
                &format!(
                    "ACME account not found on server for {directory}, recreating",
                    directory = directory
                ),
                "ferron-tls-acme",
                vec![
                    (
                        "ferron.acme.domains",
                        ferron_observability::LogAttributeValue::String(domains.clone()),
                    ),
                    (
                        "ferron.acme.directory",
                        ferron_observability::LogAttributeValue::String(directory.to_string()),
                    ),
                ],
            );
            config.account_cache.remove(&account_cache_key).await;
            let client_config = build_rustls_client_config(false)?;
            let account_builder = instant_acme::Account::builder_with_http(Box::new(
                HttpsClientForAcme::new(client_config),
            ));
            let new_account = create_new_account(
                config,
                &directory,
                &config.contact,
                config.eab_key.as_ref(),
                config.profile.as_deref(),
                account_builder,
                &account_cache_key,
                event_sink,
            )
            .await?;
            new_account.new_order(&new_order).await?
        }
        Err(e) => {
            emit_log(
                event_sink,
                ferron_observability::LogLevel::Error,
                "ACME order creation failed",
                &format!(
                    "Failed to create ACME order for {domains}: {}",
                    acme_error_to_string(&e)
                ),
                "ferron-tls-acme",
                vec![
                    (
                        "ferron.acme.domains",
                        ferron_observability::LogAttributeValue::String(domains.clone()),
                    ),
                    (
                        "error.message",
                        ferron_observability::LogAttributeValue::String(acme_error_to_string(&e)),
                    ),
                ],
            );
            return Err(Box::new(e));
        }
    };

    let mut dns_01_domains = Vec::new();
    let mut authorizations = order.authorizations();
    while let Some(auth) = authorizations.next().await {
        let mut auth = auth?;
        match auth.status {
            AuthorizationStatus::Pending => {}
            AuthorizationStatus::Valid => continue,
            _ => {
                emit_log(
                    event_sink,
                    ferron_observability::LogLevel::Error,
                    "ACME authorization failed",
                    &format!(
                        "ACME authorization failed — status: {:?}, domains: {domains}",
                        auth.status,
                    ),
                    "ferron-tls-acme",
                    vec![
                        (
                            "ferron.acme.domains",
                            ferron_observability::LogAttributeValue::String(domains.clone()),
                        ),
                        (
                            "ferron.acme.auth_status",
                            ferron_observability::LogAttributeValue::String(format!(
                                "{:?}",
                                auth.status
                            )),
                        ),
                    ],
                );
                return Err(anyhow::anyhow!("Invalid ACME authorization status").into());
            }
        }

        let mut challenge = auth
            .challenge(config.challenge_type.clone())
            .ok_or_else(|| {
                    emit_log(
                        event_sink,
                        ferron_observability::LogLevel::Error,
                        "ACME challenge type unsupported",
                        &format!(
                            "ACME server doesn't support the requested challenge type {:?} for {domains}",
                            config.challenge_type
                        ),
                        "ferron-tls-acme",
                        vec![
                            (
                                "ferron.acme.domains",
                                ferron_observability::LogAttributeValue::String(domains.clone()),
                            ),
                            (
                                "ferron.acme.challenge_type",
                                ferron_observability::LogAttributeValue::String(
                                    format!("{:?}", config.challenge_type),
                                ),
                            ),
                        ],
                    );
                    anyhow::anyhow!("The ACME server doesn't support the requested challenge type")
            })?;

        let identifier = match &challenge.identifier().identifier {
            Identifier::Dns(name) => name.to_string(),
            Identifier::Ip(ip) => ip.to_string(),
            _ => {
                emit_log(
                    event_sink,
                    ferron_observability::LogLevel::Error,
                    "ACME identifier type unsupported",
                    &format!(
                        "Unsupported ACME identifier type for {domains}: {:?}",
                        challenge.identifier().identifier
                    ),
                    "ferron-tls-acme",
                    vec![
                        (
                            "ferron.acme.domains",
                            ferron_observability::LogAttributeValue::String(domains.clone()),
                        ),
                        (
                            "ferron.acme.identifier_type",
                            ferron_observability::LogAttributeValue::String(format!(
                                "{:?}",
                                challenge.identifier().identifier
                            )),
                        ),
                    ],
                );
                return Err(anyhow::anyhow!("Unsupported ACME identifier type").into());
            }
        };

        let key_authorization = challenge.key_authorization();

        emit_log(
            event_sink,
            ferron_observability::LogLevel::Debug,
            "ACME challenge initiated",
            &format!(
                "ACME {:?} challenge initiated for {domains}",
                config.challenge_type
            ),
            "ferron-tls-acme",
            vec![
                (
                    "ferron.acme.domains",
                    ferron_observability::LogAttributeValue::String(domains.clone()),
                ),
                (
                    "ferron.acme.challenge_type",
                    ferron_observability::LogAttributeValue::String(format!(
                        "{:?}",
                        config.challenge_type
                    )),
                ),
            ],
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
                    let challenge_domain = format!("_acme-challenge.{identifier}");
                    let _ = dns_client
                        .delete_record(&challenge_domain, ferron_dns::DnsRecordType::TXT)
                        .await;

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

                    emit_log(
                        event_sink,
                        ferron_observability::LogLevel::Debug,
                        "ACME DNS-01 record created",
                        &format!("DNS-01 record created for {challenge_domain_log}, TTL {ttl}"),
                        "ferron-tls-acme",
                        vec![
                            (
                                "ferron.acme.dns_challenge_domain",
                                ferron_observability::LogAttributeValue::String(
                                    challenge_domain_log,
                                ),
                            ),
                            (
                                "ferron.acme.dns_ttl",
                                ferron_observability::LogAttributeValue::I64(ttl as i64),
                            ),
                        ],
                    );

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
            emit_log(
                event_sink,
                ferron_observability::LogLevel::Error,
                "ACME challenge ready failed",
                &format!(
                    "Failed to set ACME challenge ready for {domains}: {}",
                    acme_error_to_string(&err)
                ),
                "ferron-tls-acme",
                vec![
                    (
                        "ferron.acme.domains",
                        ferron_observability::LogAttributeValue::String(domains.clone()),
                    ),
                    (
                        "error.message",
                        ferron_observability::LogAttributeValue::String(acme_error_to_string(&err)),
                    ),
                ],
            );
            return Err(Box::new(err));
        };
        emit_log(
            event_sink,
            ferron_observability::LogLevel::Debug,
            "ACME challenge solved",
            &format!(
                "ACME {:?} challenge solved for {domains}",
                config.challenge_type
            ),
            "ferron-tls-acme",
            vec![
                (
                    "ferron.acme.domains",
                    ferron_observability::LogAttributeValue::String(domains.clone()),
                ),
                (
                    "ferron.acme.challenge_type",
                    ferron_observability::LogAttributeValue::String(format!(
                        "{:?}",
                        config.challenge_type
                    )),
                ),
            ],
        );
    }

    let order_status = match order.poll_ready(&RetryPolicy::default()).await {
        Ok(status) => status,
        Err(e) => {
            emit_log(
                event_sink,
                ferron_observability::LogLevel::Error,
                "ACME order finalization failed",
                &format!(
                    "Failed to finalize ACME order for {domains}: {}",
                    acme_error_to_string(&e)
                ),
                "ferron-tls-acme",
                vec![
                    (
                        "ferron.acme.domains",
                        ferron_observability::LogAttributeValue::String(domains.clone()),
                    ),
                    (
                        "error.message",
                        ferron_observability::LogAttributeValue::String(acme_error_to_string(&e)),
                    ),
                ],
            );
            return Err(Box::new(e));
        }
    };
    match order_status {
        OrderStatus::Ready => {}
        OrderStatus::Invalid => {
            emit_log(
                event_sink,
                ferron_observability::LogLevel::Error,
                "ACME order invalid",
                &format!(
                    "ACME order failed — status: invalid, domains: {domains}, reason: {}",
                    order.refresh().await.map_or_else(
                        |e| e.to_string(),
                        |s| s.error.as_ref().map_or(
                            "unknown (failed ACME challenge verification?)".to_string(),
                            |s| acme_error_to_string(&instant_acme::Error::Api(s.to_owned()))
                        )
                    )
                ),
                "ferron-tls-acme",
                vec![(
                    "ferron.acme.domains",
                    ferron_observability::LogAttributeValue::String(domains.clone()),
                )],
            );
            return Err(anyhow::anyhow!("ACME order is invalid").into());
        }
        _ => {
            emit_log(
                event_sink,
                ferron_observability::LogLevel::Error,
                "ACME order not ready",
                &format!("ACME order failed — status: {order_status:?}, domains: {domains}"),
                "ferron-tls-acme",
                vec![
                    (
                        "ferron.acme.domains",
                        ferron_observability::LogAttributeValue::String(domains.clone()),
                    ),
                    (
                        "ferron.acme.order_status",
                        ferron_observability::LogAttributeValue::String(format!(
                            "{order_status:?}"
                        )),
                    ),
                ],
            );
            return Err(anyhow::anyhow!("ACME order is not ready").into());
        }
    }

    let private_key_pem = match order.finalize().await {
        Ok(pem) => pem,
        Err(e) => {
            emit_log(
                event_sink,
                ferron_observability::LogLevel::Error,
                "ACME finalize failed",
                &format!(
                    "Failed to finalize ACME order for {domains}: {}",
                    acme_error_to_string(&e)
                ),
                "ferron-tls-acme",
                vec![
                    (
                        "ferron.acme.domains",
                        ferron_observability::LogAttributeValue::String(domains.clone()),
                    ),
                    (
                        "error.message",
                        ferron_observability::LogAttributeValue::String(acme_error_to_string(&e)),
                    ),
                ],
            );
            return Err(Box::new(e));
        }
    };
    let certificate_chain_pem = match order.poll_certificate(&RetryPolicy::default()).await {
        Ok(pem) => pem,
        Err(e) => {
            emit_log(
                event_sink,
                ferron_observability::LogLevel::Error,
                "ACME certificate obtain failed",
                &format!(
                    "Failed to obtain ACME certificate for {domains}: {}",
                    acme_error_to_string(&e)
                ),
                "ferron-tls-acme",
                vec![
                    (
                        "ferron.acme.domains",
                        ferron_observability::LogAttributeValue::String(domains.clone()),
                    ),
                    (
                        "error.message",
                        ferron_observability::LogAttributeValue::String(acme_error_to_string(&e)),
                    ),
                ],
            );
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
        Err(rustls_pki_types::pem::Error::Io(err)) => return Err(Box::new(err).into()),
        Err(err) => return Err(Box::new(std::io::Error::other(err)).into()),
    };

    let cache_data = CertificateCacheData {
        certificate_chain_pem: certificate_chain_pem.clone(),
        private_key_pem: private_key_pem.clone(),
    };

    // Store in cache
    if let Err(err) = config
        .certificate_cache
        .set(&certificate_cache_key, serde_json::to_vec(&cache_data)?)
        .await
    {
        emit_log(
            event_sink,
            ferron_observability::LogLevel::Warn,
            "ACME certificate cache save failed",
            &format!("Failed to save ACME certificate cache: {}", err),
            "ferron-tls-acme",
            vec![(
                "error.message",
                ferron_observability::LogAttributeValue::String(err.to_string()),
            )],
        );
    }

    // Install the cert
    install_certified_key(config, certs, private_key, &cache_data, event_sink).await?;

    config.account.replace(acme_account);

    cleanup_challenge_data(config, &dns_01_domains, event_sink).await;

    Ok(true)
}
