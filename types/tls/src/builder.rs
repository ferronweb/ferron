//! Reusable TLS builder utilities.
//!
//! Functions in this module take [`TlsCryptoConfig`], [`TlsClientAuthConfig`],
//! and related types from the [`config`](crate::config) module and produce
//! ready-to-use `rustls` objects. Any TLS provider can call these instead
//! of duplicating cipher/curve/version/client-auth logic.

use std::sync::Arc;

use crate::config::{
    build_root_cert_store, TlsCipherSuite, TlsClientAuthConfig, TlsCryptoConfig, TlsKxGroup,
    TlsVersion,
};

use rustls::crypto::aws_lc_rs::cipher_suite::*;
use rustls::crypto::aws_lc_rs::default_provider;
use rustls::crypto::aws_lc_rs::kx_group::*;
use rustls::crypto::CryptoProvider;
use rustls::server::danger::ClientCertVerifier;
use rustls::server::WebPkiClientVerifier;
use rustls::version::{TLS12, TLS13};

/// Static protocol version arrays for returning from `resolve_protocol_versions`.
static TLS12_ONLY: &[&rustls::SupportedProtocolVersion] = &[&TLS12];
static TLS13_ONLY: &[&rustls::SupportedProtocolVersion] = &[&TLS13];
static TLS12_AND_13: &[&rustls::SupportedProtocolVersion] = &[&TLS12, &TLS13];

/// Build a `CryptoProvider` with cipher suites and key exchange groups
/// from the given [`TlsCryptoConfig`].
///
/// If `cipher_suites` or `kx_groups` are empty, the rustls defaults are used.
pub fn build_crypto_provider(
    crypto: &TlsCryptoConfig,
) -> Result<CryptoProvider, Box<dyn std::error::Error>> {
    let mut provider = default_provider();

    if !crypto.cipher_suites.is_empty() {
        let cipher_suites = crypto
            .cipher_suites
            .iter()
            .filter_map(cipher_suite_to_rustls)
            .collect::<Vec<_>>();
        if cipher_suites.is_empty() {
            return Err("No valid cipher suites could be resolved from config".into());
        }
        provider.cipher_suites = cipher_suites;
    }

    if !crypto.kx_groups.is_empty() {
        let kx_groups = crypto
            .kx_groups
            .iter()
            .filter_map(|kg| kx_group_to_rustls(kg))
            .collect::<Vec<_>>();
        if kx_groups.is_empty() {
            return Err("No valid key exchange groups could be resolved from config".into());
        }
        provider.kx_groups = kx_groups;
    }

    Ok(provider)
}

/// Map a [`TlsCipherSuite`] to the corresponding `rustls` cipher suite.
fn cipher_suite_to_rustls(cs: &TlsCipherSuite) -> Option<rustls::SupportedCipherSuite> {
    match cs {
        TlsCipherSuite::Tls13Aes128GcmSha256 => Some(TLS13_AES_128_GCM_SHA256),
        TlsCipherSuite::Tls13Aes256GcmSha384 => Some(TLS13_AES_256_GCM_SHA384),
        TlsCipherSuite::Tls13Chacha20Poly1305Sha256 => Some(TLS13_CHACHA20_POLY1305_SHA256),
        TlsCipherSuite::Tls12EcdheEcdsaAes128GcmSha256 => {
            Some(TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256)
        }
        TlsCipherSuite::Tls12EcdheEcdsaAes256GcmSha384 => {
            Some(TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384)
        }
        TlsCipherSuite::Tls12EcdheEcdsaChacha20Poly1305Sha256 => {
            Some(TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256)
        }
        TlsCipherSuite::Tls12EcdheRsaAes128GcmSha256 => Some(TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256),
        TlsCipherSuite::Tls12EcdheRsaAes256GcmSha384 => Some(TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384),
        TlsCipherSuite::Tls12EcdheRsaChacha20Poly1305Sha256 => {
            Some(TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256)
        }
    }
}

/// Map a [`TlsKxGroup`] to the corresponding `rustls` key exchange group.
fn kx_group_to_rustls(kg: &TlsKxGroup) -> Option<&'static dyn rustls::crypto::SupportedKxGroup> {
    match kg {
        TlsKxGroup::Secp256r1 => Some(SECP256R1),
        TlsKxGroup::Secp384r1 => Some(SECP384R1),
        TlsKxGroup::X25519 => Some(X25519),
        TlsKxGroup::X25519Mlkem768 => Some(X25519MLKEM768),
        TlsKxGroup::Mlkem768 => Some(MLKEM768),
    }
}

/// Resolve the TLS protocol version slice from [`TlsCryptoConfig`].
///
/// Returns:
/// - Both `min_version` and `max_version` `None` → default (TLS 1.2 + 1.3)
/// - Specific range → `[min, ..., max]`
///
/// Returns an error if min > max or if a version string is unrecognized.
pub fn resolve_protocol_versions(
    crypto: &TlsCryptoConfig,
) -> Result<&'static [&'static rustls::SupportedProtocolVersion], Box<dyn std::error::Error>> {
    let min_v = crypto.min_version;
    let max_v = crypto.max_version;

    match (min_v, max_v) {
        (None, None) => Ok(TLS12_AND_13),
        (Some(TlsVersion::Tls12), None) => Ok(TLS12_AND_13),
        (Some(TlsVersion::Tls13), None) => Ok(TLS13_ONLY),
        (None, Some(TlsVersion::Tls12)) => Ok(TLS12_ONLY),
        (None, Some(TlsVersion::Tls13)) => Ok(TLS12_AND_13),
        (Some(TlsVersion::Tls12), Some(TlsVersion::Tls12)) => Ok(TLS12_ONLY),
        (Some(TlsVersion::Tls12), Some(TlsVersion::Tls13)) => Ok(TLS12_AND_13),
        (Some(TlsVersion::Tls13), Some(TlsVersion::Tls13)) => Ok(TLS13_ONLY),
        (Some(TlsVersion::Tls13), Some(TlsVersion::Tls12)) => {
            Err("Maximum TLS version is older than minimum TLS version".into())
        }
    }
}

/// Build a `ClientCertVerifier` from [`TlsClientAuthConfig`].
///
/// When `client_auth.enabled` is `false`, returns `WebPkiClientVerifier::no_client_auth()`.
///
/// When enabled:
/// - `required = true` → clients must present a valid cert
/// - `required = false` → clients may present a cert (optional / `.allow_unauthenticated()`)
///
/// The `provider` is forwarded to `WebPkiClientVerifier::builder_with_provider`
/// so the verifier uses the same crypto backend.
pub fn build_client_cert_verifier(
    client_auth: &TlsClientAuthConfig,
    provider: &Arc<CryptoProvider>,
) -> Result<Arc<dyn ClientCertVerifier>, Box<dyn std::error::Error>> {
    if !client_auth.enabled {
        return Ok(WebPkiClientVerifier::no_client_auth());
    }

    let root_store = build_root_cert_store(&client_auth.ca_source)?;

    let mut builder = WebPkiClientVerifier::builder_with_provider(root_store, provider.clone());

    if !client_auth.required {
        builder = builder.allow_unauthenticated();
    }

    Ok(builder.build()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_protocol_versions_defaults() {
        let crypto = TlsCryptoConfig::default();
        let versions = resolve_protocol_versions(&crypto).unwrap();
        assert_eq!(versions.len(), 2);
    }

    #[test]
    fn test_resolve_protocol_versions_tls13_only() {
        let crypto = TlsCryptoConfig {
            min_version: Some(TlsVersion::Tls13),
            max_version: Some(TlsVersion::Tls13),
            ..Default::default()
        };
        let versions = resolve_protocol_versions(&crypto).unwrap();
        assert_eq!(versions.len(), 1);
    }

    #[test]
    fn test_resolve_protocol_versions_min_max() {
        let crypto = TlsCryptoConfig {
            min_version: Some(TlsVersion::Tls12),
            max_version: Some(TlsVersion::Tls13),
            ..Default::default()
        };
        let versions = resolve_protocol_versions(&crypto).unwrap();
        assert_eq!(versions.len(), 2);
    }

    #[test]
    fn test_resolve_protocol_versions_invalid_range() {
        let crypto = TlsCryptoConfig {
            min_version: Some(TlsVersion::Tls13),
            max_version: Some(TlsVersion::Tls12),
            ..Default::default()
        };
        assert!(resolve_protocol_versions(&crypto).is_err());
    }

    #[test]
    fn test_build_crypto_provider_defaults() {
        let crypto = TlsCryptoConfig::default();
        let provider = build_crypto_provider(&crypto).unwrap();
        // defaults should be present
        assert!(!provider.cipher_suites.is_empty());
        assert!(!provider.kx_groups.is_empty());
    }

    #[test]
    fn test_build_crypto_provider_custom() {
        let crypto = TlsCryptoConfig {
            cipher_suites: vec![TlsCipherSuite::Tls13Aes128GcmSha256],
            kx_groups: vec![TlsKxGroup::X25519],
            ..Default::default()
        };
        let provider = build_crypto_provider(&crypto).unwrap();
        assert_eq!(provider.cipher_suites.len(), 1);
        assert_eq!(provider.kx_groups.len(), 1);
    }
}
