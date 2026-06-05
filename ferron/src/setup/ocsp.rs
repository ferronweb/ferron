use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::Context as _;
use ferron_common::logging::LogMessage;
use hyper::Request;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use num_bigint::BigInt;
use rasn::prelude::*;
use rasn_ocsp::BasicOcspResponse;
use rasn_ocsp::{CertId, OcspRequest, OcspResponse, OcspResponseStatus, Request as OcspInnerRequest, TbsRequest};
use rustls::client::WebPkiServerVerifier;
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls_pki_types::CertificateDer;
use rustls_platform_verifier::BuilderVerifierExt;
use sha1::{Digest, Sha1};
use sha2::Sha256;
use std::ops::Deref;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use x509_parser::prelude::*;

type OcspCache = Arc<RwLock<HashMap<Vec<u8>, Option<Arc<CertifiedKey>>>>>;

#[derive(Debug)]
pub struct OcspStapler {
  inner: Arc<dyn ResolvesServerCert>,
  cache: OcspCache,
  sender: async_channel::Sender<CertifiedKey>,
  cancel_token: CancellationToken,
}

impl OcspStapler {
  pub fn new(
    inner: Arc<dyn ResolvesServerCert>,
    runtime: &tokio::runtime::Runtime,
    logging_tx: Vec<async_channel::Sender<LogMessage>>,
  ) -> Self {
    let (sender, receiver) = async_channel::unbounded();
    let cache = Arc::new(RwLock::new(HashMap::new()));
    let cancel_token = CancellationToken::new();

    let stapler = Self {
      inner,
      cache,
      sender,
      cancel_token: cancel_token.clone(),
    };

    runtime.spawn(background_ocsp_task(
      receiver,
      stapler.cache.clone(),
      cancel_token,
      logging_tx,
    ));

    stapler
  }

  pub fn preload(&self, key: Arc<CertifiedKey>) {
    if !key.cert.is_empty() {
      // Add to cache immediately (even without OCSP) to track it, or just trigger fetch
      let _ = self.sender.send_blocking((*key).clone());
    }
  }

  pub async fn stop(&self) {
    self.cancel_token.cancel();
  }
}

impl ResolvesServerCert for OcspStapler {
  fn resolve(&self, client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
    let original_key = self.inner.resolve(client_hello)?;
    if let Some(leaf) = original_key.cert.first() {
      // Check cache
      //
      // If blocking_read() method is used when only Tokio is used, the program would panic on resolving a TLS certificate.
      #[cfg(feature = "runtime-monoio")]
      let cache = self.cache.blocking_read();
      #[cfg(feature = "runtime-tokio")]
      let cache = futures_executor::block_on(async { self.cache.read().await });

      if let Some(cached_key_option) = cache.get(&leaf.to_vec()) {
        if let Some(cached_key) = cached_key_option.as_ref() {
          // If cached key has OCSP, return it.
          // Note: We might want to check if it's expired here, but the background task handles cleanup/refresh.
          // For simplicity, we return what's in cache.
          if cached_key.ocsp.is_some() {
            return Some(cached_key.clone());
          }
        }
        // If cached key has no OCSP, don't trigger fetch.
      } else {
        // Not in cache or no OCSP yet. Trigger fetch.
        let _ = self.sender.send_blocking((*original_key).clone());
      }
    }
    Some(original_key)
  }
}

async fn background_ocsp_task(
  receiver: async_channel::Receiver<CertifiedKey>,
  cache: OcspCache,
  cancel_token: CancellationToken,
  logging_tx: Vec<async_channel::Sender<LogMessage>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
  // Track next update times
  let mut next_updates: HashMap<Vec<u8>, SystemTime> = HashMap::new();
  // Track known cert chains
  let mut known_certs: HashMap<Vec<u8>, CertifiedKey> = HashMap::new();

  // Create HTTP client
  let tls_config_builder =
    match rustls::ClientConfig::builder_with_provider(rustls::crypto::aws_lc_rs::default_provider().into())
      .with_safe_default_protocol_versions()
    {
      Ok(builder) => builder,
      Err(e) => {
        for tx in &logging_tx {
          let _ = tx
            .send(LogMessage::new(
              format!("Failed to create TLS config builder for OCSP stapling: {e}"),
              true,
            ))
            .await;
        }
        return Err(e.into());
      }
    };
  let https_connector = hyper_rustls::HttpsConnectorBuilder::new()
    .with_tls_config(
      (if let Ok(client_config) = BuilderVerifierExt::with_platform_verifier(tls_config_builder.clone()) {
        client_config
      } else {
        tls_config_builder.with_webpki_verifier(
          match WebPkiServerVerifier::builder(Arc::new(rustls::RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
          }))
          .build()
          {
            Ok(verifier) => verifier,
            Err(e) => {
              for tx in &logging_tx {
                let _ = tx
                  .send(LogMessage::new(
                    format!("Failed to create TLS verifier for OCSP stapling: {e}"),
                    true,
                  ))
                  .await;
              }
              return Err(e.into());
            }
          },
        )
      })
      .with_no_client_auth(),
    )
    .https_or_http()
    .enable_http1()
    .build();

  let client =
    Client::builder(TokioExecutor::new()).build::<_, http_body_util::Full<hyper::body::Bytes>>(https_connector);

  loop {
    let mut sleep_duration = Duration::from_secs(60); // Default check interval

    // Calculate time to next update
    let now = SystemTime::now();
    for next_update in next_updates.values() {
      if let Ok(duration) = next_update.duration_since(now) {
        if duration < sleep_duration {
          sleep_duration = duration;
        }
      } else {
        // Already expired, refresh immediately (or very soon)
        sleep_duration = Duration::from_secs(1);
      }
    }

    let received_certified_key = tokio::select! {
      _ = cancel_token.cancelled() => Err(anyhow::anyhow!("Cancelled"))?,
      _ = tokio::time::sleep(sleep_duration) => None,
      res = receiver.recv() => match res {
        Ok(chain) => Some(chain),
        Err(e) => Err(e)?, // Channel closed
      }
    };

    if let Some(certified_key) = received_certified_key {
      let chain = &certified_key.cert;
      if let Some(leaf) = chain.first() {
        let key = leaf.to_vec();
        if !known_certs.contains_key(&key) {
          known_certs.insert(key.clone(), certified_key);
          // Trigger immediate update for new cert
          next_updates.insert(key, SystemTime::now());
        }
      }
    }

    // Process updates
    let now = SystemTime::now();
    let mut updates_to_fetch = Vec::new();
    for (key, next_update) in &next_updates {
      if *next_update <= now {
        updates_to_fetch.push(key.clone());
      }
    }

    for key in updates_to_fetch {
      if let Some(certified_key) = known_certs.get(&key) {
        match fetch_ocsp_response(&client, &certified_key.cert).await {
          Ok(Some((response, next_update_time))) => {
            let mut new_certified_key = certified_key.clone();
            new_certified_key.ocsp = Some(response.clone());
            cache
              .write()
              .await
              .insert(certified_key.cert[0].to_vec(), Some(Arc::new(new_certified_key)));
            next_updates.insert(key, next_update_time);
          }
          Ok(None) => {
            // Don't retry OCSP stapling
            cache.write().await.insert(certified_key.cert[0].to_vec(), None);
            next_updates.remove(&key);
          }
          Err(e) => {
            // Log error
            for tx in &logging_tx {
              let _ = tx.send(LogMessage::new(format!("OCSP fetch failed: {e}"), true)).await;
            }
            // Retry later; with some randomness to avoid refresh storm.
            next_updates.insert(key, now + Duration::from_secs(rand::random_range(100..=500)));
            continue;
          }
        };
      }
    }
  }
}

async fn fetch_ocsp_response(
  client: &Client<
    hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
    http_body_util::Full<hyper::body::Bytes>,
  >,
  chain: &[CertificateDer<'_>],
) -> anyhow::Result<Option<(Vec<u8>, SystemTime)>> {
  // Try SHA-256 first (preferred algorithm)
  let response = fetch_ocsp_response_inner(client, chain, true).await;

  // If SHA-256 succeeded, return immediately (do not downgrade to SHA-1)
  if response.is_ok() {
    return response;
  }

  // Only try SHA-1 fallback for specific error types observed in the wild
  let should_try_sha1 = match &response {
    Err(e) => {
      let e_message = e.to_string();
      e_message.contains("OCSP request failed with status")
        || e_message.contains("Failed to decode OCSP response")
        || e_message.contains("OCSP response status unsuccessful")
    }
    _ => false,
  };

  if should_try_sha1 {
    if let Ok(sha1_response) = fetch_ocsp_response_inner(client, chain, false).await {
      return Ok(sha1_response);
    }
  }

  // Return the original SHA-256 result (error or success)
  response
}

async fn fetch_ocsp_response_inner(
  client: &Client<
    hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
    http_body_util::Full<hyper::body::Bytes>,
  >,
  chain: &[CertificateDer<'_>],
  use_sha256: bool,
) -> anyhow::Result<Option<(Vec<u8>, SystemTime)>> {
  if chain.len() < 2 {
    return Ok(None);
  }

  let leaf = &chain[0];
  let issuer = &chain[1];

  let (_, leaf_cert) =
    X509Certificate::from_der(leaf).map_err(|e| anyhow::anyhow!("Failed to parse leaf cert: {e}"))?;
  let (_, issuer_cert) =
    X509Certificate::from_der(issuer).map_err(|e| anyhow::anyhow!("Failed to parse issuer cert: {e}"))?;

  let Some(ocsp_url) = extract_ocsp_url(&leaf_cert) else {
    return Ok(None);
  };

  let req_der = create_ocsp_request(&leaf_cert, &issuer_cert, use_sha256)?;

  let req = Request::builder()
    .method("POST")
    .uri(&ocsp_url)
    .header("Content-Type", "application/ocsp-request")
    .body(http_body_util::Full::new(hyper::body::Bytes::from(req_der)))
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
  let response: OcspResponse =
    rasn::der::decode(&response_der).map_err(|e| anyhow::anyhow!("Failed to decode OCSP response: {e}"))?;

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

  // Verify signature (try issuer first, then certs[] in the response)
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

  let next_update = min_next_update.unwrap_or_else(|| SystemTime::now() + Duration::from_secs(300));
  Ok(Some((response_der, next_update)))
}

fn extract_ocsp_url(cert: &X509Certificate) -> Option<String> {
  for ext in cert.extensions() {
    if let x509_parser::extensions::ParsedExtension::AuthorityInfoAccess(aia) = ext.parsed_extension() {
      for access_desc in &aia.accessdescs {
        if access_desc.access_method == x509_parser::oid_registry::OID_PKIX_ACCESS_DESCRIPTOR_OCSP {
          if let x509_parser::extensions::GeneralName::URI(uri) = access_desc.access_location {
            return Some(uri.to_string());
          }
        }
      }
    }
  }
  None
}

fn create_ocsp_request(leaf: &X509Certificate, issuer: &X509Certificate, use_sha256: bool) -> anyhow::Result<Vec<u8>> {
  // 1. Hash Issuer DN
  let issuer_name_hash = if use_sha256 {
    let mut sha256 = Sha256::new();
    sha256.update(issuer.subject().as_raw());
    sha256.finalize().to_vec()
  } else {
    let mut sha1 = Sha1::new();
    sha1.update(issuer.subject().as_raw());
    sha1.finalize().to_vec()
  };

  // 2. Hash Issuer Key
  // x509-parser gives SubjectPublicKeyInfo.
  // RFC 6960: hash of the value (excluding tag and length) of the subject public key field.
  let spki = issuer.public_key();
  // spki.subject_public_key is BitString. We want the bytes.
  let pub_key_bytes = &spki.subject_public_key.data;
  let issuer_key_hash = if use_sha256 {
    let mut sha256 = Sha256::new();
    sha256.update(pub_key_bytes);
    sha256.finalize().to_vec()
  } else {
    let mut sha1 = Sha1::new();
    sha1.update(pub_key_bytes);
    sha1.finalize().to_vec()
  };

  // 3. Serial Number
  let serial_number = &leaf.tbs_certificate.serial;
  // Need to convert x509_parser serial (BigUint) to rasn Integer.
  // x509_parser serial is `BigUint`. rasn `Integer` is BigInt.
  let serial_int = rasn::types::Integer::from(num_bigint::BigInt::from_biguint(
    num_bigint::Sign::Plus,
    serial_number.to_owned(),
  ));

  let cert_id = CertId {
    hash_algorithm: rasn_pkix::AlgorithmIdentifier {
      algorithm: if use_sha256 {
        rasn::types::Oid::JOINT_ISO_ITU_T_COUNTRY_US_ORGANIZATION_GOV_CSOR_NIST_ALGORITHMS_HASH_SHA256.to_owned()
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
      version: rasn::types::Integer::from(0), // v1(0)
      requestor_name: None,
      request_list: vec![OcspInnerRequest {
        req_cert: cert_id,
        single_request_extensions: None,
      }],
      request_extensions: None,
    },
    optional_signature: None,
  };

  rasn::der::encode(&req).map_err(|e| anyhow::anyhow!(e))
}

// ---------------------------------------------------------------------------
// OCSP verification helpers (backported from ferron3 ocsp-stapler)
// ---------------------------------------------------------------------------

fn verify_ocsp_signature(basic_response: &BasicOcspResponse, issuer_cert: &X509Certificate) -> anyhow::Result<()> {
  let spki = issuer_cert.public_key();
  let alg: &dyn aws_lc_rs::signature::VerificationAlgorithm =
    match *basic_response.signature_algorithm.algorithm.deref().deref() {
      // RSA + PKCS#1
      [1, 2, 840, 113549, 1, 1, 11] => &aws_lc_rs::signature::RSA_PKCS1_2048_8192_SHA256,
      [1, 2, 840, 113549, 1, 1, 12] => &aws_lc_rs::signature::RSA_PKCS1_2048_8192_SHA384,
      [1, 2, 840, 113549, 1, 1, 13] => &aws_lc_rs::signature::RSA_PKCS1_2048_8192_SHA512,
      [1, 2, 840, 113549, 1, 1, 5] => &aws_lc_rs::signature::RSA_PKCS1_1024_8192_SHA1_FOR_LEGACY_USE_ONLY,

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
          (Some([1, 2, 840, 10045, 3, 1, 7]), 2) => &aws_lc_rs::signature::ECDSA_P256_SHA256_ASN1,
          (Some([1, 2, 840, 10045, 3, 1, 7]), 3) => &aws_lc_rs::signature::ECDSA_P256_SHA384_ASN1,
          (Some([1, 2, 840, 10045, 3, 1, 7]), 4) => &aws_lc_rs::signature::ECDSA_P256_SHA512_ASN1,

          // P-384
          (Some([1, 3, 132, 0, 34]), 2) => &aws_lc_rs::signature::ECDSA_P384_SHA256_ASN1,
          (Some([1, 3, 132, 0, 34]), 3) => &aws_lc_rs::signature::ECDSA_P384_SHA384_ASN1,
          (Some([1, 3, 132, 0, 34]), 4) => &aws_lc_rs::signature::ECDSA_P384_SHA512_ASN1,

          // P-521
          (Some([1, 3, 132, 0, 35]), 2) => &aws_lc_rs::signature::ECDSA_P521_SHA256_ASN1,
          (Some([1, 3, 132, 0, 35]), 3) => &aws_lc_rs::signature::ECDSA_P521_SHA384_ASN1,
          (Some([1, 3, 132, 0, 35]), 4) => &aws_lc_rs::signature::ECDSA_P521_SHA512_ASN1,

          // secp256k1 (not common in OCSP but handle just in case)
          (Some([1, 3, 132, 0, 10]), 2) => &aws_lc_rs::signature::ECDSA_P256K1_SHA256_ASN1,

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

  alg
    .verify_sig(
      spki.subject_public_key.data.as_ref(),
      &rasn::der::encode(&basic_response.tbs_response_data)
        .map_err(|e| anyhow::anyhow!("OCSP response signature verification failed: {e}"))?,
      signature,
    )
    .map_err(|_| anyhow::anyhow!("OCSP response signature verification failed"))?;

  Ok(())
}

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
      let Ok((_, parsed_cert)) = X509Certificate::from_der(&cert_der) else {
        continue;
      };

      // Ensure the candidate cert appears to be issued by the expected issuer (name match)
      if parsed_cert.tbs_certificate.issuer != *issuer_cert.subject() {
        // Certificate not issued by the expected issuer, skip
        continue;
      }

      if !parsed_cert.extensions().iter().any(|e| {
        let parsed = e.parsed_extension();
        match parsed {
          ParsedExtension::ExtendedKeyUsage(eku) => eku.ocsp_signing,
          _ => false,
        }
      }) {
        // The certificate does not have OCSP Extended Key Usage, skip verification
        continue;
      }

      let Err(new_last_error) = verify_ocsp_signature(basic_response, &parsed_cert) else {
        return Ok(());
      };
      last_error = new_last_error;
    }
  }

  Err(last_error)
}

fn hash_oid(data: impl AsRef<[u8]>, oid: ObjectIdentifier) -> anyhow::Result<Vec<u8>> {
  let mut ctx =
    if oid == *rasn::types::Oid::JOINT_ISO_ITU_T_COUNTRY_US_ORGANIZATION_GOV_CSOR_NIST_ALGORITHMS_HASH_SHA256 {
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
      ));
    };
  ctx.update(data.as_ref());
  Ok(ctx.finish().as_ref().to_vec())
}

fn verify_single_res(
  single_res: &rasn_ocsp::SingleResponse,
  leaf_cert: &X509Certificate,
  issuer_cert: &X509Certificate,
) -> anyhow::Result<()> {
  // Check for issuer name hash
  if single_res.cert_id.issuer_name_hash.as_ref()
    != hash_oid(
      issuer_cert.subject().as_raw(),
      single_res.cert_id.hash_algorithm.algorithm.clone(),
    )?
  {
    return Err(anyhow::anyhow!("Issuer name hash mismatch in OCSP response"));
  }

  // Check for issuer key hash
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
  let now_with_skew = SystemTime::now() + Duration::from_secs(60);
  let now = SystemTime::now();
  let this_update_st = SystemTime::from(single_res.this_update);
  if this_update_st > now_with_skew || single_res.next_update.map(SystemTime::from).is_some_and(|nu| nu < now) {
    return Err(anyhow::anyhow!("OCSP response is not current"));
  }

  Ok(())
}
