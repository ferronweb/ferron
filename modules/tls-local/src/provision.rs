use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};
use std::net::IpAddr;
use std::sync::Arc;

use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, IsCa, KeyIdMethod, KeyPair, SanType,
};
use rustls::sign::CertifiedKey;
use rustls_pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
use time::{Duration, OffsetDateTime};

use crate::cache::LocalTlsCache;
use ferron_core::config::ServerConfigurationHostFilters;

pub fn provision_local_cert(
    cache: &LocalTlsCache,
    filters: &ServerConfigurationHostFilters,
) -> Result<Arc<CertifiedKey>, Box<dyn std::error::Error>> {
    // Normalize and hash the SAN set
    let san_set = normalize_san_set(filters);
    let san_hash = compute_san_hash(&san_set);

    // Try to load cached leaf cert
    if let (Some(cert_pem), Some(key_pem)) = (
        cache.get_leaf_cert(&san_hash),
        cache.get_leaf_key(&san_hash),
    ) {
        if let Ok(certified_key) = parse_certified_key(&cert_pem, &key_pem) {
            // Check if certificate is still valid (simplified for now)
            return Ok(Arc::new(certified_key));
        }
    }

    // Need to generate/load CA
    let (ca_params, ca_key_pair) = get_or_generate_ca(cache)?;
    ferron_core::log_info!(
        "Local CA certificate can be found in \"{}\". Import the CA certificate into your \
    system trust store to trust the generated certificates.",
        cache.ca_path().display()
    );

    // Generate leaf cert
    let mut leaf_cert_params = CertificateParams::default();
    leaf_cert_params.not_before = OffsetDateTime::now_utc().saturating_sub(Duration::days(1));
    leaf_cert_params.not_after = OffsetDateTime::now_utc().saturating_add(Duration::days(365));
    leaf_cert_params.distinguished_name = DistinguishedName::new();
    leaf_cert_params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "Ferron Local TLS");
    leaf_cert_params.subject_alt_names = Vec::new();
    for san in san_set {
        leaf_cert_params
            .subject_alt_names
            .push(if let Ok(ip) = san.parse::<IpAddr>() {
                SanType::IpAddress(ip)
            } else {
                SanType::DnsName(san.try_into().map_err(|_| "Invalid SAN entry")?)
            });
    }
    leaf_cert_params.is_ca = IsCa::NoCa;

    let leaf_key_pair = KeyPair::generate()?;
    let issuer = rcgen::Issuer::from_params(&ca_params, ca_key_pair);
    let leaf_cert = leaf_cert_params.signed_by(&leaf_key_pair, &issuer)?;
    let cert_pem = leaf_cert.pem();
    let key_pem = leaf_key_pair.serialize_pem();

    // Cache the leaf cert
    cache.save_leaf(&san_hash, &cert_pem, &key_pem)?;

    let certified_key = parse_certified_key(&cert_pem, &key_pem)?;
    Ok(Arc::new(certified_key))
}

fn normalize_san_set(filters: &ServerConfigurationHostFilters) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    if let Some(ref host) = filters.host {
        set.insert(host.clone());
    }
    if let Some(ref ip) = filters.ip {
        set.insert(ip.to_string());
    }

    // If it's localhost, also add 127.0.0.1 and ::1 for convenience
    if set.contains("localhost") || set.contains("127.0.0.1") || set.contains("::1") {
        set.insert("localhost".to_string());
        set.insert("127.0.0.1".to_string());
        set.insert("::1".to_string());
    }

    set
}

fn compute_san_hash(set: &BTreeSet<String>) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for san in set {
        san.hash(&mut hasher);
    }
    format!("{:x}", hasher.finish())
}

fn get_or_generate_ca(
    cache: &LocalTlsCache,
) -> Result<(CertificateParams, KeyPair), Box<dyn std::error::Error>> {
    if let (Some(_cert_pem), Some(key_pem)) = (cache.get_ca_cert(), cache.get_ca_key()) {
        let key_pair = KeyPair::from_pem(&key_pem)?;
        let mut params = CertificateParams::default();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.distinguished_name = DistinguishedName::new();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "Ferron Local Root CA");
        params.key_identifier_method = KeyIdMethod::Sha256;
        return Ok((params, key_pair));
    }

    // Generate new CA
    let mut params = CertificateParams::default();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.distinguished_name = DistinguishedName::new();
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "Ferron Local Root CA");
    params.not_before = OffsetDateTime::now_utc().saturating_sub(Duration::days(1));
    params.not_after = OffsetDateTime::now_utc().saturating_add(Duration::days(3650)); // 10 years
    params.key_identifier_method = KeyIdMethod::Sha256;

    let key_pair = KeyPair::generate()?;
    let ca_cert = params.clone().self_signed(&key_pair)?;
    let cert_pem = ca_cert.pem();
    let key_pem = key_pair.serialize_pem();

    cache.save_ca(&cert_pem, &key_pem)?;

    Ok((params, key_pair))
}

fn parse_certified_key(
    cert_pem: &str,
    key_pem: &str,
) -> Result<CertifiedKey, Box<dyn std::error::Error>> {
    let cert_chain =
        CertificateDer::pem_reader_iter(&mut cert_pem.as_bytes()).collect::<Result<Vec<_>, _>>()?;
    let private_key = PrivateKeyDer::from_pem_slice(key_pem.as_bytes())?;

    let signing_key = rustls::crypto::aws_lc_rs::sign::any_supported_type(&private_key)?;
    Ok(CertifiedKey::new(cert_chain, signing_key))
}
