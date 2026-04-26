//! ACME configuration types and parsing from `ServerConfigurationBlock`.
//!
//! Two modes are supported:
//! - **Eager** (`AcmeConfig`): Certificates are obtained at startup for known domains.
//! - **On-demand** (`AcmeOnDemandConfig`): Certificates are obtained lazily on first TLS handshake.

use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine;
use ferron_core::config::ServerConfigurationBlock;
use ferron_dns::DnsClient;
use instant_acme::{ChallengeType, ExternalAccountKey};
use rustls::ClientConfig;
use tokio::sync::RwLock;

use crate::cache::AcmeCache;
use crate::challenge::{Http01DataLock, TlsAlpn01DataLock};

/// Shared type for SNI resolver locks used in on-demand mode.
pub type SniResolverLock =
    Arc<RwLock<std::collections::HashMap<String, Arc<dyn rustls::server::ResolvesServerCert>>>>;

/// Eager ACME configuration for obtaining certificates at startup.
pub struct AcmeConfig {
    /// The Rustls client configuration for ACME communication.
    pub rustls_client_config: ClientConfig,
    /// The domains for which to request certificates.
    pub domains: Vec<String>,
    /// The type of challenge to use.
    pub challenge_type: ChallengeType,
    /// Contact information for the ACME account.
    pub contact: Vec<String>,
    /// ACME directory URL.
    pub directory: String,
    /// Optional EAB key.
    pub eab_key: Option<Arc<ExternalAccountKey>>,
    /// Optional ACME profile name.
    pub profile: Option<String>,
    /// Cache for ACME account data.
    pub account_cache: AcmeCache,
    /// Cache for ACME certificate data.
    pub certificate_cache: AcmeCache,
    /// Lock for the certified key (updated after certificate issuance).
    pub certified_key_lock: Arc<RwLock<Option<Arc<rustls::sign::CertifiedKey>>>>,
    /// Lock for TLS-ALPN-01 challenge data.
    pub tls_alpn_01_data_lock: TlsAlpn01DataLock,
    /// Lock for HTTP-01 challenge data.
    pub http_01_data_lock: Http01DataLock,
    /// DNS provider for DNS-01 challenges (if configured).
    pub dns_client: Option<Arc<dyn DnsClient>>,
    /// The ACME account (once loaded/created).
    pub account: Option<instant_acme::Account>,
    /// Paths to save the certificate and private key files.
    pub save_paths: Option<(PathBuf, PathBuf)>,
    /// Command to run after certificate issuance.
    pub post_obtain_command: Option<String>,
}

/// On-demand ACME configuration for lazy certificate issuance.
pub struct AcmeOnDemandConfig {
    /// The Rustls client configuration for ACME communication.
    pub rustls_client_config: ClientConfig,
    /// The type of challenge to use.
    pub challenge_type: ChallengeType,
    /// Contact information for the ACME account.
    pub contact: Vec<String>,
    /// ACME directory URL.
    pub directory: String,
    /// Optional EAB key.
    pub eab_key: Option<Arc<ExternalAccountKey>>,
    /// Optional ACME profile name.
    pub profile: Option<String>,
    /// Path to the cache directory for on-domain caching.
    pub cache_path: Option<PathBuf>,
    /// Lock for SNI resolvers (shared across on-demand configs).
    pub sni_resolver_lock: SniResolverLock,
    /// Lock for TLS-ALPN-01 resolvers (shared).
    pub tls_alpn_01_resolver_lock: Arc<RwLock<Vec<TlsAlpn01DataLock>>>,
    /// Lock for HTTP-01 resolvers (shared).
    pub http_01_resolver_lock: Arc<RwLock<Vec<Http01DataLock>>>,
    /// DNS provider for DNS-01 challenges.
    pub dns_client: Option<Arc<dyn DnsClient>>,
    /// The SNI hostname pattern to match.
    pub sni_hostname: Option<String>,
    /// The port this config applies to.
    pub port: u16,
    /// Optional endpoint to ask before issuing a certificate.
    pub on_demand_ask: Option<String>,
    /// Whether to skip TLS verification for the on-demand ask endpoint.
    pub on_demand_ask_no_verification: bool,
}

/// A cloneable subset of on-demand config data used by the background task.
///
/// This struct contains only the data fields needed for on-demand config
/// conversion, excluding shared Arc locks and trait objects.
#[derive(Clone)]
pub struct AcmeOnDemandConfigData {
    /// The Rustls client configuration for ACME communication.
    pub rustls_client_config: ClientConfig,
    /// The type of challenge to use.
    pub challenge_type: ChallengeType,
    /// Contact information for the ACME account.
    pub contact: Vec<String>,
    /// ACME directory URL.
    pub directory: String,
    /// Optional EAB key.
    pub eab_key: Option<Arc<ExternalAccountKey>>,
    /// Optional ACME profile name.
    pub profile: Option<String>,
    /// Path to the cache directory for on-domain caching.
    pub cache_path: Option<PathBuf>,
    /// DNS provider for DNS-01 challenges.
    pub dns_client: Option<Arc<dyn DnsClient>>,
    /// The SNI hostname pattern to match.
    pub sni_hostname: Option<String>,
    /// The port this config applies to.
    pub port: u16,
    /// Optional endpoint to ask before issuing a certificate.
    pub on_demand_ask: Option<String>,
    /// Whether to skip TLS verification for the on-demand ask endpoint.
    pub on_demand_ask_no_verification: bool,
}

impl AcmeOnDemandConfig {
    /// Extracts the cloneable data portion for use by the background task.
    pub fn clone_for_state(&self) -> AcmeOnDemandConfigData {
        AcmeOnDemandConfigData {
            rustls_client_config: self.rustls_client_config.clone(),
            challenge_type: self.challenge_type.clone(),
            contact: self.contact.clone(),
            directory: self.directory.clone(),
            eab_key: self.eab_key.clone(),
            profile: self.profile.clone(),
            cache_path: self.cache_path.clone(),
            dns_client: self.dns_client.clone(),
            sni_hostname: self.sni_hostname.clone(),
            port: self.port,
            on_demand_ask: self.on_demand_ask.clone(),
            on_demand_ask_no_verification: self.on_demand_ask_no_verification,
        }
    }

    /// Converts from a portable data config back to a full config with shared state.
    pub fn from_data_with_state(
        data: AcmeOnDemandConfigData,
        sni_resolver_lock: SniResolverLock,
        tls_alpn_01_resolver_lock: Arc<RwLock<Vec<TlsAlpn01DataLock>>>,
        http_01_resolver_lock: Arc<RwLock<Vec<Http01DataLock>>>,
    ) -> Self {
        AcmeOnDemandConfig {
            rustls_client_config: data.rustls_client_config,
            challenge_type: data.challenge_type,
            contact: data.contact,
            directory: data.directory,
            eab_key: data.eab_key,
            profile: data.profile,
            cache_path: data.cache_path,
            sni_resolver_lock,
            tls_alpn_01_resolver_lock,
            http_01_resolver_lock,
            dns_client: data.dns_client,
            sni_hostname: data.sni_hostname,
            port: data.port,
            on_demand_ask: data.on_demand_ask,
            on_demand_ask_no_verification: data.on_demand_ask_no_verification,
        }
    }
}

/// Helper to collect multi-values for a directive.
#[allow(dead_code)]
fn collect_multi_values(config: &ServerConfigurationBlock, name: &str) -> Vec<String> {
    config
        .directives
        .get(name)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|e| e.args.first())
                .filter_map(|v| v.as_string_with_interpolations(&std::collections::HashMap::new()))
                .collect()
        })
        .unwrap_or_default()
}

/// Get the first string value for a directive.
fn first_value(config: &ServerConfigurationBlock, name: &str) -> Option<String> {
    config
        .get_value(name)
        .and_then(|v| v.as_string_with_interpolations(&std::collections::HashMap::new()))
}

/// Resolves the ACME directory URL from config.
///
/// Defaults to Let's Encrypt Production.
pub fn resolve_directory(config: &ServerConfigurationBlock) -> String {
    first_value(config, "directory").unwrap_or_else(|| {
        // Let's Encrypt Production
        "https://acme-v02.api.letsencrypt.org/directory".to_string()
    })
}

/// Parses the EAB key from configuration.
///
/// Expects: `eab "key-id" "hmac-base64"`
pub fn parse_eab(config: &ServerConfigurationBlock) -> Option<Arc<ExternalAccountKey>> {
    let entries = config.directives.get("eab")?;
    let entry = entries.first()?;
    if entry.args.len() < 2 {
        return None;
    }

    let key_id = entry.args[0].as_string_with_interpolations(&std::collections::HashMap::new())?;
    let hmac_str = entry.args[1].as_string_with_interpolations(&std::collections::HashMap::new())?;

    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(hmac_str.trim_end_matches('='))
        .ok()?;

    Some(Arc::new(ExternalAccountKey::new(key_id, &decoded)))
}

/// Resolves the ACME cache path from config.
///
/// If not specified, defaults to the platform data directory.
pub fn resolve_cache_path(config: &ServerConfigurationBlock) -> Option<PathBuf> {
    if let Some(path_str) = first_value(config, "cache") {
        return Some(PathBuf::from(&path_str));
    }

    // Default to platform data directory
    let fallback_path = dirs::data_local_dir().map(|mut p| {
        p.push("ferron-acme");
        p
    });

    // /var/cache/ferron-acme heuristic for the Docker image
    #[cfg(unix)]
    let fallback_path = fallback_path.or_else(|| {
        if matches!(std::fs::exists("/var/cache/ferron-acme"), Ok(true)) {
            Some(PathBuf::from("/var/cache/ferron-acme"))
        } else {
            None
        }
    });

    fallback_path
}

/// Builds a Rustls client configuration for ACME.
///
/// If `no_verification` is true, all certificate validation is skipped
/// (for testing or internal ACME directories).
pub fn build_rustls_client_config(
    no_verification: bool,
) -> Result<ClientConfig, Box<dyn std::error::Error + Send + Sync>> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());

    if no_verification {
        #[derive(Debug)]
        struct NoVerifier;

        impl rustls::client::danger::ServerCertVerifier for NoVerifier {
            fn verify_server_cert(
                &self,
                _end_entity: &rustls_pki_types::CertificateDer<'_>,
                _intermediates: &[rustls_pki_types::CertificateDer<'_>],
                _server_name: &rustls_pki_types::ServerName<'_>,
                _ocsp_response: &[u8],
                _now: rustls_pki_types::UnixTime,
            ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
                Ok(rustls::client::danger::ServerCertVerified::assertion())
            }

            fn verify_tls12_signature(
                &self,
                _message: &[u8],
                _cert: &rustls_pki_types::CertificateDer<'_>,
                _dss: &rustls::DigitallySignedStruct,
            ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error>
            {
                Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
            }

            fn verify_tls13_signature(
                &self,
                _message: &[u8],
                _cert: &rustls_pki_types::CertificateDer<'_>,
                _dss: &rustls::DigitallySignedStruct,
            ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error>
            {
                Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
            }

            fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
                use rustls::SignatureScheme::*;
                vec![
                    ECDSA_NISTP384_SHA384,
                    ECDSA_NISTP256_SHA256,
                    ED25519,
                    RSA_PSS_SHA512,
                    RSA_PSS_SHA384,
                    RSA_PSS_SHA256,
                    RSA_PKCS1_SHA512,
                    RSA_PKCS1_SHA384,
                    RSA_PKCS1_SHA256,
                ]
            }
        }

        Ok(ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoVerifier))
            .with_no_client_auth())
    } else {
        let root_store = build_root_cert_store()?;

        Ok(ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()?
            .with_root_certificates(root_store)
            .with_no_client_auth())
    }
}

/// Build a `RootCertStore` with native system certificates, falling back to
/// embedded `webpki-roots` if native certs cannot be loaded.
fn build_root_cert_store() -> Result<rustls::RootCertStore, Box<dyn std::error::Error + Send + Sync>>
{
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
        return Err("No root certificates available".into());
    }

    Ok(root_store)
}

/// Parse TLS configuration block for ACME settings.
///
/// This is the entry point for parsing the `tls { provider acme; ... }` block.
#[allow(clippy::too_many_arguments)]
pub fn parse_acme_config(
    config: &ServerConfigurationBlock,
    domain: &str,
    port: u16,
    memory_account_cache: Arc<RwLock<std::collections::HashMap<String, Vec<u8>>>>,
    tls_alpn_01_resolvers: Arc<RwLock<Vec<TlsAlpn01DataLock>>>,
    http_01_resolvers: Arc<RwLock<Vec<Http01DataLock>>>,
    sni_resolver_lock: SniResolverLock,
    dns_client: Option<Arc<dyn DnsClient>>,
) -> Result<AcmeConfigOrOnDemand, Box<dyn std::error::Error + Send + Sync>> {
    let challenge_type = first_value(config, "challenge")
        .and_then(|s| crate::challenge::parse_challenge_type(&s))
        .unwrap_or(ChallengeType::Http01);

    let contact = first_value(config, "contact")
        .map(|c| vec![format!("mailto:{c}")])
        .unwrap_or_default();

    let directory = resolve_directory(config);
    let eab_key = parse_eab(config);
    let profile = first_value(config, "profile");
    let on_demand = config.get_flag("on_demand");
    let no_verification = config.get_flag("no_verification");

    let save_paths =
        first_value(config, "save").and_then(|path_str| {
            // Check if there's a second value (separate directive entry)
            let entries = config.directives.get("save")?;
            if entries.len() >= 2 {
                let cert_path = entries[0].args.first().and_then(|v| {
                    v.as_string_with_interpolations(&std::collections::HashMap::new())
                })?;
                let key_path = entries[1].args.first().and_then(|v| {
                    v.as_string_with_interpolations(&std::collections::HashMap::new())
                })?;
                Some((PathBuf::from(cert_path), PathBuf::from(key_path)))
            } else {
                // Single value: just the cert path, key at same location with .key extension
                let p = PathBuf::from(&path_str);
                let mut key_path = p.clone();
                key_path.set_extension("key");
                Some((p, key_path))
            }
        });

    let post_obtain_command = first_value(config, "post_obtain_command");

    // Build the TLS client config for ACME communication
    let client_config = build_rustls_client_config(no_verification)?;

    if on_demand {
        let cache_path = resolve_cache_path(config);

        let on_demand_ask = first_value(config, "on_demand_ask");
        let on_demand_ask_no_verification = config.get_flag("on_demand_ask_no_verification");

        Ok(AcmeConfigOrOnDemand::OnDemand(AcmeOnDemandConfig {
            rustls_client_config: client_config,
            challenge_type,
            contact,
            directory,
            eab_key,
            profile,
            cache_path,
            sni_resolver_lock,
            tls_alpn_01_resolver_lock: tls_alpn_01_resolvers,
            http_01_resolver_lock: http_01_resolvers,
            dns_client,
            sni_hostname: Some(domain.to_string()),
            port,
            on_demand_ask,
            on_demand_ask_no_verification,
        }))
    } else {
        let certified_key_lock = Arc::new(RwLock::new(None));
        let tls_alpn_01_data_lock = Arc::new(RwLock::new(None));
        let http_01_data_lock = Arc::new(RwLock::new(None));

        // Add challenge data locks to shared resolver lists
        match &challenge_type {
            ChallengeType::TlsAlpn01 => {
                tls_alpn_01_resolvers
                    .blocking_write()
                    .push(tls_alpn_01_data_lock.clone());
            }
            ChallengeType::Http01 => {
                http_01_resolvers
                    .blocking_write()
                    .push(http_01_data_lock.clone());
            }
            _ => {}
        }

        let cache_path = resolve_cache_path(config);

        let account_cache = if let Some(ref path) = cache_path {
            AcmeCache::File(path.clone())
        } else {
            AcmeCache::Memory(memory_account_cache)
        };

        let certificate_cache = if let Some(ref path) = cache_path {
            AcmeCache::File(path.clone())
        } else {
            AcmeCache::Memory(Arc::new(RwLock::new(std::collections::HashMap::new())))
        };

        Ok(AcmeConfigOrOnDemand::Eager(AcmeConfig {
            rustls_client_config: client_config,
            domains: vec![domain.to_string()],
            challenge_type,
            contact,
            directory,
            eab_key,
            profile,
            account_cache,
            certificate_cache,
            certified_key_lock,
            tls_alpn_01_data_lock,
            http_01_data_lock,
            dns_client,
            account: None,
            save_paths,
            post_obtain_command,
        }))
    }
}

/// Represents either an eager or on-demand ACME config.
pub enum AcmeConfigOrOnDemand {
    Eager(AcmeConfig),
    OnDemand(AcmeOnDemandConfig),
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferron_core::config::{ServerConfigurationDirectiveEntry, ServerConfigurationValue};
    use std::collections::HashMap;

    fn make_config(entries: Vec<(&str, Vec<&str>)>) -> ServerConfigurationBlock {
        let mut directives: HashMap<String, Vec<ServerConfigurationDirectiveEntry>> =
            HashMap::new();
        for (name, values) in entries {
            directives.insert(
                name.to_string(),
                values
                    .into_iter()
                    .map(|v| ServerConfigurationDirectiveEntry {
                        args: vec![ServerConfigurationValue::String(v.to_string(), None)],
                        children: None,
                        span: None,
                    })
                    .collect(),
            );
        }
        ServerConfigurationBlock {
            directives: Arc::new(directives),
            matchers: HashMap::new(),
            span: None,
        }
    }

    #[test]
    fn test_parse_challenge_type() {
        assert!(crate::challenge::parse_challenge_type("http-01").is_some());
        assert!(crate::challenge::parse_challenge_type("tls-alpn-01").is_some());
        assert!(crate::challenge::parse_challenge_type("dns-01").is_some());
        assert!(crate::challenge::parse_challenge_type("invalid").is_none());
    }

    #[test]
    fn test_resolve_directory_default() {
        let config = make_config(vec![]);
        let dir = resolve_directory(&config);
        assert_eq!(dir, "https://acme-v02.api.letsencrypt.org/directory");
    }

    #[test]
    fn test_resolve_directory_custom() {
        let config = make_config(vec![(
            "directory",
            vec!["https://acme-staging.api.letsencrypt.org/directory"],
        )]);
        let dir = resolve_directory(&config);
        assert_eq!(dir, "https://acme-staging.api.letsencrypt.org/directory");
    }

    #[test]
    fn test_first_value() {
        let config = make_config(vec![("contact", vec!["admin@example.com"])]);
        assert_eq!(
            first_value(&config, "contact"),
            Some("admin@example.com".to_string())
        );
    }
}
