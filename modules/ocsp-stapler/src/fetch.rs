//! HTTP fetching, OCSP request building, and response decoding.

use std::time::{Duration, SystemTime};

use anyhow::{anyhow, Context as _};
use aws_lc_rs::digest::Context;
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::Request;
use hyper_util::client::legacy::Client;
use rand::random_range;
use rasn::prelude::*;
use rasn::types::{ObjectIdentifier, OctetString, Oid};
use rasn_ocsp::{
    BasicOcspResponse, CertId, OcspRequest, OcspResponse, OcspResponseStatus,
    Request as RasnOcspRequest, TbsRequest,
};
use rasn_pkix::AlgorithmIdentifier;
use rustls_pki_types::CertificateDer;

/// Convenience alias for the configured HTTPS client used for OCSP requests.
pub(crate) type OcspHttpClient = Client<
    hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
    Full<Bytes>,
>;

/// Build an `HttpsConnector` with native certificate store and webpki-roots fallback.
pub(crate) fn build_https_connector() -> Result<
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

/// Build a fresh digest context for the requested hash algorithm, honoring the
/// `fips` feature (which disallows SHA-1).
fn hash_context(use_sha256: bool) -> Context {
    #[cfg(not(feature = "fips"))]
    {
        if use_sha256 {
            Context::new(&aws_lc_rs::digest::SHA256)
        } else {
            Context::new(&aws_lc_rs::digest::SHA1_FOR_LEGACY_USE_ONLY)
        }
    }
    #[cfg(feature = "fips")]
    {
        let _ = use_sha256;
        Context::new(&aws_lc_rs::digest::SHA256)
    }
}

/// Select the hash algorithm OID for the OCSP `CertId`, honoring the `fips`
/// feature (which disallows SHA-1).
fn ocsp_hash_oid(use_sha256: bool) -> ObjectIdentifier {
    #[cfg(not(feature = "fips"))]
    {
        if use_sha256 {
            Oid::JOINT_ISO_ITU_T_COUNTRY_US_ORGANIZATION_GOV_CSOR_NIST_ALGORITHMS_HASH_SHA256
                .to_owned()
        } else {
            Oid::ISO_IDENTIFIED_ORGANISATION_OIW_SECSIG_ALGORITHM_SHA1.to_owned()
        }
    }
    #[cfg(feature = "fips")]
    {
        let _ = use_sha256;
        Oid::JOINT_ISO_ITU_T_COUNTRY_US_ORGANIZATION_GOV_CSOR_NIST_ALGORITHMS_HASH_SHA256.to_owned()
    }
}

pub(crate) async fn fetch_ocsp_response(
    client: &OcspHttpClient,
    chain: &[CertificateDer<'_>],
) -> anyhow::Result<Option<(Vec<u8>, SystemTime, Option<rasn_ocsp::CertStatus>)>> {
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
    client: &OcspHttpClient,
    chain: &[CertificateDer<'_>],
    use_sha256: bool,
) -> anyhow::Result<Option<(Vec<u8>, SystemTime, Option<rasn_ocsp::CertStatus>)>> {
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
        .body(Full::new(Bytes::from(req_der)))
        .with_context(|| format!("Failed to build OCSP request for {ocsp_url}"))?;

    let res = client.request(req).await?;
    if !res.status().is_success() {
        return Err(anyhow::anyhow!(
            "OCSP request failed with status {} for URL: {ocsp_url}",
            res.status()
        ));
    }

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

    crate::verify::verify_ocsp_signature_with_certs_field(
        &response_bytes.response,
        &basic_response,
        &issuer_cert,
    )?;

    // Compute next_update across all single responses.
    let mut min_next_update: Option<SystemTime> = None;
    let mut cert_status = None;
    for single_res in basic_response.tbs_response_data.responses {
        crate::verify::verify_single_res(&single_res, &leaf_cert, &issuer_cert)?;

        let Some(mut nu) = single_res.next_update.map(SystemTime::from) else {
            continue;
        };
        let this_update = SystemTime::from(single_res.this_update);
        let validity = nu.duration_since(this_update).unwrap_or_default();
        let margin = validity / 4 + Duration::from_secs(random_range(0..=300));
        // Refresh a bit before expiry to avoid serving a stale response.
        nu = nu.checked_sub(margin).unwrap_or(nu);
        min_next_update = Some(match min_next_update {
            Some(min) => nu.min(min),
            None => nu,
        });
        cert_status = Some(single_res.cert_status.clone());
    }

    let next_update =
        min_next_update.unwrap_or_else(|| SystemTime::now() + Duration::from_secs(300));
    Ok(Some((response_der, next_update, cert_status)))
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
    let mut issuer_name_ctx = hash_context(use_sha256);
    issuer_name_ctx.update(&rasn::der::encode(&issuer.tbs_certificate.subject)?);
    let issuer_name_hash = issuer_name_ctx.finish().as_ref().to_vec();

    // Hash issuer public key value (excluding tag/length per RFC 6960)
    let pub_key_bytes = &issuer
        .tbs_certificate
        .subject_public_key_info
        .subject_public_key
        .as_raw_slice();
    let mut issuer_key_ctx = hash_context(use_sha256);
    issuer_key_ctx.update(pub_key_bytes);
    let issuer_key_hash = issuer_key_ctx.finish().as_ref().to_vec();

    // Serial number
    let serial_int = leaf.tbs_certificate.serial_number.clone();

    let cert_id = CertId {
        hash_algorithm: AlgorithmIdentifier {
            algorithm: ocsp_hash_oid(use_sha256),
            parameters: None,
        },
        issuer_name_hash: OctetString::from(issuer_name_hash),
        issuer_key_hash: OctetString::from(issuer_key_hash),
        serial_number: serial_int,
    };

    let req = OcspRequest {
        tbs_request: TbsRequest {
            version: Integer::from(0), // v1
            requestor_name: None,
            request_list: vec![RasnOcspRequest {
                req_cert: cert_id,
                single_request_extensions: None,
            }],
            request_extensions: None,
        },
        optional_signature: None,
    };

    rasn::der::encode(&req).map_err(|e| anyhow!(e))
}
