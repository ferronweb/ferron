//! TLS-ALPN-01 ACME challenge implementation.
//!
//! The TLS-ALPN-01 challenge requires responding with a self-signed certificate
//! when the client connects with the `acme-tls/1` ALPN protocol and matching SNI.

use std::sync::Arc;

use instant_acme::KeyAuthorization;
use rcgen::{CertificateParams, CustomExtension, KeyPair};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls_pki_types::PrivateKeyDer;

use super::{TlsAlpn01DataLock, ACME_TLS_ALPN_NAME};

/// Challenge certificate with PEM material for distributed sharing:
/// (certified key, identifier, certificate-chain PEM, private-key PEM).
pub type ChallengeCertWithPem = (Arc<CertifiedKey>, String, String, String);

/// Resolver for TLS-ALPN-01 challenges.
///
/// Implements `ResolvesServerCert` to check for pending ACME challenges
/// during the TLS handshake. If the client hello contains the `acme-tls/1`
/// ALPN and the SNI matches a pending challenge, returns the self-signed
/// ACME certificate.
#[derive(Debug)]
pub struct TlsAlpn01Resolver {
    resolvers: Arc<tokio::sync::RwLock<Vec<TlsAlpn01DataLock>>>,
}

impl TlsAlpn01Resolver {
    /// Creates a new empty `TlsAlpn01Resolver`.
    pub fn new(resolvers: Arc<tokio::sync::RwLock<Vec<TlsAlpn01DataLock>>>) -> Self {
        Self { resolvers }
    }

    /// Generates a self-signed certificate for a TLS-ALPN-01 challenge.
    ///
    /// The certificate contains the ACME identifier extension with the
    /// SHA-256 digest of the key authorization.
    ///
    /// Returns the certified key, the identifier, and the PEM-encoded
    /// certificate chain + private key so the challenge can be shared with
    /// peer nodes via the shared `cache` directory (see `challenge_sync`).
    pub fn generate_challenge_cert(
        identifier: &str,
        key_authorization: &KeyAuthorization,
    ) -> Result<(Arc<CertifiedKey>, String), Box<dyn std::error::Error + Send + Sync>> {
        let (certified_key, ident, _cert_pem, _key_pem) =
            Self::generate_challenge_cert_with_pem(identifier, key_authorization)?;
        Ok((certified_key, ident))
    }

    /// Same as [`TlsAlpn01Resolver::generate_challenge_cert`], additionally
    /// returning PEM material for distributed sharing.
    pub fn generate_challenge_cert_with_pem(
        identifier: &str,
        key_authorization: &KeyAuthorization,
    ) -> Result<ChallengeCertWithPem, Box<dyn std::error::Error + Send + Sync>> {
        let mut params = CertificateParams::new(vec![identifier.to_string()])?;
        params
            .custom_extensions
            .push(CustomExtension::new_acme_identifier(
                key_authorization.digest().as_ref(),
            ));
        let key_pair = KeyPair::generate()?;
        let certificate = params.self_signed(&key_pair)?;
        let certificate_chain_pem = certificate.pem();
        let private_key_pem = key_pair.serialize_pem();
        let private_key = PrivateKeyDer::try_from(key_pair.serialize_der())?;

        let signing_key = rustls::crypto::aws_lc_rs::default_provider()
            .key_provider
            .load_private_key(private_key)?;

        let certified_key = Arc::new(CertifiedKey::new(
            vec![certificate.der().to_owned()],
            signing_key,
        ));

        Ok((
            certified_key,
            identifier.to_string(),
            certificate_chain_pem,
            private_key_pem,
        ))
    }
}

impl ResolvesServerCert for TlsAlpn01Resolver {
    fn resolve(&self, client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        // Check if the client requested the ACME TLS-ALPN protocol
        let mut alpn = client_hello.alpn()?;

        let has_acme_alpn = alpn.any(|name| name == ACME_TLS_ALPN_NAME);
        if !has_acme_alpn {
            return None;
        }

        let server_name = client_hello.server_name()?;

        // Search through all resolvers for a matching challenge
        let resolvers = self.resolvers.try_read().ok()?;
        for lock in resolvers.iter() {
            if let Some(data) = lock.try_read().ok().and_then(|g| g.clone()) {
                if data.1 == server_name {
                    return Some(data.0);
                }
            }
        }

        None
    }
}
