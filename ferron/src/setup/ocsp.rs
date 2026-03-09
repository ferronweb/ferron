use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use ferron_common::logging::LogMessage;
use hyper::Request;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use rasn::prelude::*;
use rasn_ocsp::{CertId, OcspRequest, OcspResponse, OcspResponseStatus, Request as OcspInnerRequest, TbsRequest};
use rustls::client::WebPkiServerVerifier;
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls_pki_types::CertificateDer;
use rustls_platform_verifier::BuilderVerifierExt;
use sha1::{Digest, Sha1};
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
            // Retry later
            next_updates.insert(key, now + Duration::from_secs(300));
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
  if chain.len() < 2 {
    // Certificate chain too short, don't bother with OCSP
    return Ok(None);
  }
  let leaf = &chain[0];
  let issuer = &chain[1];

  let leaf_cert = X509Certificate::from_der(leaf)?.1;
  let issuer_cert = X509Certificate::from_der(issuer)?.1;

  // Extract OCSP URL
  let Some(ocsp_url) = extract_ocsp_url(&leaf_cert) else {
    // No OCSP URL found
    return Ok(None);
  };

  // Create Request
  let req_der = create_ocsp_request(&leaf_cert, &issuer_cert)?;

  let req = Request::builder()
    .method("POST")
    .uri(&ocsp_url)
    .header("Content-Type", "application/ocsp-request")
    .body(http_body_util::Full::new(hyper::body::Bytes::from(req_der)))?;

  let res = client.request(req).await?;
  if !res.status().is_success() {
    return Err(anyhow::anyhow!(
      "OCSP request failed with status: {} for URL: {ocsp_url}",
      res.status()
    ));
  }

  // Read response
  use http_body_util::BodyExt;
  let body_bytes = res.collect().await?.to_bytes();
  let response_der = body_bytes.to_vec();

  // Parse response to get next update
  let response: OcspResponse =
    rasn::der::decode(&response_der).map_err(|e| anyhow::anyhow!("Failed to decode OCSP response: {}", e))?;

  if response.status != OcspResponseStatus::Successful {
    return Err(anyhow::anyhow!(
      "OCSP response status unsuccessful: {:?}",
      response.status
    ));
  }

  let bytes = response.bytes.ok_or_else(|| anyhow::anyhow!("No response bytes"))?;
  if bytes.r#type
    != ObjectIdentifier::new(vec![1, 3, 6, 1, 5, 5, 7, 48, 1, 1])
      .ok_or_else(|| anyhow::anyhow!("Invalid OCSP basic response OID"))?
  {
    return Err(anyhow::anyhow!("Unsupported OCSP response type"));
  }

  let basic_response: rasn_ocsp::BasicOcspResponse =
    rasn::der::decode(&bytes.response).map_err(|e| anyhow::anyhow!("Failed to decode BasicOcspResponse: {}", e))?;

  // Check validities of all single responses.
  // For simplicity, take the earliest next_update.
  let mut min_next_update = None;

  // Need to adjust for data types. `rasn_ocsp` uses `rasn::types::UtcTime` or `GeneralizedTime`.
  // We need to convert to SystemTime.

  for single_res in basic_response.tbs_response_data.responses {
    let next_update = single_res.next_update.map(SystemTime::from);

    if let Some(mut nu) = next_update {
      // Next update with safety margin.
      let nu_safety_margin = nu
        .duration_since(SystemTime::from(single_res.this_update))
        .map(|d| d / 4)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .max(Duration::from_hours(1)); // Minimum 1h

      // Add randomness to avoid refresh storm.
      let nu_safety_margin = nu_safety_margin + (nu_safety_margin.mul_f64(rand::random_range::<f64, _>(0.0..0.5)));

      if nu - nu_safety_margin > SystemTime::now() {
        nu -= nu_safety_margin;
      }

      match min_next_update {
        Some(min) => {
          if nu < min {
            min_next_update = Some(nu)
          }
        }
        None => min_next_update = Some(nu),
      }
    }
  }

  let next_update = min_next_update.unwrap_or_else(|| SystemTime::now() + Duration::from_hours(12));

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

fn create_ocsp_request(leaf: &X509Certificate, issuer: &X509Certificate) -> anyhow::Result<Vec<u8>> {
  // 1. Hash Issuer DN
  let mut sha1 = Sha1::new();
  sha1.update(issuer.subject().as_raw());
  let issuer_name_hash = sha1.finalize().to_vec();

  // 2. Hash Issuer Key
  // x509-parser gives SubjectPublicKeyInfo.
  // RFC 6960: hash of the value (excluding tag and length) of the subject public key field.
  let spki = issuer.public_key();
  // spki.subject_public_key is BitString. We want the bytes.
  let pub_key_bytes = &spki.subject_public_key.data;
  let mut sha1 = Sha1::new();
  sha1.update(pub_key_bytes);
  let issuer_key_hash = sha1.finalize().to_vec();

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
      algorithm: rasn::types::Oid::ISO_IDENTIFIED_ORGANISATION_OIW_SECSIG_ALGORITHM_SHA1.to_owned(), // sha1
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
