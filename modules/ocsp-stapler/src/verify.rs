//! OCSP response signature and single-response verification.

use std::ops::Deref;
use std::time::{Duration, SystemTime};

use anyhow::Result;
use chrono::{DateTime, Utc};
use rasn::prelude::*;
use rasn_ocsp::{BasicOcspResponse, SingleResponse};
use rasn_pkix::Certificate;

use crate::crypto::hash_oid;
use crate::der::extract_tbs_bytes;

#[derive(AsnType, Clone, Debug, Decode, Encode, PartialEq, Eq, Hash)]
pub(crate) struct RSASSAPSSParams {
    #[rasn(tag(context, 0))]
    pub hash_algorithm: Option<rasn_pkix::AlgorithmIdentifier>,
    #[rasn(tag(context, 1))]
    pub mask_gen_algorithm: Option<rasn_pkix::AlgorithmIdentifier>,
    #[rasn(tag(context, 2))]
    pub salt_length: Option<Integer>,
    #[rasn(tag(context, 3))]
    pub trailer_field: Option<Integer>,
}

/// Verify the signature on the OCSP response using the issuer's public key.
#[inline]
pub(crate) fn verify_ocsp_signature(
    response_bytes: &[u8],
    basic_response: &BasicOcspResponse,
    issuer_cert: &Certificate,
) -> Result<()> {
    // `response` is an OCTET STRING containing the DER-encoded BasicOcspResponse.
    // Keep the raw bytes for signature verification — re-encoding `tbs_response_data`
    // via `rasn::der::encode` is not byte-identical to the original (e.g. an empty
    // `singleExtensions [1] SEQUENCE OF` with 0 elements is omitted on re-encode,
    // 4 bytes shorter) and causes ECDSA verification to fail.
    //
    // So basically, rasn's re-encoding quirks!
    verify_signature(
        &basic_response.signature,
        &basic_response.signature_algorithm,
        &extract_tbs_bytes(response_bytes)
            .map_err(|e| anyhow::anyhow!("OCSP response signature verification failed: {e}"))?,
        issuer_cert,
    )
}

/// Verify a signature using the issuer's public key.
fn verify_signature(
    signature: &rasn::types::BitString,
    signature_algorithm: &rasn_pkix::AlgorithmIdentifier,
    message: &[u8],
    issuer_cert: &Certificate,
) -> Result<()> {
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
                let params: Option<RSASSAPSSParams> = spki
                    .algorithm
                    .parameters
                    .as_ref()
                    .map(|v| v.as_bytes())
                    .and_then(|v| rasn::der::decode::<RSASSAPSSParams>(v).ok());
                let halgorithm = params.and_then(|p| p.hash_algorithm);
                let algorithm_oid = halgorithm.as_ref().map(|a| &a.algorithm);
                let algorithm_oid_u32: Option<&[u32]> = algorithm_oid.map(|oid| oid.as_ref());
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
                let curve_oid: Option<ObjectIdentifier> = spki
                    .algorithm
                    .parameters
                    .as_ref()
                    .map(|v| v.as_bytes())
                    .and_then(|v| rasn::der::decode::<ObjectIdentifier>(v).ok());
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
pub(crate) fn verify_ocsp_signature_with_certs_field(
    response_bytes: &[u8],
    basic_response: &BasicOcspResponse,
    issuer_cert: &Certificate,
) -> Result<()> {
    let Err(mut last_error) = verify_ocsp_signature(response_bytes, basic_response, issuer_cert)
    else {
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

            let Err(new_last_error) = verify_ocsp_signature(response_bytes, basic_response, cert)
            else {
                return Ok(());
            };
            last_error = new_last_error;
        }
    }

    Err(last_error)
}

/// Verify that the SingleResponse matches the leaf and issuer certs.
///
/// This includes checking the issuer name and key hashes, and the serial number. This
/// is important to prevent replay attacks where an attacker could use a valid OCSP response
/// for a different certificate.
pub(crate) fn verify_single_res(
    single_res: &SingleResponse,
    leaf_cert: &Certificate,
    issuer_cert: &Certificate,
) -> Result<()> {
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
    let now_with_skew = DateTime::<Utc>::from(SystemTime::now() + Duration::from_secs(60));
    let now = DateTime::<Utc>::from(SystemTime::now());
    if single_res.this_update > now_with_skew
        || single_res.next_update.as_ref().is_some_and(|nu| *nu < now)
    {
        return Err(anyhow::anyhow!("OCSP response is not current"));
    }

    Ok(())
}
