//! OCSP stapling support for Ferron TLS servers.
//!
//! This crate provides:
//! - `OcspStapler`: a `ResolvesServerCert` wrapper that attaches OCSP responses
//! - `OcspServiceHandle`: shared handle to the background OCSP fetching service
//! - `init_ocsp_service()`: spawns the background service on the secondary runtime
//! - `get_service_handle()`: global accessor for TLS providers
//!
//! # Architecture
//!
//! A single background task runs on the secondary tokio runtime, fetching OCSP
//! responses over HTTPS and caching them. TLS providers wrap their certificate
//! resolver with `OcspStapler`, which intercepts `resolve()` calls and attaches
//! stapled responses from the cache.
//!
//! # Usage
//!
//! 1. During startup, call `init_ocsp_service(&runtime)` from your `ModuleLoader`
//! 2. In your TLS provider, call `get_service_handle()` and wrap your resolver:
//!    ```ignore
//!    if let Some(handle) = ferron_ocsp::get_service_handle() {
//!        config.cert_resolver = Arc::new(OcspStapler::new(inner_resolver, &handle));
//!    }
//!    ```

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::Context as _;
use ferron_core::log_warn;
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
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls_pki_types::CertificateDer;
use sha1::{Digest, Sha1};
use sha2::Sha256;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use x509_parser::prelude::*;

// Type alias for the OCSP cache to reduce type complexity
type OcspCache = Arc<RwLock<HashMap<Vec<u8>, Option<Arc<CertifiedKey>>>>>;

/// Error returned when `init_ocsp_service` is called more than once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlreadyInitialized;

impl std::fmt::Display for AlreadyInitialized {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "OCSP service already initialized")
    }
}

impl std::error::Error for AlreadyInitialized {}

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

/// Eagerly-created global state. The channel is created on first access so
/// certs can be queued via the sender even before `init_ocsp_service` spawns
/// the background task.
struct GlobalState {
    sender: mpsc::UnboundedSender<CertifiedKey>,
    receiver: std::sync::Mutex<Option<mpsc::UnboundedReceiver<CertifiedKey>>>,
    cache: OcspCache,
    cancel_token: CancellationToken,
}

static GLOBAL_STATE: std::sync::OnceLock<GlobalState> = std::sync::OnceLock::new();

fn get_or_init_global() -> &'static GlobalState {
    GLOBAL_STATE.get_or_init(|| {
        let (sender, receiver) = mpsc::unbounded_channel();
        GlobalState {
            sender,
            receiver: std::sync::Mutex::new(Some(receiver)),
            cache: Arc::new(RwLock::new(HashMap::new())),
            cancel_token: CancellationToken::new(),
        }
    })
}

/// Initialize the OCSP service. Call once during startup.
///
/// Spawns the background task on the secondary tokio runtime via
/// `runtime.spawn_secondary_task()`. Returns `Err(AlreadyInitialized)`
/// if called more than once.
///
/// The sender channel is created eagerly on first access, so certs can be
/// queued before this function is called. When the background task starts,
/// it will process any queued certs.
pub fn init_ocsp_service(
    runtime: &ferron_core::runtime::Runtime,
) -> Result<(), AlreadyInitialized> {
    let state = get_or_init_global();

    // Take the receiver — if already taken, service was already started
    let receiver = state
        .receiver
        .lock()
        .unwrap()
        .take()
        .ok_or(AlreadyInitialized)?;

    runtime.spawn_secondary_task(background_ocsp_task(
        receiver,
        state.cache.clone(),
        state.cancel_token.clone(),
    ));

    Ok(())
}

/// Get the global `OcspServiceHandle`.
///
/// Always returns `Some` — the channel and cache are created on first access.
/// Certs can be queued via the returned handle even before `init_ocsp_service`
/// spawns the background task.
pub fn get_service_handle() -> Option<OcspServiceHandle> {
    let state = get_or_init_global();
    Some(OcspServiceHandle {
        sender: state.sender.clone(),
        cache: state.cache.clone(),
        cancel_token: state.cancel_token.clone(),
    })
}

// ---------------------------------------------------------------------------
// Shared handle
// ---------------------------------------------------------------------------

/// Cheap to clone (`Arc`-backed channels and locks).
#[derive(Clone)]
pub struct OcspServiceHandle {
    sender: mpsc::UnboundedSender<CertifiedKey>,
    cache: OcspCache,
    #[allow(dead_code)]
    cancel_token: CancellationToken,
}

impl OcspServiceHandle {
    /// Send a `CertifiedKey` to the background task for OCSP fetching.
    pub fn preload(&self, key: CertifiedKey) {
        if !key.cert.is_empty() {
            let _ = self.sender.send(key);
        }
    }
}

// ---------------------------------------------------------------------------
// OcspStapler — ResolvesServerCert wrapper
// ---------------------------------------------------------------------------

/// Wraps an inner `ResolvesServerCert` and attaches OCSP responses from the
/// shared cache.
///
/// On the first `resolve()` call for a given certificate, the original key is
/// returned and a fetch is triggered in the background. Subsequent calls
/// return the key with the stapled OCSP response attached.
#[derive(Debug)]
pub struct OcspStapler {
    inner: Arc<dyn ResolvesServerCert>,
    cache: OcspCache,
    sender: mpsc::UnboundedSender<CertifiedKey>,
}

impl OcspStapler {
    /// Create a new `OcspStapler` wrapping `inner`.
    pub fn new(inner: Arc<dyn ResolvesServerCert>, handle: &OcspServiceHandle) -> Self {
        Self {
            inner,
            cache: handle.cache.clone(),
            sender: handle.sender.clone(),
        }
    }
}

impl ResolvesServerCert for OcspStapler {
    fn resolve(&self, client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        let original_key = self.inner.resolve(client_hello)?;
        if let Some(leaf) = original_key.cert.first() {
            let leaf_bytes: Vec<u8> = leaf.to_vec();

            // Read cache — uses parking_lot::RwLock which is safe to call from
            // any thread (including vibeio primary threads).
            let cached = self.cache.read();

            if let Some(cached_entry) = cached.get(&leaf_bytes) {
                if let Some(stapled) = cached_entry {
                    return Some(stapled.clone());
                }
                // Entry exists but has no OCSP yet — return original without re-triggering
            } else {
                // Not in cache yet — trigger fetch
                drop(cached);
                let _ = self.sender.send((*original_key).clone());
            }
        }
        Some(original_key)
    }
}

// ---------------------------------------------------------------------------
// HTTPS client construction
// ---------------------------------------------------------------------------

/// Build an `HttpsConnector` with native certificate store and webpki-roots fallback.
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

// ---------------------------------------------------------------------------
// Background task
// ---------------------------------------------------------------------------

async fn background_ocsp_task(
    mut receiver: mpsc::UnboundedReceiver<CertifiedKey>,
    cache: OcspCache,
    cancel_token: CancellationToken,
) {
    // Track next-update times per cert
    let mut next_updates: HashMap<Vec<u8>, SystemTime> = HashMap::new();
    // Track known cert chains
    let mut known_certs: HashMap<Vec<u8>, CertifiedKey> = HashMap::new();

    // Build HTTPS client with native certificate store and webpki-roots fallback
    let https_connector =
        build_https_connector().expect("failed to create HTTPS connector with native/webpki roots");

    let client = Client::builder(TokioExecutor::new())
        .build::<_, http_body_util::Full<Bytes>>(https_connector);

    let sleep_duration = Duration::from_secs(60); // default check interval

    loop {
        let received_certified_key = tokio::select! {
            _ = cancel_token.cancelled() => return,
            _ = tokio::time::sleep(sleep_duration) => None,
            res = receiver.recv() => match res {
                Some(chain) => Some(chain),
                None => return, // channel closed
            },
        };

        // Process newly received cert
        if let Some(certified_key) = received_certified_key {
            let chain = &certified_key.cert;
            if let Some(leaf) = chain.first() {
                let key: Vec<u8> = leaf.to_vec();
                if !known_certs.contains_key(&key) {
                    known_certs.insert(key.clone(), certified_key);
                    // Trigger immediate fetch
                    next_updates.insert(key, SystemTime::now());
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
            if let Some(certified_key) = known_certs.get(&key) {
                match fetch_ocsp_response(&client, &certified_key.cert).await {
                    Ok(Some((response_der, next_update_time))) => {
                        let mut new_certified_key = certified_key.clone();
                        new_certified_key.ocsp = Some(response_der);
                        cache
                            .write()
                            .insert(key.clone(), Some(Arc::new(new_certified_key)));
                        next_updates.insert(key, next_update_time);
                    }
                    Ok(None) => {
                        // No OCSP possible (e.g. no OCSP URL in cert)
                        cache.write().insert(key.clone(), None);
                        next_updates.remove(&key);
                    }
                    Err(e) => {
                        log_warn!("OCSP fetch failed: {e}");
                        // Retry later with randomness to avoid refresh storms
                        let jitter = rand::random_range(100..=500);
                        next_updates.insert(key, now + Duration::from_secs(jitter));
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// OCSP fetch & construction
// ---------------------------------------------------------------------------

async fn fetch_ocsp_response(
    client: &Client<
        hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
        http_body_util::Full<Bytes>,
    >,
    chain: &[CertificateDer<'_>],
) -> anyhow::Result<Option<(Vec<u8>, SystemTime)>> {
    // Try SHA-256 first, fall back to SHA-1
    let response = fetch_ocsp_response_inner(client, chain, true).await;
    if response.is_ok() {
        return response;
    }
    if let Ok(sha1_response) = fetch_ocsp_response_inner(client, chain, false).await {
        return Ok(sha1_response);
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
            "OCSP response status unsuccessful: {:?}",
            response.status
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

    // Compute next_update across all single responses
    let mut min_next_update: Option<SystemTime> = None;
    for single_res in basic_response.tbs_response_data.responses {
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
                _ => min_next_update.unwrap(),
            });
        }
    }

    let next_update =
        min_next_update.unwrap_or_else(|| SystemTime::now() + Duration::from_hours(12));
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
    let issuer_name_hash = if use_sha256 {
        Sha256::digest(issuer.subject().as_raw()).to_vec()
    } else {
        Sha1::digest(issuer.subject().as_raw()).to_vec()
    };

    // Hash issuer public key value (excluding tag/length per RFC 6960)
    let pub_key_bytes = &issuer.public_key().subject_public_key.data;
    let issuer_key_hash = if use_sha256 {
        Sha256::digest(pub_key_bytes).to_vec()
    } else {
        Sha1::digest(pub_key_bytes).to_vec()
    };

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ocsp_service_handle_is_cloneable() {
        fn assert_clone<T: Clone>() {}
        assert_clone::<OcspServiceHandle>();
    }

    #[test]
    fn test_ocsp_stapler_impls_resolves_server_cert() {
        fn assert_resolves<T: ResolvesServerCert>() {}
        assert_resolves::<OcspStapler>();
    }
}
