//! Certificate subject identification and SAN extraction helpers.

use aws_lc_rs::digest::Context;
use rasn::prelude::*;
use rasn_pkix::{Certificate, SubjectAltName};
use rustls_pki_types::CertificateDer;

/// Human-readable identifier for a certificate chain, used in logs and metrics.
pub(crate) fn cert_identifier(chain: &[CertificateDer<'_>]) -> String {
    if let Some(leaf) = chain.first() {
        if let Ok(cert) = rasn::der::decode::<Certificate>(leaf) {
            let rasn_pkix::Name::RdnSequence(s) = cert.tbs_certificate.subject;
            if let Some(sf) = s.first() {
                if let Some(cn) = sf
                    .to_vec()
                    .iter()
                    .filter(|satv| {
                        satv.r#type == Oid::JOINT_ISO_ITU_T_DS_ATTRIBUTE_TYPE_COMMON_NAME
                    })
                    .filter_map(|satv| {
                        // Transform Any -> DER -> CommonName
                        rasn::der::encode(&satv.value)
                            .ok()
                            .and_then(|der| rasn::der::decode::<rasn_pkix::CommonName>(&der).ok())
                    })
                    .next()
                {
                    return String::from_utf8_lossy(cn.as_bytes()).to_string();
                }
            }

            // Fallback: first 4 bytes of SHA-256 SPKI hash
            let pub_key = &cert
                .tbs_certificate
                .subject_public_key_info
                .subject_public_key
                .as_raw_slice();
            let mut hash_ctx = Context::new(&aws_lc_rs::digest::SHA256);
            hash_ctx.update(pub_key);
            let hash = hash_ctx.finish().as_ref().to_vec();
            return format!("<SPKI {}>", hex::encode(&hash[..4]));
        }
    }
    "<unknown>".to_string()
}

/// Best-effort extraction of the certificate's primary SAN (DNS name or IP).
///
/// Returns `None` when the leaf certificate has no Subject Alternative Name
/// extension or no suitable entry.
pub(crate) fn primary_san(chain: &[CertificateDer<'_>]) -> Option<String> {
    let leaf = chain.first()?;
    let leaf_cert = rasn::der::decode::<Certificate>(leaf).ok()?;
    let extensions = leaf_cert.tbs_certificate.extensions.as_ref()?;

    let san_ext = extensions
        .iter()
        .find(|e| e.extn_id == Oid::JOINT_ISO_ITU_T_DS_CERTIFICATE_EXTENSION_SUBJECT_ALT_NAME)?;
    let sans = rasn::der::decode::<SubjectAltName>(&san_ext.extn_value).ok()?;
    let san = sans.first().cloned()?;

    match san {
        rasn_pkix::GeneralName::DnsName(dns) => Some(dns.to_string()),
        rasn_pkix::GeneralName::IpAddress(ip) => {
            let octets: &[u8] = &ip;
            match <[u8; 16]>::try_from(octets) {
                Ok(v6) => Some(std::net::IpAddr::from(v6).to_string()),
                Err(_) => <[u8; 4]>::try_from(octets)
                    .ok()
                    .map(|v4| std::net::IpAddr::from(v4).to_string()),
            }
        }
        _ => None,
    }
}
