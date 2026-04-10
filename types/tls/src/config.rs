//! Shared TLS configuration types and parsing.
//!
//! Provides configuration structs that can be parsed from a
//! [`ServerConfigurationBlock`](ferron_core::config::ServerConfigurationBlock).
//! These types are reusable across any TLS resolver provider.

use ferron_core::config::ServerConfigurationBlock;
use rustls_pki_types::pem::PemObject;
use std::sync::Arc;

/// Supported TLS cipher suites.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsCipherSuite {
    // TLS 1.3 cipher suites
    Tls13Aes128GcmSha256,
    Tls13Aes256GcmSha384,
    Tls13Chacha20Poly1305Sha256,
    // TLS 1.2 cipher suites (ECDHE_ECDSA)
    Tls12EcdheEcdsaAes128GcmSha256,
    Tls12EcdheEcdsaAes256GcmSha384,
    Tls12EcdheEcdsaChacha20Poly1305Sha256,
    // TLS 1.2 cipher suites (ECDHE_RSA)
    Tls12EcdheRsaAes128GcmSha256,
    Tls12EcdheRsaAes256GcmSha384,
    Tls12EcdheRsaChacha20Poly1305Sha256,
}

impl TlsCipherSuite {
    /// Parse a cipher suite name from a string.
    pub fn try_parse(s: &str) -> Option<Self> {
        match s {
            "TLS_AES_128_GCM_SHA256" => Some(Self::Tls13Aes128GcmSha256),
            "TLS_AES_256_GCM_SHA384" => Some(Self::Tls13Aes256GcmSha384),
            "TLS_CHACHA20_POLY1305_SHA256" => Some(Self::Tls13Chacha20Poly1305Sha256),
            "TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256" => Some(Self::Tls12EcdheEcdsaAes128GcmSha256),
            "TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384" => Some(Self::Tls12EcdheEcdsaAes256GcmSha384),
            "TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256" => {
                Some(Self::Tls12EcdheEcdsaChacha20Poly1305Sha256)
            }
            "TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256" => Some(Self::Tls12EcdheRsaAes128GcmSha256),
            "TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384" => Some(Self::Tls12EcdheRsaAes256GcmSha384),
            "TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256" => {
                Some(Self::Tls12EcdheRsaChacha20Poly1305Sha256)
            }
            _ => None,
        }
    }
}

/// Supported ECDH key exchange groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsKxGroup {
    Secp256r1,
    Secp384r1,
    X25519,
    X25519Mlkem768,
    Mlkem768,
}

impl TlsKxGroup {
    /// Parse an ECDH curve name from a string.
    pub fn try_parse(s: &str) -> Option<Self> {
        match s {
            "secp256r1" => Some(Self::Secp256r1),
            "secp384r1" => Some(Self::Secp384r1),
            "x25519" => Some(Self::X25519),
            "x25519mlkem768" => Some(Self::X25519Mlkem768),
            "mlkem768" => Some(Self::Mlkem768),
            _ => None,
        }
    }
}

/// Supported TLS protocol versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsVersion {
    Tls12,
    Tls13,
}

impl TlsVersion {
    /// Parse a TLS version string.
    pub fn try_parse(s: &str) -> Option<Self> {
        match s {
            "TLSv1.2" => Some(Self::Tls12),
            "TLSv1.3" => Some(Self::Tls13),
            _ => None,
        }
    }
}

/// Crypto provider settings parsed from configuration.
#[derive(Debug, Clone, Default)]
pub struct TlsCryptoConfig {
    /// Cipher suites to use (empty = rustls defaults).
    pub cipher_suites: Vec<TlsCipherSuite>,
    /// Key exchange groups to use (empty = rustls defaults).
    pub kx_groups: Vec<TlsKxGroup>,
    /// Minimum TLS version (None = default).
    pub min_version: Option<TlsVersion>,
    /// Maximum TLS version (None = default).
    pub max_version: Option<TlsVersion>,
}

impl TlsCryptoConfig {
    /// Parse crypto settings from a TLS configuration block.
    ///
    /// Recognized directives:
    /// - `cipher_suite <name>` (multiple allowed)
    /// - `ecdh_curve <name>` (multiple allowed)
    /// - `min_version <version>`
    /// - `max_version <version>`
    pub fn from_config(config: &ServerConfigurationBlock) -> Self {
        let cipher_suites = collect_multi_values(config, "cipher_suite")
            .iter()
            .filter_map(|s| TlsCipherSuite::try_parse(s))
            .collect();

        let kx_groups = collect_multi_values(config, "ecdh_curve")
            .iter()
            .filter_map(|s| TlsKxGroup::try_parse(s))
            .collect();

        let min_version =
            collect_first_string(config, "min_version").and_then(|s| TlsVersion::try_parse(&s));

        let max_version =
            collect_first_string(config, "max_version").and_then(|s| TlsVersion::try_parse(&s));

        Self {
            cipher_suites,
            kx_groups,
            min_version,
            max_version,
        }
    }
}

/// Source of trusted CA certificates for client certificate verification.
#[derive(Debug, Clone)]
pub enum TlsClientAuthCaSource {
    /// Use the system native trust store (via `rustls-native-certs`).
    SystemRoots,
    /// Use the Mozilla root certificates bundle (via `webpki-roots`).
    WebPkiRoots,
    /// Load a single CA certificate from the given file path.
    SingleCaCert(String),
}

/// Client authentication (mTLS) configuration.
#[derive(Debug, Clone)]
pub struct TlsClientAuthConfig {
    /// Whether client authentication is enabled.
    pub enabled: bool,
    /// Whether client certificates are required (`true`) or optional (`false`).
    /// When optional, the server offers client certs but allows anonymous connections.
    pub required: bool,
    /// Source of trusted CA certificates for verifying client certificates.
    pub ca_source: TlsClientAuthCaSource,
}

impl TlsClientAuthConfig {
    /// Parse client auth settings from a TLS configuration block.
    ///
    /// Recognized directives:
    /// - `client_auth true|false` — enables/disables mTLS (default: `false`).
    ///   When `true`, client certs are required.
    /// - `client_auth_ca <path|system|webpki>` — CA certificate source:
    ///   - A file path — load a single CA cert
    ///   - `system` — native OS trust store
    ///   - `webpki` — Mozilla roots fallback
    ///
    /// If `client_auth_ca` is omitted, defaults to `webpki`.
    pub fn from_config(config: &ServerConfigurationBlock) -> Self {
        let enabled = config
            .get_value("client_auth")
            .map(|v| {
                // Accept both boolean and string "true"/"false"
                if let Some(b) = v.as_boolean() {
                    b
                } else if let Some(s) = v.as_str() {
                    s.eq_ignore_ascii_case("true")
                } else {
                    false
                }
            })
            .unwrap_or(false);

        // client_auth as a boolean: true => required, false => disabled
        // We keep it simple: enabled=true means required=true
        let required = enabled;

        let ca_source = match collect_first_string(config, "client_auth_ca").as_deref() {
            Some("system") => TlsClientAuthCaSource::SystemRoots,
            Some("webpki") => TlsClientAuthCaSource::WebPkiRoots,
            Some(path) => TlsClientAuthCaSource::SingleCaCert(path.to_string()),
            None => TlsClientAuthCaSource::WebPkiRoots, // default fallback
        };

        Self {
            enabled,
            required,
            ca_source,
        }
    }
}

/// Complete TLS server configuration parsed from a TLS config block.
#[derive(Debug, Clone)]
pub struct TlsServerConfig {
    /// Path to the server certificate file.
    pub cert_path: String,
    /// Path to the server private key file.
    pub key_path: String,
    /// Crypto provider settings (cipher suites, curves, versions).
    pub crypto: TlsCryptoConfig,
    /// Client authentication (mTLS) settings.
    pub client_auth: TlsClientAuthConfig,
}

impl TlsServerConfig {
    /// Parse full TLS config from a TLS configuration block.
    ///
    /// Requires `cert` and `key` directives. All other directives are optional.
    pub fn from_config(config: &ServerConfigurationBlock) -> Result<Self, String> {
        let cert_path = collect_first_string(config, "cert")
            .ok_or_else(|| "'cert' TLS parameter missing or invalid".to_string())?;
        let key_path = collect_first_string(config, "key")
            .ok_or_else(|| "'key' TLS parameter missing or invalid".to_string())?;

        let crypto = TlsCryptoConfig::from_config(config);
        let client_auth = TlsClientAuthConfig::from_config(config);

        Ok(Self {
            cert_path,
            key_path,
            crypto,
            client_auth,
        })
    }
}

// --- helpers ---

/// Collect all string values for a multi-value directive (e.g. `cipher_suite`).
fn collect_multi_values(config: &ServerConfigurationBlock, name: &str) -> Vec<String> {
    config
        .directives
        .get(name)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|e| e.args.first())
                .filter_map(|v| {
                    v.as_string_with_interpolations(
                        &std::collections::HashMap::<String, String>::new(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Get the first string value for a directive.
fn collect_first_string(config: &ServerConfigurationBlock, name: &str) -> Option<String> {
    config.get_value(name).and_then(|v| {
        v.as_string_with_interpolations(&std::collections::HashMap::<String, String>::new())
    })
}

/// Build a `rustls::RootCertStore` from a [`TlsClientAuthCaSource`].
///
/// This is a helper used by the builder module and by any provider
/// that needs to construct a root store for client verification.
pub fn build_root_cert_store(
    ca_source: &TlsClientAuthCaSource,
) -> Result<Arc<rustls::RootCertStore>, Box<dyn std::error::Error>> {
    let mut root_store = rustls::RootCertStore::empty();

    match ca_source {
        TlsClientAuthCaSource::SingleCaCert(path) => {
            let certs = load_ca_cert_file(path)?;
            for cert in certs {
                root_store.add(cert)?;
            }
        }
        TlsClientAuthCaSource::SystemRoots => {
            #[cfg(feature = "native-certs")]
            {
                let native_certs = rustls_native_certs::load_native_certs();
                for err in native_certs.errors {
                    ferron_core::log_warn!("Failed to load native root cert: {}", err);
                }
                let count = native_certs.certs.len();
                root_store.add_parsable_certificates(native_certs.certs);
                ferron_core::log_info!("Loaded {} native root certificates", count);
            }
            #[cfg(not(feature = "native-certs"))]
            {
                return Err(
                    "native-certs feature not enabled; recompile with --features native-certs"
                        .into(),
                );
            }
        }
        TlsClientAuthCaSource::WebPkiRoots => {
            #[cfg(feature = "webpki-roots")]
            {
                root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
                ferron_core::log_info!(
                    "Loaded {} webpki root certificates",
                    webpki_roots::TLS_SERVER_ROOTS.len()
                );
            }
            #[cfg(not(feature = "webpki-roots"))]
            {
                return Err(
                    "webpki-roots feature not enabled; recompile with --features webpki-roots"
                        .into(),
                );
            }
        }
    }

    Ok(Arc::new(root_store))
}

/// Load CA certificates from a PEM file.
fn load_ca_cert_file(
    path: &str,
) -> Result<Vec<rustls_pki_types::CertificateDer<'static>>, std::io::Error> {
    let mut file = std::fs::File::open(path)?;
    rustls_pki_types::CertificateDer::pem_reader_iter(&mut file)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| match e {
            rustls_pki_types::pem::Error::Io(err) => err,
            err => std::io::Error::other(err),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferron_core::config::{
        ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationValue,
    };
    use std::collections::HashMap;
    use std::sync::Arc;

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
    fn test_cipher_suite_parsing() {
        assert!(TlsCipherSuite::try_parse("TLS_AES_128_GCM_SHA256").is_some());
        assert!(TlsCipherSuite::try_parse("TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384").is_some());
        assert!(TlsCipherSuite::try_parse("INVALID_SUITE").is_none());
    }

    #[test]
    fn test_kx_group_parsing() {
        assert!(TlsKxGroup::try_parse("x25519").is_some());
        assert!(TlsKxGroup::try_parse("secp256r1").is_some());
        assert!(TlsKxGroup::try_parse("invalid").is_none());
    }

    #[test]
    fn test_tls_version_parsing() {
        assert_eq!(TlsVersion::try_parse("TLSv1.2"), Some(TlsVersion::Tls12));
        assert_eq!(TlsVersion::try_parse("TLSv1.3"), Some(TlsVersion::Tls13));
        assert_eq!(TlsVersion::try_parse("TLSv1.1"), None);
    }

    #[test]
    fn test_crypto_config_from_config() {
        let config = make_config(vec![
            (
                "cipher_suite",
                vec!["TLS_AES_128_GCM_SHA256", "TLS_AES_256_GCM_SHA384"],
            ),
            ("ecdh_curve", vec!["x25519", "secp256r1"]),
            ("min_version", vec!["TLSv1.2"]),
            ("max_version", vec!["TLSv1.3"]),
        ]);

        let crypto = TlsCryptoConfig::from_config(&config);
        assert_eq!(crypto.cipher_suites.len(), 2);
        assert_eq!(crypto.kx_groups.len(), 2);
        assert_eq!(crypto.min_version, Some(TlsVersion::Tls12));
        assert_eq!(crypto.max_version, Some(TlsVersion::Tls13));
    }

    #[test]
    fn test_client_auth_config_defaults() {
        let config = make_config(vec![
            ("cert", vec!["/path/cert.pem"]),
            ("key", vec!["/path/key.pem"]),
        ]);
        let client_auth = TlsClientAuthConfig::from_config(&config);
        assert!(!client_auth.enabled);
        assert!(!client_auth.required);
        assert!(matches!(
            client_auth.ca_source,
            TlsClientAuthCaSource::WebPkiRoots
        ));
    }

    #[test]
    fn test_client_auth_config_enabled_with_ca_path() {
        let config = make_config(vec![
            ("client_auth", vec!["true"]),
            ("client_auth_ca", vec!["/path/ca.pem"]),
        ]);
        let client_auth = TlsClientAuthConfig::from_config(&config);
        assert!(client_auth.enabled);
        assert!(client_auth.required);
        if let TlsClientAuthCaSource::SingleCaCert(p) = &client_auth.ca_source {
            assert_eq!(p, "/path/ca.pem");
        } else {
            panic!("expected SingleCaCert");
        }
    }

    #[test]
    fn test_client_auth_config_system_roots() {
        let config = make_config(vec![
            ("client_auth", vec!["true"]),
            ("client_auth_ca", vec!["system"]),
        ]);
        let client_auth = TlsClientAuthConfig::from_config(&config);
        assert!(client_auth.enabled);
        assert!(matches!(
            client_auth.ca_source,
            TlsClientAuthCaSource::SystemRoots
        ));
    }

    #[test]
    fn test_tls_server_config_full() {
        let config = make_config(vec![
            ("cert", vec!["/path/cert.pem"]),
            ("key", vec!["/path/key.pem"]),
            ("cipher_suite", vec!["TLS_AES_128_GCM_SHA256"]),
            ("ecdh_curve", vec!["x25519"]),
            ("min_version", vec!["TLSv1.3"]),
            ("max_version", vec!["TLSv1.3"]),
            ("client_auth", vec!["true"]),
            ("client_auth_ca", vec!["system"]),
        ]);

        let tls_config = TlsServerConfig::from_config(&config).unwrap();
        assert_eq!(tls_config.cert_path, "/path/cert.pem");
        assert_eq!(tls_config.key_path, "/path/key.pem");
        assert_eq!(tls_config.crypto.cipher_suites.len(), 1);
        assert_eq!(tls_config.crypto.kx_groups.len(), 1);
        assert_eq!(tls_config.crypto.min_version, Some(TlsVersion::Tls13));
        assert!(tls_config.client_auth.enabled);
        assert!(matches!(
            tls_config.client_auth.ca_source,
            TlsClientAuthCaSource::SystemRoots
        ));
    }

    #[test]
    fn test_tls_server_config_missing_cert() {
        let config = make_config(vec![("key", vec!["/path/key.pem"])]);
        let result = TlsServerConfig::from_config(&config);
        assert!(result.is_err());
    }
}
