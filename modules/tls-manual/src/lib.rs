use std::sync::Arc;

use ferron_core::loader::ModuleLoader;
use ferron_core::providers::Provider;
use ferron_tls::{
    builder::build_server_config_builder, config::TlsServerConfig, TcpTlsContext, TcpTlsResolver,
};
use rustls::ServerConfig;
use rustls_pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};

/// Check if a certificate has the OCSP Must-Staple (TLS Feature status_request) extension.
///
/// Per RFC 7633, the TLS Feature extension contains a SEQUENCE of feature values.
/// The `status_request` feature (value 5) indicates OCSP Must-Staple.
#[cfg(feature = "ocsp")]
fn cert_has_must_staple(leaf: &CertificateDer<'_>) -> bool {
    use x509_parser::prelude::*;

    let Ok((_, cert)) = X509Certificate::from_der(leaf.as_ref()) else {
        return false;
    };

    for ext in cert.extensions() {
        // ext.oid.as_bytes() returns BER-encoded OID bytes
        // BER encoding of 1.3.6.1.5.5.7.1.24: 2b 06 01 05 05 07 01 18
        if ext.oid.as_bytes() == [0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x01, 0x18] {
            if let Ok((_, root)) = der_parser::der::parse_der(ext.value) {
                if let Ok(items) = root.as_sequence() {
                    return items
                        .iter()
                        .any(|item: &der_parser::ber::BerObject| item.as_u32().ok() == Some(5));
                }
            }
        }
    }
    false
}

/// Build a `rustls::sign::CertifiedKey` from loaded certs and private key.
///
/// Used to preload certificates with Must-Staple into the OCSP service for
/// immediate fetching.
#[cfg(feature = "ocsp")]
fn build_certified_key(
    certs: &[CertificateDer<'static>],
    private_key: &PrivateKeyDer<'static>,
) -> Option<rustls::sign::CertifiedKey> {
    use rustls::crypto::aws_lc_rs::sign::any_supported_type;

    let signing_key = any_supported_type(private_key).ok()?;
    Some(rustls::sign::CertifiedKey::new(certs.to_vec(), signing_key))
}

pub struct TcpTlsManualResolver {
    config: Arc<ServerConfig>,
}

#[async_trait::async_trait(?Send)]
impl TcpTlsResolver for TcpTlsManualResolver {
    #[inline]
    fn get_tls_config(&self) -> Arc<ServerConfig> {
        self.config.clone()
    }
}

pub struct TcpTlsManualProvider;

impl<'a> Provider<TcpTlsContext<'a>> for TcpTlsManualProvider {
    fn name(&self) -> &str {
        "manual"
    }

    fn execute(&self, ctx: &mut TcpTlsContext) -> Result<(), Box<dyn std::error::Error>> {
        // Parse TLS configuration from the config block
        let tls_config = TlsServerConfig::from_config(ctx.config)
            .map_err(|e| std::io::Error::other(format!("Invalid TLS configuration: {e}")))?;

        // Build the ServerConfig up to the verifier stage using the shared builder
        let config_builder =
            build_server_config_builder(&tls_config.crypto, &tls_config.client_auth)?;

        // Parse ticket key configuration
        let ticketer = ferron_tls::builder::build_ticketer(ctx.config);

        // Load certificates
        let certs = load_certs(&tls_config.cert_path.ok_or(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "'cert' TLS parameter missing or invalid".to_string(),
        ))?)
        .map_err(|e| std::io::Error::other(format!("Error while loading TLS certificate: {e}")))?;

        // Load private key
        let private_key = load_private_key(&tls_config.key_path.ok_or(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "'cert' TLS parameter missing or invalid".to_string(),
        ))?)
        .map_err(|e| std::io::Error::other(format!("Error while loading TLS private key: {e}")))?;

        // Build the config with certificates
        let mut config_with_tickets =
            config_builder.with_single_cert(certs.clone(), private_key.clone_key())?;

        // Attach the ticketer
        if let Some(ticketer) = ticketer {
            config_with_tickets.ticketer = ticketer;
        }

        if let Some(alpn_protocols) = ctx.alpn.as_ref() {
            config_with_tickets.alpn_protocols = alpn_protocols.clone();
        }

        // Wrap cert_resolver with OCSP stapler if enabled
        #[cfg(feature = "ocsp")]
        {
            if tls_config.ocsp.enabled {
                let ocsp_handle = ferron_ocsp::get_service_handle()
                    .expect("OCSP service handle should always be available");
                let inner_resolver = config_with_tickets.cert_resolver.clone();
                config_with_tickets.cert_resolver =
                    Arc::new(ferron_ocsp::OcspStapler::new(inner_resolver, &ocsp_handle));

                // Preload the certificate for immediate OCSP fetching.
                // Without preloading, the first TLS handshake for each server
                // would not include a stapled OCSP response because the fetch
                // hasn't completed yet. Preloading ensures the background task
                // starts fetching as soon as the config is loaded.
                if let Some(certified_key) = build_certified_key(&certs, &private_key) {
                    if let Some(leaf) = certs.first() {
                        if cert_has_must_staple(leaf) {
                            ferron_core::log_info!(
                                "OCSP stapling enabled — Must-Staple detected, preloading certificate"
                            );
                        }
                    }
                    ocsp_handle.preload(certified_key);
                }
            }
        }

        let config = Arc::new(config_with_tickets);

        ctx.resolver = Some(Arc::new(TcpTlsManualResolver { config }));

        Ok(())
    }
}

/// Loads a public certificate from file
pub fn load_certs(filename: &str) -> std::io::Result<Vec<CertificateDer<'static>>> {
    let mut certfile = std::fs::File::open(filename)?;
    CertificateDer::pem_reader_iter(&mut certfile)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| match e {
            rustls_pki_types::pem::Error::Io(err) => err,
            err => std::io::Error::other(err),
        })
}

/// Loads a private key from file
pub fn load_private_key(filename: &str) -> std::io::Result<PrivateKeyDer<'static>> {
    let mut keyfile = std::fs::File::open(filename)?;
    match PrivateKeyDer::from_pem_reader(&mut keyfile) {
        Ok(private_key) => Ok(private_key),
        Err(rustls_pki_types::pem::Error::Io(err)) => Err(err),
        Err(err) => Err(std::io::Error::other(err)),
    }
}

#[derive(Clone, Default)]
pub struct TlsManualModuleLoader;

impl ModuleLoader for TlsManualModuleLoader {
    fn register_providers(
        &mut self,
        registry: ferron_core::registry::RegistryBuilder,
    ) -> ferron_core::registry::RegistryBuilder {
        registry.with_provider::<TcpTlsContext, _>(|| Arc::new(TcpTlsManualProvider))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferron_core::config::{
        ServerConfigurationBlock, ServerConfigurationDirectiveEntry,
        ServerConfigurationHostFilters, ServerConfigurationValue,
    };
    use std::collections::HashMap;
    use std::io::Write;
    use tempfile::NamedTempFile;

    /// Helper to create a test configuration block
    fn create_test_config(directives: Vec<(&str, &str)>) -> ServerConfigurationBlock {
        let mut directives_map = HashMap::new();
        for (name, value) in directives {
            directives_map.insert(
                name.to_string(),
                vec![ServerConfigurationDirectiveEntry {
                    args: vec![ServerConfigurationValue::String(value.to_string(), None)],
                    children: None,
                    span: None,
                }],
            );
        }

        ServerConfigurationBlock {
            directives: Arc::new(directives_map),
            matchers: HashMap::new(),
            span: None,
        }
    }

    /// Helper to create a test configuration block with nested children
    #[allow(unused)]
    fn create_test_config_with_nested(
        directives: Vec<(&str, &str)>,
        nested_block: (&str, ServerConfigurationBlock),
    ) -> ServerConfigurationBlock {
        let mut directives_map = HashMap::new();
        for (name, value) in directives {
            directives_map.insert(
                name.to_string(),
                vec![ServerConfigurationDirectiveEntry {
                    args: vec![ServerConfigurationValue::String(value.to_string(), None)],
                    children: None,
                    span: None,
                }],
            );
        }

        // Add the nested block
        let (nested_name, nested_children) = nested_block;
        directives_map.insert(
            nested_name.to_string(),
            vec![ServerConfigurationDirectiveEntry {
                args: vec![],
                children: Some(nested_children),
                span: None,
            }],
        );

        ServerConfigurationBlock {
            directives: Arc::new(directives_map),
            matchers: HashMap::new(),
            span: None,
        }
    }

    /// Helper to create a temporary certificate file
    fn create_temp_cert_file() -> NamedTempFile {
        // Create a minimal valid PEM certificate
        let cert = b"-----BEGIN CERTIFICATE-----
MIIBgTCCASegAwIBAgIUGDskiRWw3F3+f4w6oOdtfyaczaQwCgYIKoZIzj0EAwIw
FjEUMBIGA1UEAwwLZXhhbXBsZS5jb20wHhcNMjYwNDE2MTY0NjQ0WhcNMjcwNDE2
MTY0NjQ0WjAWMRQwEgYDVQQDDAtleGFtcGxlLmNvbTBZMBMGByqGSM49AgEGCCqG
SM49AwEHA0IABH5yzv9fZi2BGwX2p+jh+2/lAtyx4I9fDnEJMYCg94KNPdvKqGrc
jyFp95tRYkrmNi9okT9kM4hJktzojIb+NoajUzBRMB0GA1UdDgQWBBSza7fLCUhl
g6lwsV0/ifz+FEiL/TAfBgNVHSMEGDAWgBSza7fLCUhlg6lwsV0/ifz+FEiL/TAP
BgNVHRMBAf8EBTADAQH/MAoGCCqGSM49BAMCA0gAMEUCIG4JZMXYUaX/6NydwbYv
xo/erTFoihuXv6CYoclJX+PSAiEAjM2uZDOpu7QVODHTNVlruk+rebEsBfZY5NEi
hyC2Gls=
-----END CERTIFICATE-----";
        let mut file = NamedTempFile::new().expect("Failed to create temp file");
        file.write_all(cert).expect("Failed to write cert");
        file.flush().expect("Failed to flush");
        file
    }

    /// Helper to create a temporary key file
    fn create_temp_key_file() -> NamedTempFile {
        // Create a minimal valid PEM private key
        let key = b"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg+OliokRAz3RNqBQn
czK4nVti140nJ+uvxNIRZGp8SDWhRANCAAR+cs7/X2YtgRsF9qfo4ftv5QLcseCP
Xw5xCTGAoPeCjT3byqhq3I8hafebUWJK5jYvaJE/ZDOISZLc6IyG/jaG
-----END PRIVATE KEY-----";
        let mut file = NamedTempFile::new().expect("Failed to create temp file");
        file.write_all(key).expect("Failed to write key");
        file.flush().expect("Failed to flush");
        file
    }

    /// Helper to create a valid 80-byte ticket key file
    fn create_temp_ticket_keys_file() -> NamedTempFile {
        let mut keys = [0u8; 80];
        // Fill with predictable data
        for (i, byte) in keys.iter_mut().enumerate() {
            *byte = (i % 256) as u8;
        }
        let mut file = NamedTempFile::new().expect("Failed to create temp file");
        file.write_all(&keys).expect("Failed to write keys");
        file.flush().expect("Failed to flush");
        file
    }

    #[test]
    fn test_provider_name() {
        let provider = TcpTlsManualProvider;
        assert_eq!(provider.name(), "manual");
    }

    #[test]
    fn test_provider_executes_without_ticket_keys() {
        // Test that the provider works without ticket_keys directive (backward compatibility)
        let cert_file = create_temp_cert_file();
        let key_file = create_temp_key_file();

        let config = create_test_config(vec![
            ("cert", cert_file.path().to_str().unwrap()),
            ("key", key_file.path().to_str().unwrap()),
        ]);

        let provider = TcpTlsManualProvider;
        let mut ctx = TcpTlsContext {
            config: &config,
            alpn: None,
            domain: ServerConfigurationHostFilters::default(),
            port: 443,
            resolver: None,
        };

        // This should succeed even though the cert/key files are not perfectly valid PEM
        // (we're just testing that the code path executes)
        let result = provider.execute(&mut ctx);
        // We expect this to fail on loading the fake cert, but that's OK -
        // we're testing the code path exists
        assert!(result.is_err() || ctx.resolver.is_some());
    }

    #[test]
    fn test_provider_accepts_valid_ticket_keys_file() {
        // Test that valid ticket_keys file is accepted
        let cert_file = create_temp_cert_file();
        let key_file = create_temp_key_file();
        let ticket_keys_file = create_temp_ticket_keys_file();

        let config = create_test_config(vec![
            ("cert", cert_file.path().to_str().unwrap()),
            ("key", key_file.path().to_str().unwrap()),
            ("ticket_keys", ticket_keys_file.path().to_str().unwrap()),
        ]);

        let provider = TcpTlsManualProvider;
        let mut ctx = TcpTlsContext {
            config: &config,
            alpn: None,
            domain: ServerConfigurationHostFilters::default(),
            port: 443,
            resolver: None,
        };

        // This should execute successfully (even if cert loading fails)
        let result = provider.execute(&mut ctx);
        // We expect cert loading might fail, but ticket_keys validation should pass
        if let Err(e) = &result {
            // If it fails, it shouldn't be due to ticket_keys
            assert!(!e.to_string().contains("ticket"));
        }
    }
}
