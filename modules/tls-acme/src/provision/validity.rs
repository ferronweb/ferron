use std::ops::Sub;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use rustls_pki_types::pem::PemObject;
use rustls_pki_types::CertificateDer;
use x509_parser::prelude::{FromDer, X509Certificate};

use crate::cache::{get_certificate_cache_key, CertificateCacheData};
use crate::config::AcmeConfig;

use super::install_certified_key;

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
    event_sink: &Arc<ferron_observability::CompositeEventSink>,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    // Check if currently loaded cert is still valid
    if let Some(certified_key) = config.certified_key_lock.read().await.as_deref() {
        if let Some(certificate) = certified_key.cert.first() {
            if let Some(acme_account) = &config.account {
                if let Ok(certificate_id) = cert_id_from_cert(certificate) {
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

    let certificate_cache_key =
        get_certificate_cache_key(&config.domains, config.profile.as_deref());
    if let Some(serialized_data) = config.certificate_cache.get(&certificate_cache_key).await {
        if let Ok(data) = serde_json::from_slice::<CertificateCacheData>(&serialized_data) {
            if let Ok(certs) = CertificateDer::pem_slice_iter(data.certificate_chain_pem.as_bytes())
                .collect::<Result<Vec<_>, _>>()
            {
                if let Some(certificate) = certs.first() {
                    let is_valid = if let Some(acme_account) = &config.account {
                        if let Ok(certificate_id) = cert_id_from_cert(certificate) {
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
                        if let Ok(private_key) = rustls_pki_types::PrivateKeyDer::from_pem_slice(
                            data.private_key_pem.as_bytes(),
                        ) {
                            install_certified_key(config, certs, private_key, &data, event_sink)
                                .await?;
                            return Ok(true);
                        }
                    }
                }
            }
        }
    }

    Ok(false)
}

fn cert_id_from_cert<'a>(
    certificate: &CertificateDer<'a>,
) -> Result<instant_acme::CertificateIdentifier<'a>, String> {
    // Implementation taken from `instant-acme` itself
    // (https://docs.rs/instant-acme/0.8.5/src/instant_acme/types.rs.html#875-903)
    let (_, parsed) = x509_parser::parse_x509_certificate(certificate.as_ref())
        .map_err(|e| format!("failed to parse x509 certificate: {e}"))?;

    let Some(authority_key_identifier) =
        parsed
            .iter_extensions()
            .find_map(|ext| match ext.parsed_extension() {
                x509_parser::extensions::ParsedExtension::AuthorityKeyIdentifier(aki_ext) => {
                    aki_ext
                        .key_identifier
                        .as_ref()
                        .map(|aki| rustls_pki_types::Der::from_slice(aki.0))
                }
                _ => None,
            })
    else {
        return Err("x509 certificate does not have an AKI extension".into());
    };

    Ok(instant_acme::CertificateIdentifier::new(
        authority_key_identifier,
        rustls_pki_types::Der::from_slice(parsed.tbs_certificate.raw_serial()),
    ))
}
