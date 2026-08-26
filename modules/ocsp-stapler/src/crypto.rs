//! Cryptographic hashing helpers for OCSP.

use anyhow::{anyhow, Result};
use rasn::types::ObjectIdentifier;

/// Hash the given data using the specified hash algorithm OID.
///
/// This is used for computing the issuer name and key hashes in OCSP requests
/// and responses.
pub(crate) fn hash_oid(data: impl AsRef<[u8]>, oid: ObjectIdentifier) -> Result<Vec<u8>> {
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
            return Err(anyhow!(
                "Unsupported hash algorithm OID in OCSP response: {}",
                oid
            ));
        }
    } else {
        return Err(anyhow!(
            "Unsupported hash algorithm OID in OCSP response: {}",
            oid
        ))
    };
    ctx.update(data.as_ref());
    Ok(ctx.finish().as_ref().to_vec())
}
