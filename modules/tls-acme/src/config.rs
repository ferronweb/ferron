//! ACME configuration types and parsing from `ServerConfigurationBlock`.
//!
//! Two modes are supported:
//! - **Eager** (`AcmeConfig`): Certificates are obtained at startup for known domains.
//! - **On-demand** (`AcmeOnDemandConfig`): Certificates are obtained lazily on first TLS handshake.
//!
//! A `fallback` block may be specified to provide alternative ACME directories
//! for retrying when the primary provider fails.

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

const LETS_ENCRYPT_PRODUCTION: &str = "https://acme-v02.api.letsencrypt.org/directory";

/// Shared type for SNI resolver locks used in on-demand mode.
pub type SniResolverLock =
    Arc<RwLock<std::collections::HashMap<String, Arc<dyn rustls::server::ResolvesServerCert>>>>;

/// A single ACME provider configuration (directory + optional parameters).
#[derive(Clone)]
pub struct AcmeProviderConfig {
    /// ACME directory URL.
    pub directory: String,
    /// Contact information for the ACME account.
    pub contact: Vec<String>,
    /// Optional EAB key.
    pub eab_key: Option<Arc<ExternalAccountKey>>,
    /// Optional ACME profile name.
    pub profile: Option<String>,
}

impl AcmeProviderConfig {
    /// Builds a provider config from a configuration block.
    pub fn from_block(
        block: &ServerConfigurationBlock,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let directory = resolve_directory(block);
        let contact = first_value(block, "contact")
            .map(|c| vec![format!("mailto:{c}")])
            .unwrap_or_default();
        let eab_key = parse_eab(block);
        let profile = first_value(block, "profile");

        Ok(Self {
            directory,
            contact,
            eab_key,
            profile,
        })
    }
}

/// List of ACME providers with fallback support.
#[derive(Clone)]
pub struct AcmeProviderList {
    /// The primary provider.
    pub primary: AcmeProviderConfig,
    /// Fallback providers.
    pub fallbacks: Vec<AcmeProviderConfig>,
}

impl AcmeProviderList {
    #[cfg(test)]
    pub(crate) fn empty() -> Self {
        Self {
            primary: AcmeProviderConfig {
                directory: "https://directory.placeholder.invalid".into(),
                contact: vec![],
                eab_key: None,
                profile: None,
            },
            fallbacks: Vec::new(),
        }
    }

    /// Builds a provider list from a configuration block.
    ///
    /// The primary provider is extracted from the enclosing `tls { }` block,
    /// and fallbacks are extracted from `fallback { ... }` blocks.
    pub fn from_block(
        config: &ServerConfigurationBlock,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let primary = AcmeProviderConfig::from_block(config)?;
        let fallbacks = config
            .directives
            .get("fallback")
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.children.as_ref())
            .flat_map(|fallback_block| AcmeProviderConfig::from_block(fallback_block).ok())
            .collect();

        Ok(Self { primary, fallbacks })
    }
}

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
    /// ACME directory URL (set from the selected provider during provisioning).
    pub directory: String,
    /// Optional EAB key (set from the selected provider during provisioning).
    pub eab_key: Option<Arc<ExternalAccountKey>>,
    /// Optional ACME profile name (set from the selected provider during provisioning).
    pub profile: Option<String>,
    /// The list of ACME providers (primary + fallbacks).
    pub provider_list: Arc<RwLock<AcmeProviderList>>,
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
    /// Error message lock
    pub error_message: Arc<parking_lot::RwLock<Option<String>>>,
    /// TLS certificate signature algorithm
    pub cert_algo: &'static rcgen::SignatureAlgorithm,
    /// TLS certificate RSA key size
    pub cert_key_size: Option<rcgen::RsaKeySize>,
    // Whether TLS certificate signature algortihm is implicit
    pub cert_algo_default: bool,
}

/// On-demand ACME configuration for lazy certificate issuance.
pub struct AcmeOnDemandConfig {
    /// The Rustls client configuration for ACME communication.
    pub rustls_client_config: ClientConfig,
    /// The type of challenge to use.
    pub challenge_type: ChallengeType,
    /// Contact information for the ACME account.
    pub contact: Vec<String>,
    /// The list of ACME providers (primary + fallbacks).
    pub provider_list: Arc<RwLock<AcmeProviderList>>,
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
    /// Optional authentication for the on-demand ask endpoint.
    pub on_demand_ask_auth: Option<String>,
    /// Whether to skip TLS verification for the on-demand ask endpoint.
    pub on_demand_ask_no_verification: bool,
    /// Error message lock
    pub error_message: Arc<parking_lot::RwLock<Option<String>>>,
    /// The profile for this provider.
    pub profile: Option<String>,
    /// TLS certificate signature algorithm
    pub cert_algo: &'static rcgen::SignatureAlgorithm,
    /// TLS certificate RSA key size
    pub cert_key_size: Option<rcgen::RsaKeySize>,
    // Whether TLS certificate signature algortihm is implicit
    pub cert_algo_default: bool,
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
    /// The list of ACME providers (primary + fallbacks).
    pub provider_list: Arc<RwLock<AcmeProviderList>>,
    /// Path to the cache directory for on-domain caching.
    pub cache_path: Option<PathBuf>,
    /// DNS provider for DNS-01 challenges.
    pub dns_client: Option<Arc<dyn DnsClient>>,
    /// The SNI hostname pattern to match.
    pub sni_hostname: Option<String>,
    /// The profile for this provider.
    pub profile: Option<String>,
    /// ACME directory URL (from the primary provider's data).
    pub directory: String,
    /// Optional EAB key (from the primary provider's data).
    pub eab_key: Option<Arc<ExternalAccountKey>>,
    /// The port this config applies to.
    pub port: u16,
    /// Optional endpoint to ask before issuing a certificate.
    pub on_demand_ask: Option<String>,
    /// Optional authentication for the on-demand ask endpoint.
    pub on_demand_ask_auth: Option<String>,
    /// Whether to skip TLS verification for the on-demand ask endpoint.
    pub on_demand_ask_no_verification: bool,
    /// Error message lock
    pub error_message: Arc<parking_lot::RwLock<Option<String>>>,
    /// TLS certificate signature algorithm
    pub cert_algo: &'static rcgen::SignatureAlgorithm,
    /// TLS certificate RSA key size
    pub cert_key_size: Option<rcgen::RsaKeySize>,
    // Whether TLS certificate signature algortihm is implicit
    pub cert_algo_default: bool,
}

impl AcmeOnDemandConfig {
    /// Extracts the cloneable data portion for use by the background task.
    pub fn clone_for_state(&self) -> AcmeOnDemandConfigData {
        let provider_list = self.provider_list.blocking_read();
        let primary = &provider_list.primary;
        AcmeOnDemandConfigData {
            rustls_client_config: self.rustls_client_config.clone(),
            challenge_type: self.challenge_type.clone(),
            contact: self.contact.clone(),
            provider_list: self.provider_list.clone(),
            cache_path: self.cache_path.clone(),
            dns_client: self.dns_client.clone(),
            sni_hostname: self.sni_hostname.clone(),
            port: self.port,
            on_demand_ask: self.on_demand_ask.clone(),
            on_demand_ask_auth: self.on_demand_ask_auth.clone(),
            on_demand_ask_no_verification: self.on_demand_ask_no_verification,
            error_message: self.error_message.clone(),
            profile: primary.profile.clone(),
            directory: primary.directory.clone(),
            eab_key: primary.eab_key.clone(),
            cert_algo: self.cert_algo,
            cert_key_size: self.cert_key_size,
            cert_algo_default: self.cert_algo_default,
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
            provider_list: data.provider_list,
            cache_path: data.cache_path,
            sni_resolver_lock,
            tls_alpn_01_resolver_lock,
            http_01_resolver_lock,
            dns_client: data.dns_client,
            sni_hostname: data.sni_hostname,
            port: data.port,
            on_demand_ask: data.on_demand_ask,
            on_demand_ask_auth: data.on_demand_ask_auth,
            on_demand_ask_no_verification: data.on_demand_ask_no_verification,
            error_message: data.error_message,
            profile: data.profile,
            cert_algo: data.cert_algo,
            cert_key_size: data.cert_key_size,
            cert_algo_default: data.cert_algo_default,
        }
    }
}

/// Get the first string value for a directive.
fn first_value(config: &ServerConfigurationBlock, name: &str) -> Option<String> {
    config
        .get_value(name)
        .and_then(|v| v.as_string_with_interpolations(&std::collections::HashMap::new()))
}

/// Resolves the ACME directory URL from config.
///
/// Supports both single-value and multi-value syntax:
/// - `directory "https://..."`
/// - `directory "https://..." "https://..."`
///
/// Defaults to Let's Encrypt Production.
pub fn resolve_directory(config: &ServerConfigurationBlock) -> String {
    config
        .directives
        .get("directory")
        .and_then(|entries| entries.first())
        .and_then(|entry| entry.args.first())
        .and_then(|v| v.as_string_with_interpolations(&std::collections::HashMap::new()))
        .unwrap_or_else(|| LETS_ENCRYPT_PRODUCTION.to_string())
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

    // /var/cache/ferron-acme heuristic for the Docker image
    #[cfg(not(unix))]
    let mut fallback_path = None;
    #[cfg(unix)]
    let mut fallback_path = {
        use nix::fcntl::{AtFlags, AT_FDCWD};
        use nix::unistd::{faccessat, AccessFlags};

        if std::fs::metadata("/var/cache/ferron-acme").is_ok_and(|m| m.is_dir())
            && faccessat(
                AT_FDCWD,
                "/var/cache/ferron-acme",
                AccessFlags::R_OK | AccessFlags::W_OK,
                AtFlags::empty(),
            )
            .is_ok()
        {
            Some(PathBuf::from("/var/cache/ferron-acme"))
        } else {
            None
        }
    };

    // Default to platform data directory (if writable)
    if fallback_path.is_none() {
        fallback_path = dirs::data_local_dir().map(|mut p| {
            p.push("ferron-acme");
            p
        });
    }

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
            .with_safe_default_protocol_versions()
            .expect("failed to initialize Rustls client builder")
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoVerifier))
            .with_no_client_auth())
    } else {
        let root_store = build_root_cert_store()?;

        Ok(ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("failed to initialize Rustls client builder")
            .with_root_certificates(root_store)
            .with_no_client_auth())
    }
}

/// Resolve a TLS certificate configuration from the given `config`.
fn resolve_tls_cert_config(
    config: &ServerConfigurationBlock,
) -> Result<
    (
        &'static rcgen::SignatureAlgorithm,
        bool,
        Option<rcgen::RsaKeySize>,
    ),
    Box<dyn std::error::Error + Send + Sync>,
> {
    let (algo, algo_default, is_rsa) = resolve_tls_cert_algo_config(config)?;
    let key_size = resolve_tls_cert_key_config(config, is_rsa)?;
    Ok((algo, algo_default, key_size))
}

#[inline]
pub(crate) fn resolve_tls_cert_algo_config(
    config: &ServerConfigurationBlock,
) -> Result<
    (&'static rcgen::SignatureAlgorithm, bool, bool),
    Box<dyn std::error::Error + Send + Sync>,
> {
    match first_value(config, "key_algo")
        .map(|v| v.to_uppercase())
        .as_deref()
    {
        Some("PKCS_ECDSA_P256_SHA256") | Some("ECDSA") => {
            Ok((&rcgen::PKCS_ECDSA_P256_SHA256, false, false))
        }
        Some("PKCS_ECDSA_P384_SHA384") => Ok((&rcgen::PKCS_ECDSA_P384_SHA384, false, false)),
        Some("PKCS_ECDSA_P521_SHA256") => Ok((&rcgen::PKCS_ECDSA_P521_SHA256, false, false)),
        Some("PKCS_ECDSA_P521_SHA384") => Ok((&rcgen::PKCS_ECDSA_P521_SHA384, false, false)),
        Some("PKCS_ECDSA_P521_SHA512") => Ok((&rcgen::PKCS_ECDSA_P521_SHA512, false, false)),

        Some("PKCS_ED25519") => Ok((&rcgen::PKCS_ED25519, false, false)),

        Some("PKCS_ML_DSA_44") => Ok((&rcgen::PKCS_ML_DSA_44, false, false)),
        Some("PKCS_ML_DSA_65") => Ok((&rcgen::PKCS_ML_DSA_65, false, false)),
        Some("PKCS_ML_DSA_87") => Ok((&rcgen::PKCS_ML_DSA_87, false, false)),

        Some("PKCS_RSA_SHA256") | Some("RSA") => Ok((&rcgen::PKCS_RSA_SHA256, false, true)),
        Some("PKCS_RSA_SHA384") => Ok((&rcgen::PKCS_RSA_SHA384, false, true)),
        Some("PKCS_RSA_SHA512") => Ok((&rcgen::PKCS_RSA_SHA512, false, true)),

        None => {
            if config
                .get_value("key_size")
                .and_then(|v| v.as_number())
                .is_some()
            {
                // Similar to rcgen defaults (SHA256 hash), but it's RSA
                Ok((&rcgen::PKCS_RSA_SHA256, true, true))
            } else {
                // Same as rcgen defaults
                Ok((&rcgen::PKCS_ECDSA_P256_SHA256, true, false))
            }
        }
        _ => Err(
            anyhow::anyhow!("unsupported private key signature algorithm").into_boxed_dyn_error(),
        ),
    }
}

#[inline]
pub(crate) fn resolve_tls_cert_key_config(
    config: &ServerConfigurationBlock,
    is_rsa: bool,
) -> Result<Option<rcgen::RsaKeySize>, Box<dyn std::error::Error + Send + Sync>> {
    let key_size = config
        .get_value("key_size")
        .and_then(|v| v.as_number())
        .map(|sz| match sz {
            2048 => Ok(rcgen::RsaKeySize::_2048),
            3072 => Ok(rcgen::RsaKeySize::_3072),
            4096 => Ok(rcgen::RsaKeySize::_4096),
            _ => Err(anyhow::anyhow!("unsupported RSA key size").into_boxed_dyn_error()),
        })
        .transpose()?;

    if !is_rsa && key_size.is_some() {
        return Err(
            anyhow::anyhow!("RSA key sizes cannot be used with non-RSA keys")
                .into_boxed_dyn_error(),
        );
    }

    Ok(key_size)
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
    domain: Option<&str>,
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

    let provider_list = AcmeProviderList::from_block(config)?;

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

    let client_config = build_rustls_client_config(no_verification)?;
    let (algo, algo_default, key_size) = resolve_tls_cert_config(config)?;

    if on_demand {
        let cache_path = resolve_cache_path(config);

        let on_demand_ask = first_value(config, "on_demand_ask");
        let on_demand_ask_auth = first_value(config, "on_demand_ask_auth");
        let on_demand_ask_no_verification = config.get_flag("on_demand_ask_no_verification");

        // A host block without a hostname matches every SNI hostname. Without an
        // approval endpoint, anyone could make Ferron request certificates for
        // arbitrary names, so the endpoint is mandatory here.
        if domain.is_none() && on_demand_ask.is_none() {
            return Err(
                "On-demand ACME in a host block without a hostname requires `on_demand_ask`".into(),
            );
        }

        Ok(AcmeConfigOrOnDemand::OnDemand(AcmeOnDemandConfig {
            rustls_client_config: client_config,
            challenge_type,
            contact,
            provider_list: Arc::new(RwLock::new(provider_list)),
            cache_path,
            sni_resolver_lock,
            tls_alpn_01_resolver_lock: tls_alpn_01_resolvers,
            http_01_resolver_lock: http_01_resolvers,
            dns_client,
            sni_hostname: domain.map(str::to_string),
            port,
            on_demand_ask,
            on_demand_ask_auth,
            on_demand_ask_no_verification,
            error_message: Arc::new(parking_lot::RwLock::new(None)),
            profile: None,
            cert_algo: algo,
            cert_key_size: key_size,
            cert_algo_default: algo_default,
        }))
    } else {
        let domain = domain.ok_or(
            "ACME TLS provider requires a domain name or IP address unless `on_demand` is enabled",
        )?;
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
            directory: provider_list.primary.directory.clone(),
            eab_key: provider_list.primary.eab_key.clone(),
            profile: provider_list.primary.profile.clone(),
            provider_list: Arc::new(RwLock::new(provider_list)),
            account_cache,
            certificate_cache,
            certified_key_lock,
            tls_alpn_01_data_lock,
            http_01_data_lock,
            dns_client,
            account: None,
            save_paths,
            post_obtain_command,
            error_message: Arc::new(parking_lot::RwLock::new(None)),
            cert_algo: algo,
            cert_key_size: key_size,
            cert_algo_default: algo_default,
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

    fn block(directives: &[(&str, Option<&str>)]) -> ServerConfigurationBlock {
        ServerConfigurationBlock {
            directives: Arc::new(
                directives
                    .iter()
                    .map(|(name, value)| {
                        let args = match value {
                            Some(value) => {
                                vec![ServerConfigurationValue::String(value.to_string(), None)]
                            }
                            None => vec![ServerConfigurationValue::Boolean(true, None)],
                        };
                        (
                            name.to_string(),
                            vec![ServerConfigurationDirectiveEntry {
                                args,
                                ..Default::default()
                            }],
                        )
                    })
                    .collect(),
            ),
            ..Default::default()
        }
    }

    fn parse(
        config: &ServerConfigurationBlock,
        domain: Option<&str>,
    ) -> Result<AcmeConfigOrOnDemand, Box<dyn std::error::Error + Send + Sync>> {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        parse_acme_config(
            config,
            domain,
            443,
            Arc::new(RwLock::new(std::collections::HashMap::new())),
            Arc::new(RwLock::new(Vec::new())),
            Arc::new(RwLock::new(Vec::new())),
            Arc::new(RwLock::new(std::collections::HashMap::new())),
            None,
        )
    }

    #[test]
    fn catch_all_on_demand_with_ask_has_no_sni_pattern() {
        let config = block(&[
            ("on_demand", None),
            ("on_demand_ask", Some("http://127.0.0.1:8081/check")),
        ]);
        match parse(&config, None).expect("catch-all on-demand config") {
            AcmeConfigOrOnDemand::OnDemand(on_demand) => {
                assert_eq!(on_demand.sni_hostname, None);
                assert_eq!(
                    on_demand.on_demand_ask.as_deref(),
                    Some("http://127.0.0.1:8081/check")
                );
            }
            AcmeConfigOrOnDemand::Eager(_) => panic!("expected an on-demand config"),
        }
    }

    #[test]
    fn catch_all_on_demand_without_ask_is_rejected() {
        let config = block(&[("on_demand", None)]);
        let error = parse(&config, None).err().expect("missing ask endpoint");
        assert!(error.to_string().contains("on_demand_ask"));
    }

    #[test]
    fn eager_config_without_hostname_is_rejected() {
        let config = block(&[]);
        assert!(parse(&config, None).is_err());
    }

    #[test]
    fn named_on_demand_config_keeps_its_pattern_without_ask() {
        let config = block(&[("on_demand", None)]);
        match parse(&config, Some("*.example.com")).expect("named on-demand config") {
            AcmeConfigOrOnDemand::OnDemand(on_demand) => {
                assert_eq!(on_demand.sni_hostname.as_deref(), Some("*.example.com"));
            }
            AcmeConfigOrOnDemand::Eager(_) => panic!("expected an on-demand config"),
        }
    }
}
