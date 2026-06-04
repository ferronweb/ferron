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
    CompositeEventSink, Event, LogEvent, LogLevel, MetricAttributeValue, MetricEvent, MetricType,
    MetricValue,
};
use hyper::body::Bytes;
use hyper::Request;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use num_bigint::BigInt;
use parking_lot::RwLock;
use rasn::prelude::*;
use rasn_ocsp::{
    BasicOcspResponse, CertId, OcspRequest, OcspResponse, OcspResponseStatus,
    Request as RasnOcspRequest, TbsRequest,
};
use rustls_pki_types::CertificateDer;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use x509_parser::prelude::*;

// Type alias for the OCSP cache to reduce type complexity
type OcspCache = Arc<RwLock<HashMap<Vec<u8>, Option<Vec<u8>>>>>;

// ---------------------------------------------------------------------------
// HTTPS client construction
// ---------------------------------------------------------------------------

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
fn verify_ocsp_signature(
    basic_response: &BasicOcspResponse,
    issuer_cert: &X509Certificate,
) -> anyhow::Result<()> {
    let spki = issuer_cert.public_key();
    let alg: &dyn aws_lc_rs::signature::VerificationAlgorithm =
        match *basic_response.signature_algorithm.algorithm.deref().deref() {
            // RSA + PKCS#1
            [1, 2, 840, 113549, 1, 1, 11] => &aws_lc_rs::signature::RSA_PKCS1_2048_8192_SHA256,
            [1, 2, 840, 113549, 1, 1, 12] => &aws_lc_rs::signature::RSA_PKCS1_2048_8192_SHA384,
            [1, 2, 840, 113549, 1, 1, 13] => &aws_lc_rs::signature::RSA_PKCS1_2048_8192_SHA512,
            [1, 2, 840, 113549, 1, 1, 5] => {
                &aws_lc_rs::signature::RSA_PKCS1_1024_8192_SHA1_FOR_LEGACY_USE_ONLY
            }

            // Ed25519
            [1, 3, 101, 112] => &aws_lc_rs::signature::ED25519,

            // ECDSA
            [1, 2, 840, 10045, 4, 3, algo] => {
                // Get curve OID
                let curve_oid: Option<ObjectIdentifier> = issuer_cert
                    .public_key()
                    .algorithm
                    .parameters
                    .as_ref()
                    .and_then(|v| rasn::der::decode::<ObjectIdentifier>(v.as_bytes()).ok());
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

                    // secp256k1 (not common in OCSP but handle just in case)
                    (Some([1, 3, 132, 0, 10]), 2) => {
                        &aws_lc_rs::signature::ECDSA_P256K1_SHA256_ASN1
                    }

                    _ => {
                        return Err(anyhow::anyhow!(
                            "Unsupported OCSP signature algorithm OID: {}",
                            basic_response.signature_algorithm.algorithm
                        ))
                    }
                }
            }

            _ => {
                return Err(anyhow::anyhow!(
                    "Unsupported OCSP signature algorithm OID: {}",
                    basic_response.signature_algorithm.algorithm
                ))
            }
        };

    let signature = basic_response.signature.as_raw_slice();

    alg.verify_sig(
        spki.subject_public_key.data.as_ref(),
        &rasn::der::encode(&basic_response.tbs_response_data)
            .map_err(|e| anyhow::anyhow!("OCSP response signature verification failed: {e}"))?,
        signature,
    )
    .map_err(|_| anyhow::anyhow!("OCSP response signature verification failed"))?;

    Ok(())
}

/// Verify OCSP signature, trying certificates in the certs field if initial verification fails.
fn verify_ocsp_signature_with_certs_field(
    basic_response: &BasicOcspResponse,
    issuer_cert: &X509Certificate,
) -> anyhow::Result<()> {
    let Err(mut last_error) = verify_ocsp_signature(basic_response, issuer_cert) else {
        return Ok(());
    };

    if let Some(ref certs) = basic_response.certs {
        for cert in certs {
            // Re-encode the cert to DER and parse with x509-parser to get
            // an X509Certificate struct for signature verification
            let Ok(cert_der) = rasn::der::encode(cert) else {
                continue;
            };
            let Ok((_, cert)) = X509Certificate::from_der(&cert_der) else {
                continue;
            };

            if cert
                .verify_signature(Some(issuer_cert.public_key()))
                .is_err()
            {
                // The certificate is not signed by the issuer, skip verification
                continue;
            }

            if !cert.extensions().iter().any(|e| {
                let parsed = e.parsed_extension();
                match parsed {
                    ParsedExtension::ExtendedKeyUsage(eku) => eku.ocsp_signing,
                    _ => false,
                }
            }) {
                // The certificate does not have OCSP Extended Key Usage, skip verification
                continue;
            }

            let Err(new_last_error) = verify_ocsp_signature(basic_response, &cert) else {
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
        aws_lc_rs::digest::Context::new(&aws_lc_rs::digest::SHA1_FOR_LEGACY_USE_ONLY)
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
    leaf_cert: &X509Certificate,
    issuer_cert: &X509Certificate,
) -> anyhow::Result<()> {
    // Check for issue name hash
    if single_res.cert_id.issuer_name_hash.as_ref()
        != hash_oid(
            issuer_cert.subject().as_raw(),
            single_res.cert_id.hash_algorithm.algorithm.clone(),
        )?
    {
        return Err(anyhow::anyhow!(
            "Issuer name hash mismatch in OCSP response"
        ));
    }

    // Check for issue key hash
    if single_res.cert_id.issuer_key_hash.as_ref()
        != hash_oid(
            issuer_cert.public_key().subject_public_key.data.as_ref(),
            single_res.cert_id.hash_algorithm.algorithm.clone(),
        )?
    {
        return Err(anyhow::anyhow!("Issuer key hash mismatch in OCSP response"));
    }

    // Check for serial number
    let serial_number = &leaf_cert.tbs_certificate.serial;
    let serial_int = BigInt::from_biguint(num_bigint::Sign::Plus, serial_number.to_owned());
    if single_res.cert_id.serial_number != rasn::types::Integer::from(serial_int) {
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
            "Failed to initialize HTTPS for OCSP background task",
            "ferron_ocsp",
        );
        return;
    };

    let client = Client::builder(TokioExecutor::new())
        .build::<_, http_body_util::Full<Bytes>>(https_connector);

    let sleep_duration = Duration::from_secs(60); // default check interval

    emit_log(
        &event_sink,
        LogLevel::Info,
        "OCSP background task started",
        "ferron_ocsp",
    );

    loop {
        let received_certified_key = tokio::select! {
            _ = cancel_token.cancelled() => {
                emit_log(&event_sink, LogLevel::Info, "OCSP background task shutting down", "ferron_ocsp");
                return;
            }
            _ = tokio::time::sleep(sleep_duration) => None,
            res = receiver.recv() => match res {
                Some(chain) => Some(chain),
                None => return, // channel closed
            },
        };

        // Process newly received cert
        if let Some(chain) = received_certified_key {
            if let Some(leaf) = chain.first() {
                let key: Vec<u8> = leaf.to_vec();
                if !known_certs.contains_key(&key) {
                    let ident = cert_identifier(&chain);
                    emit_log(
                        &event_sink,
                        LogLevel::Debug,
                        &format!("OCSP fetch triggered for certificate {ident}"),
                        "ferron_ocsp",
                    );
                    known_certs.insert(key.clone(), chain.clone());
                    // Trigger immediate fetch (use time in the past to ensure it is fetched immediately)
                    next_updates.insert(key, SystemTime::now() - std::time::Duration::from_secs(1));
                }
            }
        }

        // Fetch OCSP for certs whose next_update has passed
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
                        emit_log(
                            &event_sink,
                            LogLevel::Debug,
                            &format!(
                                "OCSP response cached for {ident}, valid until {}",
                                chrono::DateTime::<chrono::Utc>::from(next_update_time)
                                    .format("%Y-%m-%d %H:%M:%S")
                            ),
                            "ferron_ocsp",
                        );
                        emit_metric(
                            &event_sink,
                            "ferron.ocsp.fetches_total",
                            MetricValue::U64(1),
                            MetricType::Counter,
                            Some("{fetch}"),
                            Some("Total OCSP fetch attempts"),
                            vec![(
                                "ferron.ocsp.status",
                                MetricAttributeValue::StaticStr("success"),
                            )],
                        );
                        emit_metric(
                            &event_sink,
                            "ferron.ocsp.fetch_duration_seconds",
                            MetricValue::F64(duration),
                            MetricType::Histogram(None),
                            Some("s"),
                            Some("Time to fetch OCSP response"),
                            vec![],
                        );

                        cache.write().insert(key.clone(), Some(response_der));
                        next_updates.insert(key, next_update_time);
                    }
                    Ok(None) => {
                        let ident = cert_identifier(cert);
                        emit_log(
                            &event_sink,
                            LogLevel::Debug,
                            &format!(
                                "OCSP stapling skipped — \
                                no OCSP URL or incomplete chain in certificate {ident}"
                            ),
                            "ferron_ocsp",
                        );
                        emit_metric(
                            &event_sink,
                            "ferron.ocsp.fetches_total",
                            MetricValue::U64(1),
                            MetricType::Counter,
                            Some("{fetch}"),
                            Some("Total OCSP fetch attempts"),
                            vec![(
                                "ferron.ocsp.status",
                                MetricAttributeValue::StaticStr("skipped"),
                            )],
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
                            &format!("OCSP fetch failed for {ident}: {e}"),
                            "ferron_ocsp",
                        );
                        emit_metric(
                            &event_sink,
                            "ferron.ocsp.fetches_total",
                            MetricValue::U64(1),
                            MetricType::Counter,
                            Some("{fetch}"),
                            Some("Total OCSP fetch attempts"),
                            vec![(
                                "ferron.ocsp.status",
                                MetricAttributeValue::StaticStr("error"),
                            )],
                        );
                        emit_metric(
                            &event_sink,
                            "ferron.ocsp.fetch_duration_seconds",
                            MetricValue::F64(duration),
                            MetricType::Histogram(None),
                            Some("s"),
                            Some("Time to fetch OCSP response"),
                            vec![],
                        );
                        // Retry later with randomness to avoid refresh storms
                        let jitter = rand::random_range(100..=500);
                        next_updates.insert(key, now + Duration::from_secs(jitter));
                    }
                }
            }
        }

        // Emit gauge metrics each cycle
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
    message: &str,
    target: &'static str,
) {
    if let Some(ref sink) = event_sink {
        sink.emit(Event::Log(LogEvent {
            level,
            message: message.to_string(),
            target,
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
        if let Ok((_, cert)) = X509Certificate::from_der(leaf) {
            if let Some(cn) = cert.subject().iter_common_name().next() {
                if let Ok(cn_str) = cn.as_str() {
                    return cn_str.to_string();
                }
            }
            // Fallback: first 8 bytes of SHA-256 SPKI hash
            let pub_key = &cert.public_key().subject_public_key.data;
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

    // Return the original SHA-256 error or success
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

    let (_, leaf_cert) = X509Certificate::from_der(leaf)
        .map_err(|e| anyhow::anyhow!("Failed to parse leaf cert: {e}"))?;
    let (_, issuer_cert) = X509Certificate::from_der(issuer)
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

    // Parse response
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

fn extract_ocsp_url(cert: &X509Certificate) -> Option<String> {
    for ext in cert.extensions() {
        if let x509_parser::extensions::ParsedExtension::AuthorityInfoAccess(aia) =
            ext.parsed_extension()
        {
            for access_desc in &aia.accessdescs {
                if access_desc.access_method
                    == x509_parser::oid_registry::OID_PKIX_ACCESS_DESCRIPTOR_OCSP
                {
                    if let x509_parser::extensions::GeneralName::URI(uri) =
                        access_desc.access_location
                    {
                        return Some(uri.to_string());
                    }
                }
            }
        }
    }
    None
}

fn create_ocsp_request(
    leaf: &X509Certificate,
    issuer: &X509Certificate,
    use_sha256: bool,
) -> anyhow::Result<Vec<u8>> {
    // Hash issuer subject DN
    let mut issuer_name_ctx = if use_sha256 {
        aws_lc_rs::digest::Context::new(&aws_lc_rs::digest::SHA256)
    } else {
        aws_lc_rs::digest::Context::new(&aws_lc_rs::digest::SHA1_FOR_LEGACY_USE_ONLY)
    };
    issuer_name_ctx.update(issuer.subject().as_raw());
    let issuer_name_hash = issuer_name_ctx.finish().as_ref().to_vec();

    // Hash issuer public key value (excluding tag/length per RFC 6960)
    let pub_key_bytes = &issuer.public_key().subject_public_key.data;
    let mut issuer_key_ctx = if use_sha256 {
        aws_lc_rs::digest::Context::new(&aws_lc_rs::digest::SHA256)
    } else {
        aws_lc_rs::digest::Context::new(&aws_lc_rs::digest::SHA1_FOR_LEGACY_USE_ONLY)
    };
    issuer_key_ctx.update(pub_key_bytes);
    let issuer_key_hash = issuer_key_ctx.finish().as_ref().to_vec();

    // Serial number
    let serial_number = &leaf.tbs_certificate.serial;
    let serial_int = rasn::types::Integer::from(BigInt::from_biguint(
        num_bigint::Sign::Plus,
        serial_number.to_owned(),
    ));

    let cert_id = CertId {
        hash_algorithm: rasn_pkix::AlgorithmIdentifier {
            algorithm: if use_sha256 {
                rasn::types::Oid::JOINT_ISO_ITU_T_COUNTRY_US_ORGANIZATION_GOV_CSOR_NIST_ALGORITHMS_HASH_SHA256
                    .to_owned()
            } else {
                rasn::types::Oid::ISO_IDENTIFIED_ORGANISATION_OIW_SECSIG_ALGORITHM_SHA1.to_owned()
            },
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
