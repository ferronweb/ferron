use std::sync::Arc;
use std::time::{Duration, SystemTime};

use rustls_pki_types::pem::PemObject;
use rustls_pki_types::CertificateDer;

use crate::cache::{get_account_cache_key, get_certificate_cache_key, CertificateCacheData};
use crate::config::AcmeConfig;
use crate::provision::account::HttpsClientForAcme;

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
    let x509_certificate = rasn::der::decode::<rasn_pkix::Certificate>(certificate)?;
    let validity = &x509_certificate.tbs_certificate.validity;
    let time_to_expiration_delta = match &validity.not_after {
        rasn_pkix::Time::Utc(t) => {
            t.signed_duration_since(chrono::DateTime::<chrono::Utc>::from(SystemTime::now()))
        }
        rasn_pkix::Time::General(t) => {
            t.signed_duration_since(chrono::DateTime::<chrono::Utc>::from(SystemTime::now()))
        }
    };
    if let Ok(time_to_expiration) = time_to_expiration_delta.to_std() {
        let valid_duration_delta = match (&validity.not_before, &validity.not_after) {
            (rasn_pkix::Time::Utc(b), rasn_pkix::Time::Utc(a)) => a.signed_duration_since(b),
            (rasn_pkix::Time::Utc(b), rasn_pkix::Time::General(a)) => a.signed_duration_since(b),
            (rasn_pkix::Time::General(b), rasn_pkix::Time::Utc(a)) => a.signed_duration_since(b),
            (rasn_pkix::Time::General(b), rasn_pkix::Time::General(a)) => {
                a.signed_duration_since(b)
            }
        };
        let time_before_expiration = if let Ok(valid_duration) = valid_duration_delta.to_std() {
            (valid_duration.as_secs() / 2).min(SECONDS_BEFORE_RENEWAL)
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
            if let Ok(certificate_id) = cert_id_from_cert(certificate) {
                if let Some(renewal_info) = get_ari_renewal_info(config, &certificate_id).await {
                    if SystemTime::now() < renewal_info.0.suggested_window.start {
                        return Ok(true);
                    }
                } else if check_certificate_validity(certificate, None)? {
                    return Ok(true);
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
                    let is_valid = if let Ok(certificate_id) = cert_id_from_cert(certificate) {
                        if let Some(renewal_info) =
                            get_ari_renewal_info(config, &certificate_id).await
                        {
                            SystemTime::now() < renewal_info.0.suggested_window.start
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
    let parsed = rasn::der::decode::<rasn_pkix::Certificate>(certificate.as_ref())
        .map_err(|e| format!("failed to parse x509 certificate: {e}"))?;

    let Some(extensions) = &parsed.tbs_certificate.extensions else {
        return Err("x509 certificate does not have any extensions".into());
    };

    let Some(authority_key_identifier) = extensions.iter().find_map(|ext| {
        if ext.extn_id
            == rasn::types::Oid::JOINT_ISO_ITU_T_DS_CERTIFICATE_EXTENSION_AUTHORITY_KEY_IDENTIFIER
        {
            let Ok(aki_parsed) =
                rasn::der::decode::<rasn_pkix::AuthorityKeyIdentifier>(&ext.extn_value)
            else {
                return None;
            };
            let ki = aki_parsed.key_identifier?;
            // Use raw key identifier bytes (without OCTET STRING tag/length) per RFC 9773 §4.1.
            // This is the DER-encoded OCTET STRING value without the tag and length bytes.
            Some(rustls_pki_types::Der::from(ki.as_ref().to_vec()))
        } else {
            None
        }
    }) else {
        return Err("x509 certificate does not have an AKI extension".into());
    };

    let serial_number_der = get_raw_x509_serial(certificate.as_ref());
    Ok(instant_acme::CertificateIdentifier::new(
        authority_key_identifier,
        rustls_pki_types::Der::from(serial_number_der),
    ))
}

async fn get_ari_renewal_info(
    config: &AcmeConfig,
    certificate_id: &instant_acme::CertificateIdentifier<'_>,
) -> Option<(instant_acme::RenewalInfo, std::time::Duration)> {
    let mut acme_accounts = Vec::new();
    if let Some(a) = config.account.clone() {
        acme_accounts.push(a);
    }
    if acme_accounts.is_empty() {
        // ACME account may be installer later on into configuration during the provisioning,
        // so temporary obtain ACME accounts from cache
        let list = config.provider_list.read().await;
        for provider in std::iter::once(&list.primary).chain(list.fallbacks.iter()) {
            let account_cache_key = get_account_cache_key(&provider.contact, &provider.directory);
            if let Some(credentials_bytes) = config.account_cache.get(&account_cache_key).await {
                if let Ok(credentials) =
                    serde_json::from_slice::<instant_acme::AccountCredentials>(&credentials_bytes)
                {
                    if let Ok(account) = instant_acme::Account::builder_with_http(Box::new(
                        HttpsClientForAcme::new(config.rustls_client_config.clone()),
                    ))
                    .from_credentials(credentials)
                    .await
                    {
                        acme_accounts.push(account);
                    }
                }
            }
        }
    }
    for acme_account in acme_accounts {
        if let Ok(renewal_info) = acme_account.renewal_info(certificate_id).await {
            return Some(renewal_info);
        }
    }
    None
}

fn get_raw_x509_serial(x509_cert_der: &[u8]) -> Vec<u8> {
    // Extract the raw DER-encoded serialNumber value bytes (without tag and length)
    // directly from the original certificate DER, without re-encoding via rasn.
    // This preserves the exact bytes as encoded (including leading 0x00 for
    // positive integers with MSB set) per RFC 9773 §4.1: "DER-encoded Serial
    // Number field (without the tag and length bytes)".
    //
    // ASN.1 structure:
    //   Certificate  ::= SEQUENCE { tbsCertificate TBSCertificate, ... }
    //   TBSCertificate ::= SEQUENCE { [0] EXPLICIT Version DEFAULT v1,
    //                                 serialNumber CertificateSerialNumber,
    //                                 ... }
    // So we parse: outer SEQUENCE -> inner TBSCertificate SEQUENCE -> optional
    // [0] version -> INTEGER serialNumber. We return the INTEGER value bytes.

    fn parse_length(data: &[u8]) -> Option<(usize, usize)> {
        if data.is_empty() {
            return None;
        }
        let first = data[0];
        if first & 0x80 == 0 {
            // Short form
            Some((first as usize, 1))
        } else {
            let num_bytes = (first & 0x7F) as usize;
            // 0 means indefinite length, not allowed in DER
            if num_bytes == 0 || num_bytes > 4 {
                return None;
            }
            if data.len() < 1 + num_bytes {
                return None;
            }
            let mut len = 0usize;
            for &b in &data[1..1 + num_bytes] {
                len = (len << 8) | b as usize;
            }
            Some((len, 1 + num_bytes))
        }
    }

    let mut pos = 0usize;

    // Outer Certificate SEQUENCE (tag 0x30)
    if x509_cert_der.get(pos) != Some(&0x30) {
        return vec![];
    }
    pos += 1;
    let (outer_len, outer_len_bytes) = match parse_length(&x509_cert_der[pos..]) {
        Some(v) => v,
        None => return vec![],
    };
    pos += outer_len_bytes;
    let outer_end = pos + outer_len;
    if outer_end > x509_cert_der.len() {
        return vec![];
    }

    // TBSCertificate SEQUENCE (tag 0x30), first element of Certificate
    if x509_cert_der.get(pos) != Some(&0x30) {
        return vec![];
    }
    pos += 1;
    let (tbs_len, tbs_len_bytes) = match parse_length(&x509_cert_der[pos..]) {
        Some(v) => v,
        None => return vec![],
    };
    pos += tbs_len_bytes;
    let tbs_start = pos;
    let tbs_end = tbs_start + tbs_len;
    if tbs_end > x509_cert_der.len() || tbs_end > outer_end {
        return vec![];
    }

    // Optional [0] EXPLICIT version (tag 0xA0, constructed)
    if pos < tbs_end && x509_cert_der[pos] == 0xA0 {
        pos += 1;
        let (v_len, v_len_bytes) = match parse_length(&x509_cert_der[pos..]) {
            Some(v) => v,
            None => return vec![],
        };
        pos += v_len_bytes;
        if pos + v_len > tbs_end {
            return vec![];
        }
        pos += v_len;
    }

    // serialNumber INTEGER (tag 0x02)
    if pos >= tbs_end || x509_cert_der[pos] != 0x02 {
        return vec![];
    }
    pos += 1;
    let (serial_len, serial_len_bytes) = match parse_length(&x509_cert_der[pos..]) {
        Some(v) => v,
        None => return vec![],
    };
    pos += serial_len_bytes;
    if pos + serial_len > tbs_end {
        return vec![];
    }

    // Return the INTEGER value bytes (without tag and length), preserving
    // the exact encoding including any leading 0x00 for positive integers.
    x509_cert_der[pos..pos + serial_len].to_vec()
}
