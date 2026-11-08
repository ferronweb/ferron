use std::error::Error;

use rcgen::{
    CertificateParams, CertificateSigningRequest, DistinguishedName, KeyPair, RsaKeySize,
    SignatureAlgorithm,
};

#[inline]
pub async fn generate_csr(
    mut identifiers: instant_acme::Identifiers<'_>,
    algo: &'static SignatureAlgorithm,
    key_size: Option<RsaKeySize>,
) -> Result<(KeyPair, CertificateSigningRequest), Box<dyn Error + Send + Sync>> {
    // Similar to what's done in `instant_acme::Order::finalize function contents`

    let mut names = Vec::new();
    while let Some(result) = identifiers.next().await {
        names.push(
            result
                .map_err(|e| {
                    anyhow::anyhow!("Cannot obtain ACME identifiers for CSR generation: {e}")
                })?
                .to_string(),
        );
    }

    let mut params = CertificateParams::new(names)?;
    params.distinguished_name = DistinguishedName::new();
    let key = if let Some(key_size) = key_size {
        KeyPair::generate_rsa_for(algo, key_size)?
    } else {
        KeyPair::generate_for(algo)?
    };
    let csr = params.serialize_request(&key)?;

    Ok((key, csr))
}
