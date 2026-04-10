//! Shared ACME challenge types.
//!
//! Defines the shared data locks used across challenge implementations.

pub mod dns01;
pub mod http01;
pub mod tlsalpn01;

use std::sync::Arc;

use instant_acme::ChallengeType;
use rustls::sign::CertifiedKey;
use tokio::sync::RwLock;

/// ACME TLS-ALPN-01 challenge data lock.
/// Holds the self-signed ACME certificate and its associated identifier.
pub type TlsAlpn01DataLock = Arc<RwLock<Option<(Arc<CertifiedKey>, String)>>>;

/// ACME HTTP-01 challenge data lock.
/// Holds the challenge token and key authorization value.
pub type Http01DataLock = Arc<RwLock<Option<(String, String)>>>;

/// Parse challenge type from a string.
pub fn parse_challenge_type(s: &str) -> Option<ChallengeType> {
    match s.to_lowercase().as_str() {
        "http-01" => Some(ChallengeType::Http01),
        "tls-alpn-01" => Some(ChallengeType::TlsAlpn01),
        "dns-01" => Some(ChallengeType::Dns01),
        _ => None,
    }
}

/// ACME TLS-ALPN protocol name used during the handshake.
pub const ACME_TLS_ALPN_NAME: &[u8] = b"acme-tls/1";
