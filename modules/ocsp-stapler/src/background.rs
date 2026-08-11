//! Background OCSP fetching task and helpers owned by the ocsp-stapler module.
//!
//! This file contains the runtime-heavy HTTP client, OCSP parsing, caching, and
//! the long-running task that periodically fetches and refreshes OCSP
//! responses. Keeping this code in the module crate keeps the types crate
//! lightweight and free of networking/parsing dependencies.

use std::collections::HashMap;
use std::ops::Deref;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::Context as _;
use ferron_observability::{
    CompositeEventSink, Event, LogAttributeValue, LogEvent, LogLevel, MetricAttributeValue,
    MetricEvent, MetricType, MetricValue,
};
use hyper::body::Bytes;
use hyper::Request;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use parking_lot::RwLock;
use rasn::prelude::*;
use rasn_ocsp::{
    BasicOcspResponse, CertId, OcspRequest, OcspResponse, OcspResponseStatus,
    Request as RasnOcspRequest, TbsRequest,
};
use rustls_pki_types::CertificateDer;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

// Type alias for the OCSP cache to reduce type complexity
type OcspCache = Arc<RwLock<HashMap<Vec<u8>, Option<Vec<u8>>>>>;

/// Maps certificate leaf bytes to hostname for per-host OCSP metrics.
type OcspHostMap = Arc<RwLock<HashMap<Vec<u8>, String>>>;

#[derive(AsnType, Clone, Debug, Decode, Encode, PartialEq, Eq, Hash)]
pub struct RSASSAPSSParams {
    #[rasn(tag(context, 0))]
    pub hash_algorithm: Option<rasn_pkix::AlgorithmIdentifier>,
    #[rasn(tag(context, 1))]
    pub mask_gen_algorithm: Option<rasn_pkix::AlgorithmIdentifier>,
    #[rasn(tag(context, 2))]
    pub salt_length: Option<Integer>,
    #[rasn(tag(context, 3))]
    pub trailer_field: Option<Integer>,
}

/// Build an `HttpsConnector` with native certificate store and webpki-roots fallback
fn build_https_connector() -> Result<
    hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
    std::io::Error,
> {
    use rustls::ClientConfig;

    let mut root_store = rustls::RootCertStore::empty();
    let mut found_any = false;

    // Try native certs first
    match rustls_native_certs::load_native_certs() {
        cert_result if !cert_result.errors.is_empty() => {
            ferron_core::log_warn!(
                "native root CA certificate loading errors: {:?}",
                cert_result.errors
            );
        }
        cert_result if cert_result.certs.is_empty() => {
            ferron_core::log_warn!("no native root CA certificates found");
        }
        cert_result => {
            for cert in cert_result.certs {
                if let Err(err) = root_store.add(cert) {
                    ferron_core::log_warn!("native certificate parsing failed: {:?}", err);
                } else {
                    found_any = true;
                }
            }
        }
    }

    // Always add webpki-roots as fallback
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    if !found_any {
        ferron_core::log_warn!("using webpki-roots as fallback (no native root CAs available)");
    }

    if root_store.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no root certificates available",
        ));
    }

    let tls_config =
        ClientConfig::builder_with_provider(rustls::crypto::aws_lc_rs::default_provider().into())
            .with_safe_default_protocol_versions()
            .map_err(std::io::Error::other)?
            .with_root_certificates(root_store)
            .with_no_client_auth();

    Ok(hyper_rustls::HttpsConnectorBuilder::new()
        .with_tls_config(tls_config)
        .https_or_http()
        .enable_http1()
        .enable_http2()
        .build())
}

/// Verify the signature on the OCSP response using the issuer's public key.
#[inline]
fn verify_ocsp_signature(
    basic_response: &BasicOcspResponse,
    issuer_cert: &rasn_pkix::Certificate,
) -> anyhow::Result<()> {
    verify_signature(
        &basic_response.signature,
        &basic_response.signature_algorithm,
        &rasn::der::encode(&basic_response.tbs_response_data)
            .map_err(|e| anyhow::anyhow!("OCSP response signature verification failed: {e}"))?,
        issuer_cert,
    )
}

/// Verify a signature on the OCSP response using the issuer's public key.
fn verify_signature(
    signature: &rasn::types::BitString,
    signature_algorithm: &rasn_pkix::AlgorithmIdentifier,
    message: &[u8],
    issuer_cert: &rasn_pkix::Certificate,
) -> anyhow::Result<()> {
    let spki = &issuer_cert.tbs_certificate.subject_public_key_info;
    let alg: &dyn aws_lc_rs::signature::VerificationAlgorithm =
        match *signature_algorithm.algorithm.deref().deref() {
            // RSA + PKCS#1
            [1, 2, 840, 113549, 1, 1, 11] => &aws_lc_rs::signature::RSA_PKCS1_2048_8192_SHA256,
            [1, 2, 840, 113549, 1, 1, 12] => &aws_lc_rs::signature::RSA_PKCS1_2048_8192_SHA384,
            [1, 2, 840, 113549, 1, 1, 13] => &aws_lc_rs::signature::RSA_PKCS1_2048_8192_SHA512,
            #[cfg(not(feature = "fips"))]
            [1, 2, 840, 113549, 1, 1, 5] => {
                &aws_lc_rs::signature::RSA_PKCS1_1024_8192_SHA1_FOR_LEGACY_USE_ONLY
            }

            // RSA-PSS
            [1, 2, 840, 113549, 1, 1, 10] => {
                let params: Option<RSASSAPSSParams> = signature_algorithm
                    .parameters
                    .as_ref()
                    .and_then(|v| rasn::der::encode(&v).ok())
                    .and_then(|v| rasn::der::decode::<RSASSAPSSParams>(&v).ok());
                let halgorithm = params.and_then(|p| p.hash_algorithm);
                let algorithm_oid = halgorithm.as_ref().map(|a| &a.algorithm);
                let algorithm_oid_u32: Option<&[u32]> =
                    algorithm_oid.map(|oid| oid.as_ref());
                match algorithm_oid_u32 {
                    Some([2, 16, 840, 1, 101, 3, 4, 2, 1]) => {
                        &aws_lc_rs::signature::RSA_PSS_2048_8192_SHA256
                    }
                    Some([2, 16, 840, 1, 101, 3, 4, 2, 2]) => {
                        &aws_lc_rs::signature::RSA_PSS_2048_8192_SHA384
                    }
                    Some([2, 16, 840, 1, 101, 3, 4, 2, 3]) => {
                        &aws_lc_rs::signature::RSA_PSS_2048_8192_SHA512
                    }

                    _ => {
                        return Err(anyhow::anyhow!(
                            "Unsupported signature algorithm OID: {}",
                            signature_algorithm.algorithm
                        ))
                    }
                }
            }

            // Ed25519
            #[cfg(not(feature = "fips"))]
            [1, 3, 101, 112] => &aws_lc_rs::signature::ED25519,

            // ECDSA
            [1, 2, 840, 10045, 4, 3, algo] => {
                let curve_oid: Option<ObjectIdentifier> = signature_algorithm
                    .parameters
                    .as_ref()
                    .and_then(|v| rasn::der::encode(&v).ok())
                    .and_then(|v| rasn::der::decode::<ObjectIdentifier>(&v).ok());
                let curve_oid_u32: Option<&[u32]> = curve_oid.as_deref().map(|oid| oid.as_ref());
                match (curve_oid_u32, algo) {
                    // P-256
                    (Some([1, 2, 840, 10045, 3, 1, 7]), 2) => {
                        &aws_lc_rs::signature::ECDSA_P256_SHA256_ASN1
                    }
                    (Some([1, 2, 840, 10045, 3, 1, 7]), 3) => {
                        &aws_lc_rs::signature::ECDSA_P256_SHA384_ASN1
                    }
                    (Some([1, 2, 840, 10045, 3, 1, 7]), 4) => {
                        &aws_lc_rs::signature::ECDSA_P256_SHA512_ASN1
                    }

                    // P-384
                    (Some([1, 3, 132, 0, 34]), 2) => &aws_lc_rs::signature::ECDSA_P384_SHA256_ASN1,
                    (Some([1, 3, 132, 0, 34]), 3) => &aws_lc_rs::signature::ECDSA_P384_SHA384_ASN1,
                    (Some([1, 3, 132, 0, 34]), 4) => &aws_lc_rs::signature::ECDSA_P384_SHA512_ASN1,

                    // P-521
                    (Some([1, 3, 132, 0, 35]), 2) => &aws_lc_rs::signature::ECDSA_P521_SHA256_ASN1,
                    (Some([1, 3, 132, 0, 35]), 3) => &aws_lc_rs::signature::ECDSA_P521_SHA384_ASN1,
                    (Some([1, 3, 132, 0, 35]), 4) => &aws_lc_rs::signature::ECDSA_P521_SHA512_ASN1,

                    // secp256k1 (not common but handle just in case)
                    #[cfg(not(feature = "fips"))]
                    (Some([1, 3, 132, 0, 10]), 2) => {
                        &aws_lc_rs::signature::ECDSA_P256K1_SHA256_ASN1
                    }

                    _ => {
                        return Err(anyhow::anyhow!(
                            "Unsupported signature algorithm OID: {}",
                            signature_algorithm.algorithm
                        ))
                    }
                }
            }

            _ => {
                return Err(anyhow::anyhow!(
                    "Unsupported signature algorithm OID: {}",
                    signature_algorithm.algorithm
                ))
            }
        };

    let signature = signature.as_raw_slice();

    alg.verify_sig(spki.subject_public_key.as_raw_slice(), message, signature)
        .map_err(|_| anyhow::anyhow!("Signature verification failed"))?;

    Ok(())
}

/// Verify OCSP signature, trying certificates in the certs field if initial verification fails.
fn verify_ocsp_signature_with_certs_field(
    basic_response: &BasicOcspResponse,
    issuer_cert: &rasn_pkix::Certificate,
) -> anyhow::Result<()> {
    let Err(mut last_error) = verify_ocsp_signature(basic_response, issuer_cert) else {
        return Ok(());
    };

    if let Some(ref certs) = basic_response.certs {
        for cert in certs {
            let Ok(raw_tbs) = rasn::der::encode(&cert.tbs_certificate) else {
                // Invalid certificate?
                continue;
            };
            if verify_signature(
                &cert.signature_value,
                &cert.signature_algorithm,
                &raw_tbs,
                issuer_cert,
            )
            .is_err()
            {
                // The certificate is not signed by the issuer, skip verification
                continue;
            }

            let Some(extensions) = &cert.tbs_certificate.extensions else {
                // No extensions, no EKU, no OCSP...
                continue;
            };

            if !extensions.iter().any(|e| {
                if e.extn_id == rasn::types::Oid::JOINT_ISO_ITU_T_DS_CERTIFICATE_EXTENSION_AUTHORITY_EXT_KEY_USAGE {
                    let Ok(ekus_parsed) = rasn::der::decode::<rasn_pkix::ExtKeyUsageSyntax>(&e.extn_value) else {
                        return false;
                    };
                    ekus_parsed.iter().any(|eku| eku == rasn::types::Oid::ISO_IDENTIFIED_ORGANISATION_DOD_INTERNET_SECURITY_MECHANISMS_PKIX_KP_OCSP_SIGNING)
                } else {
                    false
                }
            }) {
                // The certificate does not have OCSP Extended Key Usage, skip verification
                continue;
            }

            let Err(new_last_error) = verify_ocsp_signature(basic_response, cert) else {
                return Ok(());
            };
            last_error = new_last_error;
        }
    }

    Err(last_error)
}

/// Hash the given data using the specified hash algorithm OID.
///
/// This is used for computing the issuer name and key hashes in OCSP requests and responses.
fn hash_oid(data: impl AsRef<[u8]>, oid: ObjectIdentifier) -> anyhow::Result<Vec<u8>> {
    let mut ctx = if oid == *rasn::types::Oid::JOINT_ISO_ITU_T_COUNTRY_US_ORGANIZATION_GOV_CSOR_NIST_ALGORITHMS_HASH_SHA256 {
        aws_lc_rs::digest::Context::new(&aws_lc_rs::digest::SHA256)
    } else if oid == *rasn::types::Oid::JOINT_ISO_ITU_T_COUNTRY_US_ORGANIZATION_GOV_CSOR_NIST_ALGORITHMS_HASH_SHA384 {
        aws_lc_rs::digest::Context::new(&aws_lc_rs::digest::SHA384)
    } else if oid == *rasn::types::Oid::JOINT_ISO_ITU_T_COUNTRY_US_ORGANIZATION_GOV_CSOR_NIST_ALGORITHMS_HASH_SHA512 {
        aws_lc_rs::digest::Context::new(&aws_lc_rs::digest::SHA512)
    } else if oid == *rasn::types::Oid::ISO_IDENTIFIED_ORGANISATION_OIW_SECSIG_ALGORITHM_SHA1 {
        #[cfg(not(feature = "fips"))]
        {
            aws_lc_rs::digest::Context::new(&aws_lc_rs::digest::SHA1_FOR_LEGACY_USE_ONLY)
        }
        #[cfg(feature = "fips")]
        {
            return Err(anyhow::anyhow!(
                "Unsupported hash algorithm OID in OCSP response: {}",
                oid
            ));
        }
    } else {
        return Err(anyhow::anyhow!(
            "Unsupported hash algorithm OID in OCSP response: {}",
            oid
        ))
    };
    ctx.update(data.as_ref());
    Ok(ctx.finish().as_ref().to_vec())
}

/// Verify that the SingleResponse matches the leaf and issuer certs.
///
/// This includes checking the issuer name and key hashes, and the serial number. This
/// is important to prevent replay attacks where an attacker could use a valid OCSP response
/// for a different certificate.
fn verify_single_res(
    single_res: &rasn_ocsp::SingleResponse,
    leaf_cert: &rasn_pkix::Certificate,
    issuer_cert: &rasn_pkix::Certificate,
) -> anyhow::Result<()> {
    if single_res.cert_id.issuer_name_hash.as_ref()
        != hash_oid(
            rasn::der::encode(&issuer_cert.tbs_certificate.subject)?,
            single_res.cert_id.hash_algorithm.algorithm.clone(),
        )?
    {
        return Err(anyhow::anyhow!(
            "Issuer name hash mismatch in OCSP response"
        ));
    }

    if single_res.cert_id.issuer_key_hash.as_ref()
        != hash_oid(
            issuer_cert
                .tbs_certificate
                .subject_public_key_info
                .subject_public_key
                .as_raw_slice(),
            single_res.cert_id.hash_algorithm.algorithm.clone(),
        )?
    {
        return Err(anyhow::anyhow!("Issuer key hash mismatch in OCSP response"));
    }

    if single_res.cert_id.serial_number != leaf_cert.tbs_certificate.serial_number {
        return Err(anyhow::anyhow!("Serial number mismatch in OCSP response"));
    }

    // Check if the response falls between the issuer's valid time range,
    // allowing for a 60-second clock skew to account for network latency and time differences.
    let now_with_skew =
        chrono::DateTime::<chrono::Utc>::from(SystemTime::now() + Duration::from_secs(60));
    let now = chrono::DateTime::<chrono::Utc>::from(SystemTime::now());
    if single_res.this_update > now_with_skew
        || single_res.next_update.as_ref().is_some_and(|nu| *nu < now)
    {
        return Err(anyhow::anyhow!("OCSP response is not current"));
    }

    Ok(())
}

pub async fn background_ocsp_task(
    mut receiver: mpsc::UnboundedReceiver<Vec<CertificateDer<'static>>>,
    cache: OcspCache,
    host_map: OcspHostMap,
    cancel_token: CancellationToken,
    event_sink: Option<Arc<CompositeEventSink>>,
) {
    // Track next-update times per cert
    let mut next_updates: HashMap<Vec<u8>, SystemTime> = HashMap::new();
    // Track known cert chains
    let mut known_certs: HashMap<Vec<u8>, Vec<CertificateDer<'static>>> = HashMap::new();

    // Build HTTPS client with native certificate store and webpki-roots fallback
    let Ok(https_connector) = build_https_connector() else {
        emit_log(
            &event_sink,
            LogLevel::Info,
            "OCSP HTTPS initialization failed",
            "Failed to initialize HTTPS for OCSP background task",
            "ferron-ocsp-stapler",
            Vec::new(),
        );
        return;
    };

    let client = Client::builder(TokioExecutor::new())
        .build::<_, http_body_util::Full<Bytes>>(https_connector);

    let sleep_duration = Duration::from_secs(60); // default check interval

    emit_log(
        &event_sink,
        LogLevel::Debug,
        "OCSP background task started",
        "OCSP background task started",
        "ferron-ocsp-stapler",
        Vec::new(),
    );

    loop {
        let received_certified_key = tokio::select! {
            _ = cancel_token.cancelled() => {
                emit_log(&event_sink, LogLevel::Info, "OCSP background task shutting down", "OCSP background task shutting down", "ferron-ocsp-stapler", Vec::new());
                return;
            }
            _ = tokio::time::sleep(sleep_duration) => None,
            res = receiver.recv() => match res {
                Some(chain) => Some(chain),
                None => return, // channel closed
            },
        };

        if let Some(chain) = received_certified_key {
            if let Some(leaf) = chain.first() {
                let key: Vec<u8> = leaf.to_vec();
                if !known_certs.contains_key(&key) {
                    let ident = cert_identifier(&chain);
                    emit_log(
                        &event_sink,
                        LogLevel::Debug,
                        "OCSP fetch triggered",
                        &format!("OCSP fetch triggered for certificate {ident}"),
                        "ferron-ocsp-stapler",
                        vec![(
                            "ferron.ocsp.cert.subject",
                            LogAttributeValue::String(ident.clone()),
                        )],
                    );
                    known_certs.insert(key.clone(), chain.clone());
                    // Trigger immediate fetch (use time in the past to ensure it is fetched immediately)
                    next_updates.insert(key, SystemTime::now() - std::time::Duration::from_secs(1));
                }
            }
        }

        let now = SystemTime::now();
        let updates_to_fetch: Vec<Vec<u8>> = next_updates
            .iter()
            .filter(|(_, next_update)| **next_update <= now)
            .map(|(key, _)| key.clone())
            .collect();

        for key in updates_to_fetch {
            if let Some(cert) = known_certs.get(&key) {
                let start = std::time::Instant::now();
                match fetch_ocsp_response(&client, cert).await {
                    Ok(Some((response_der, next_update_time))) => {
                        let duration = start.elapsed().as_secs_f64();
                        let ident = cert_identifier(cert);
                        let next_update_ts = next_update_time
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs() as i64;
                        let primary_san = cert
                            .first()
                            .and_then(|leaf| {
                                rasn::der::decode::<rasn_pkix::Certificate>(leaf).ok()
                            })
                            .as_ref()
                            .and_then(|leaf| {
                                let Some(extensions) = &leaf.tbs_certificate.extensions else {
                                    return None;
                                };

                                extensions.iter().find_map(|e| {
                                    if e.extn_id == rasn::types::Oid::JOINT_ISO_ITU_T_DS_CERTIFICATE_EXTENSION_AUTHORITY_EXT_KEY_USAGE {
                                        let Ok(sans_parsed) = rasn::der::decode::<rasn_pkix::SubjectAltName>(&e.extn_value) else {
                                            return None;
                                        };
                                        sans_parsed.first().cloned()
                                    } else {
                                        None
                                    }
                                })
                            })
                            .and_then(|san| match san {
                                rasn_pkix::GeneralName::DnsName(dns) => {
                                    Some(dns.to_string())
                                },
                                rasn_pkix::GeneralName::IpAddress(ip) => {
                                    if let Ok(ipv6_octets) = {
                                        let v: &[u8] = &ip;
                                        let v: Result<[u8; 16], _> = v.try_into();
                                        v
                                    } {
                                        Some(std::net::IpAddr::from(ipv6_octets).to_string())
                                    } else if let Ok(ipv4_octets) = {
                                        let v: &[u8] = &ip;
                                        let v: Result<[u8; 4], _> = v.try_into();
                                        v
                                    } {
                                        Some(std::net::IpAddr::from(ipv4_octets).to_string())
                                    } else {
                                        None
                                    }
                                },
                                _ => None
                            });
                        let primary_san_formatted = if let Some(san) = &primary_san {
                            let mut fmtd = String::new();
                            fmtd.push_str(" (");
                            fmtd.push_str(san);
                            fmtd.push(')');
                            fmtd
                        } else {
                            "".to_owned()
                        };

                        let mut log_attributes = vec![
                            (
                                "ferron.ocsp.cert.subject",
                                LogAttributeValue::String(ident.clone()),
                            ),
                            (
                                "ferron.ocsp.next_update",
                                LogAttributeValue::I64(next_update_ts),
                            ),
                        ];
                        if let Some(san) = primary_san {
                            log_attributes.push((
                                "ferron.ocsp.cert.primary_san",
                                LogAttributeValue::String(san),
                            ));
                        }
                        emit_log(
                            &event_sink,
                            LogLevel::Info,
                            "OCSP response cached",
                            &format!(
                                "OCSP response cached for {ident}{primary_san_formatted}, valid until {}",
                                chrono::DateTime::<chrono::Utc>::from(next_update_time)
                                    .format("%Y-%m-%d %H:%M:%S")
                            ),
                            "ferron-ocsp-stapler",
                            log_attributes,
                        );
                        emit_metric(
                            &event_sink,
                            "ferron.ocsp.fetches_total",
                            MetricValue::U64(1),
                            MetricType::Counter,
                            Some("{fetch}"),
                            Some("Total OCSP fetch attempts"),
                            vec![
                                (
                                    "ferron.ocsp.status",
                                    MetricAttributeValue::StaticStr("success"),
                                ),
                                (
                                    "ferron.host",
                                    MetricAttributeValue::String(
                                        host_map
                                            .read()
                                            .get(&key)
                                            .cloned()
                                            .unwrap_or_else(|| "_global".to_string()),
                                    ),
                                ),
                            ],
                        );
                        emit_metric(
                            &event_sink,
                            "ferron.ocsp.fetch_duration_seconds",
                            MetricValue::F64(duration),
                            MetricType::Histogram(None),
                            Some("s"),
                            Some("Time to fetch OCSP response"),
                            vec![(
                                "ferron.host",
                                MetricAttributeValue::String(
                                    host_map
                                        .read()
                                        .get(&key)
                                        .cloned()
                                        .unwrap_or_else(|| "_global".to_string()),
                                ),
                            )],
                        );

                        cache.write().insert(key.clone(), Some(response_der));
                        next_updates.insert(key, next_update_time);
                    }
                    Ok(None) => {
                        let ident = cert_identifier(cert);
                        emit_log(
                            &event_sink,
                            LogLevel::Debug,
                            "OCSP stapling skipped",
                            &format!(
                                "OCSP stapling skipped — \
                                 no OCSP URL or incomplete chain in certificate {ident}"
                            ),
                            "ferron-ocsp-stapler",
                            vec![
                                ("ferron.ocsp.cert.subject", LogAttributeValue::String(ident)),
                                (
                                    "ferron.ocsp.reason",
                                    LogAttributeValue::StaticStr("no_ocsp_url_or_incomplete_chain"),
                                ),
                            ],
                        );
                        emit_metric(
                            &event_sink,
                            "ferron.ocsp.fetches_total",
                            MetricValue::U64(1),
                            MetricType::Counter,
                            Some("{fetch}"),
                            Some("Total OCSP fetch attempts"),
                            vec![
                                (
                                    "ferron.ocsp.status",
                                    MetricAttributeValue::StaticStr("skipped"),
                                ),
                                (
                                    "ferron.host",
                                    MetricAttributeValue::String(
                                        host_map
                                            .read()
                                            .get(&key)
                                            .cloned()
                                            .unwrap_or_else(|| "_global".to_string()),
                                    ),
                                ),
                            ],
                        );
                        // No OCSP possible (e.g. no OCSP URL in cert)
                        cache.write().insert(key.clone(), None);
                        next_updates.remove(&key);
                    }
                    Err(e) => {
                        let duration = start.elapsed().as_secs_f64();
                        let ident = cert_identifier(cert);
                        emit_log(
                            &event_sink,
                            LogLevel::Warn,
                            "OCSP fetch failed",
                            &format!("OCSP fetch failed for {ident}: {e}"),
                            "ferron-ocsp-stapler",
                            vec![
                                ("ferron.ocsp.cert.subject", LogAttributeValue::String(ident)),
                                ("error.message", LogAttributeValue::String(e.to_string())),
                            ],
                        );
                        emit_metric(
                            &event_sink,
                            "ferron.ocsp.fetches_total",
                            MetricValue::U64(1),
                            MetricType::Counter,
                            Some("{fetch}"),
                            Some("Total OCSP fetch attempts"),
                            vec![
                                (
                                    "ferron.ocsp.status",
                                    MetricAttributeValue::StaticStr("error"),
                                ),
                                (
                                    "ferron.host",
                                    MetricAttributeValue::String(
                                        host_map
                                            .read()
                                            .get(&key)
                                            .cloned()
                                            .unwrap_or_else(|| "_global".to_string()),
                                    ),
                                ),
                            ],
                        );
                        emit_metric(
                            &event_sink,
                            "ferron.ocsp.fetch_duration_seconds",
                            MetricValue::F64(duration),
                            MetricType::Histogram(None),
                            Some("s"),
                            Some("Time to fetch OCSP response"),
                            vec![(
                                "ferron.host",
                                MetricAttributeValue::String(
                                    host_map
                                        .read()
                                        .get(&key)
                                        .cloned()
                                        .unwrap_or_else(|| "_global".to_string()),
                                ),
                            )],
                        );
                        // Retry later with randomness to avoid refresh storms
                        let jitter = rand::random_range(100..=500);
                        next_updates.insert(key, now + Duration::from_secs(jitter));
                    }
                }
            }
        }

        let stapled_count = cache.read().iter().filter(|(_, v)| v.is_some()).count();
        emit_metric(
            &event_sink,
            "ferron.ocsp.cached_certificates",
            MetricValue::U64(known_certs.len() as u64),
            MetricType::Gauge,
            Some("{certificate}"),
            Some("Number of certificates in OCSP cache"),
            vec![],
        );
        emit_metric(
            &event_sink,
            "ferron.ocsp.certificates_with_stapling",
            MetricValue::U64(stapled_count as u64),
            MetricType::Gauge,
            Some("{certificate}"),
            Some("Number of certificates with valid OCSP stapling"),
            vec![],
        );
    }
}

fn emit_log(
    event_sink: &Option<Arc<CompositeEventSink>>,
    level: LogLevel,
    summary: &'static str,
    message: &str,
    target: &'static str,
    attributes: Vec<(&'static str, LogAttributeValue)>,
) {
    if let Some(ref sink) = event_sink {
        sink.emit(Event::Log(LogEvent {
            level,
            message: message.to_string(),
            summary: summary.into(),
            target,
            attributes,
            trace_context: None,
        }));
    }
}

fn emit_metric(
    event_sink: &Option<Arc<CompositeEventSink>>,
    name: &'static str,
    value: MetricValue,
    ty: MetricType,
    unit: Option<&'static str>,
    description: Option<&'static str>,
    attributes: Vec<(&'static str, MetricAttributeValue)>,
) {
    if let Some(ref sink) = event_sink {
        sink.emit(Event::Metric(MetricEvent {
            name,
            attributes,
            ty,
            value,
            unit,
            description,
            trace_context: None,
        }));
    }
}

fn cert_identifier(chain: &[CertificateDer<'_>]) -> String {
    if let Some(leaf) = chain.first() {
        if let Ok(cert) = rasn::der::decode::<rasn_pkix::Certificate>(leaf) {
            let rasn_pkix::Name::RdnSequence(s) = cert.tbs_certificate.subject;
            if let Some(sf) = s.first() {
                for satv in sf.to_vec() {
                    if satv.r#type
                        == rasn::types::Oid::JOINT_ISO_ITU_T_DS_ATTRIBUTE_TYPE_COMMON_NAME
                    {
                        if let Ok(der) = rasn::der::encode(&satv.value) {
                            if let Ok(cn) = rasn::der::decode::<rasn_pkix::CommonName>(&der) {
                                return String::from_utf8_lossy(cn.as_bytes()).to_string();
                            }
                        }
                    }
                }
            }

            // Fallback: first 8 bytes of SHA-256 SPKI hash
            let pub_key = &cert
                .tbs_certificate
                .subject_public_key_info
                .subject_public_key
                .as_raw_slice();
            let mut hash_ctx = aws_lc_rs::digest::Context::new(&aws_lc_rs::digest::SHA256);
            hash_ctx.update(pub_key);
            let hash = hash_ctx.finish().as_ref().to_vec();
            return format!("<SPKI {}>", hex::encode(&hash[..4]));
        }
    }
    "<unknown>".to_string()
}

async fn fetch_ocsp_response(
    client: &Client<
        hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
        http_body_util::Full<Bytes>,
    >,
    chain: &[CertificateDer<'_>],
) -> anyhow::Result<Option<(Vec<u8>, SystemTime)>> {
    // Try SHA-256 first (preferred algorithm)
    let response = fetch_ocsp_response_inner(client, chain, true).await;

    // If SHA-256 succeeded, return immediately (do not downgrade to SHA-1)
    if response.is_ok() {
        return response;
    }

    #[cfg(not(feature = "fips"))]
    {
        // Only try SHA-1 fallback for specific error types
        let should_try_sha1 = response.as_ref().is_err_and(|e| {
            let e_message = e.to_string();
            e_message.starts_with("OCSP request failed with status ")
                || e_message.starts_with("Failed to decode OCSP response:")
                || e_message.starts_with("OCSP response status unsuccessful:")
        });

        if should_try_sha1 {
            if let Ok(sha1_response) = fetch_ocsp_response_inner(client, chain, false).await {
                return Ok(sha1_response);
            }
        }
    }

    response
}

async fn fetch_ocsp_response_inner(
    client: &Client<
        hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
        http_body_util::Full<Bytes>,
    >,
    chain: &[CertificateDer<'_>],
    use_sha256: bool,
) -> anyhow::Result<Option<(Vec<u8>, SystemTime)>> {
    if chain.len() < 2 {
        return Ok(None);
    }

    let leaf = &chain[0];
    let issuer = &chain[1];

    let leaf_cert = rasn::der::decode::<rasn_pkix::Certificate>(leaf)
        .map_err(|e| anyhow::anyhow!("Failed to parse leaf cert: {e}"))?;
    let issuer_cert = rasn::der::decode::<rasn_pkix::Certificate>(issuer)
        .map_err(|e| anyhow::anyhow!("Failed to parse issuer cert: {e}"))?;

    let Some(ocsp_url) = extract_ocsp_url(&leaf_cert) else {
        return Ok(None);
    };

    let req_der = create_ocsp_request(&leaf_cert, &issuer_cert, use_sha256)?;

    let req = Request::builder()
        .method("POST")
        .uri(&ocsp_url)
        .header("Content-Type", "application/ocsp-request")
        .body(http_body_util::Full::new(Bytes::from(req_der)))
        .with_context(|| format!("Failed to build OCSP request for {ocsp_url}"))?;

    let res = client.request(req).await?;
    if !res.status().is_success() {
        return Err(anyhow::anyhow!(
            "OCSP request failed with status {} for URL: {ocsp_url}",
            res.status()
        ));
    }

    use http_body_util::BodyExt;
    let body_bytes = res.collect().await?.to_bytes();
    let response_der = body_bytes.to_vec();

    let response: OcspResponse = rasn::der::decode(&response_der)
        .map_err(|e| anyhow::anyhow!("Failed to decode OCSP response: {e}"))?;

    if response.status != OcspResponseStatus::Successful {
        return Err(anyhow::anyhow!(
            "OCSP response status unsuccessful: {}",
            response.status.identifier()
        ));
    }

    let response_bytes = response
        .bytes
        .ok_or_else(|| anyhow::anyhow!("No response bytes in OCSP response"))?;

    if response_bytes.r#type
        != ObjectIdentifier::new(vec![1, 3, 6, 1, 5, 5, 7, 48, 1, 1])
            .ok_or_else(|| anyhow::anyhow!("Invalid OCSP basic response OID"))?
    {
        return Err(anyhow::anyhow!("Unsupported OCSP response type"));
    }

    let basic_response: BasicOcspResponse = rasn::der::decode(&response_bytes.response)
        .map_err(|e| anyhow::anyhow!("Failed to decode BasicOcspResponse: {e}"))?;

    verify_ocsp_signature_with_certs_field(&basic_response, &issuer_cert)?;

    // Compute next_update across all single responses
    let mut min_next_update: Option<SystemTime> = None;
    for single_res in basic_response.tbs_response_data.responses {
        verify_single_res(&single_res, &leaf_cert, &issuer_cert)?;

        let next_update = single_res.next_update.map(SystemTime::from);
        if let Some(mut nu) = next_update {
            // Safety margin: 25% of validity period + jitter
            let this_update = SystemTime::from(single_res.this_update);
            let validity = nu
                .duration_since(this_update)
                .unwrap_or_else(|_| Duration::from_secs(0));
            let margin = validity / 4 + Duration::from_secs(rand::random_range(0..=300));

            if nu.checked_sub(margin).unwrap_or(nu) > SystemTime::now() {
                nu = nu.checked_sub(margin).unwrap_or(nu);
            }
            min_next_update = Some(match min_next_update {
                Some(min) if nu < min => nu,
                None => nu,
                _ => min_next_update.ok_or(anyhow::anyhow!("Failed to compute next update"))?,
            });
        }
    }

    let next_update =
        min_next_update.unwrap_or_else(|| SystemTime::now() + Duration::from_secs(300));
    Ok(Some((response_der, next_update)))
}

fn extract_ocsp_url(cert: &rasn_pkix::Certificate) -> Option<String> {
    let extensions = cert.tbs_certificate.extensions.as_ref()?;

    extensions.iter().find_map(|e| {
        if e.extn_id == rasn::oid!("1.3.6.1.5.5.7.1.1") {
            let Ok(aia_parsed) = rasn::der::decode::<rasn_pkix::AuthorityInfoAccessSyntax>(&e.extn_value) else {
                return None;
            };
            aia_parsed.iter().find(|aia| aia.access_method == rasn::types::Oid::ISO_IDENTIFIED_ORGANISATION_DOD_INTERNET_SECURITY_MECHANISMS_PKIX_AD_OCSP).and_then(|aia| {
                match &aia.access_location {
                    rasn_pkix::GeneralName::Uri(uri) => {
                        Some(uri.to_string())
                    },
                    _ => None
                }
            })
        } else {
            None
        }
    })
}

fn create_ocsp_request(
    leaf: &rasn_pkix::Certificate,
    issuer: &rasn_pkix::Certificate,
    use_sha256: bool,
) -> anyhow::Result<Vec<u8>> {
    // Hash issuer subject DN
    #[cfg(not(feature = "fips"))]
    let mut issuer_name_ctx = if use_sha256 {
        aws_lc_rs::digest::Context::new(&aws_lc_rs::digest::SHA256)
    } else {
        aws_lc_rs::digest::Context::new(&aws_lc_rs::digest::SHA1_FOR_LEGACY_USE_ONLY)
    };
    #[cfg(feature = "fips")]
    let mut issuer_name_ctx = aws_lc_rs::digest::Context::new(&aws_lc_rs::digest::SHA256);
    issuer_name_ctx.update(&rasn::der::encode(&issuer.tbs_certificate.subject)?);
    let issuer_name_hash = issuer_name_ctx.finish().as_ref().to_vec();

    // Hash issuer public key value (excluding tag/length per RFC 6960)
    let pub_key_bytes = &issuer
        .tbs_certificate
        .subject_public_key_info
        .subject_public_key
        .as_raw_slice();
    #[cfg(not(feature = "fips"))]
    let mut issuer_key_ctx = if use_sha256 {
        aws_lc_rs::digest::Context::new(&aws_lc_rs::digest::SHA256)
    } else {
        aws_lc_rs::digest::Context::new(&aws_lc_rs::digest::SHA1_FOR_LEGACY_USE_ONLY)
    };
    #[cfg(feature = "fips")]
    let mut issuer_key_ctx = aws_lc_rs::digest::Context::new(&aws_lc_rs::digest::SHA256);
    issuer_key_ctx.update(pub_key_bytes);
    let issuer_key_hash = issuer_key_ctx.finish().as_ref().to_vec();

    // Serial number
    let serial_int = leaf.tbs_certificate.serial_number.clone();

    let cert_id = CertId {
        hash_algorithm: rasn_pkix::AlgorithmIdentifier {
            #[cfg(not(feature = "fips"))]
            algorithm: if use_sha256 {
                rasn::types::Oid::JOINT_ISO_ITU_T_COUNTRY_US_ORGANIZATION_GOV_CSOR_NIST_ALGORITHMS_HASH_SHA256
                    .to_owned()
            } else {
                rasn::types::Oid::ISO_IDENTIFIED_ORGANISATION_OIW_SECSIG_ALGORITHM_SHA1.to_owned()
            },
            #[cfg(feature = "fips")]
            algorithm: rasn::types::Oid::JOINT_ISO_ITU_T_COUNTRY_US_ORGANIZATION_GOV_CSOR_NIST_ALGORITHMS_HASH_SHA256
                .to_owned(),
            parameters: None,
        },
        issuer_name_hash: rasn::types::OctetString::from(issuer_name_hash),
        issuer_key_hash: rasn::types::OctetString::from(issuer_key_hash),
        serial_number: serial_int,
    };

    let req = OcspRequest {
        tbs_request: TbsRequest {
            version: rasn::types::Integer::from(0), // v1
            requestor_name: None,
            request_list: vec![RasnOcspRequest {
                req_cert: cert_id,
                single_request_extensions: None,
            }],
            request_extensions: None,
        },
        optional_signature: None,
    };

    rasn::der::encode(&req).map_err(|e| anyhow::anyhow!(e))
}
