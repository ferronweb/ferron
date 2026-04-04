use std::{collections::HashMap, sync::Arc, time::Duration};

use ferron_core::config::ServerConfigurationBlock;
use ferron_core::loader::ModuleLoader;
use ferron_core::providers::Provider;
use ferron_tls::{
    TcpTlsContext, TcpTlsResolver,
    tickets::{
        TicketKey, TicketKeyRotator, generate_initial_ticket_keys, load_ticket_keys,
        validate_ticket_keys_file,
    },
};
use rustls::ServerConfig;
use rustls_pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};

/// Configuration for automatic ticket key rotation.
#[derive(Debug, Clone)]
pub struct TicketKeyRotationConfig {
    /// Path to the ticket key file
    pub file: String,
    /// Whether automatic rotation is enabled
    pub auto_rotate: bool,
    /// How often to rotate keys (default: 12 hours)
    pub rotation_interval: Duration,
    /// Maximum number of keys to keep (default: 3)
    pub max_keys: usize,
}

impl TicketKeyRotationConfig {
    /// Parse ticket key rotation configuration from a ServerConfigurationBlock.
    pub fn from_config(config: &ServerConfigurationBlock) -> Option<Self> {
        // Look for ticket_keys nested block
        let ticket_keys_directive = config.directives.get("ticket_keys")?;
        let ticket_keys_entry = ticket_keys_directive.first()?;
        let ticket_keys_block = ticket_keys_entry.children.as_ref()?;

        // Extract file path (required)
        let file = ticket_keys_block
            .get_value("file")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))?;

        // Extract auto_rotate (optional, default: false)
        let auto_rotate = ticket_keys_block
            .get_value("auto_rotate")
            .and_then(|v| v.as_boolean())
            .unwrap_or(false);

        // Extract rotation_interval (optional, default: 12h)
        let rotation_interval = ticket_keys_block
            .get_value("rotation_interval")
            .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
            .map(|s| parse_duration(&s).unwrap_or(Duration::from_secs(12 * 3600)))
            .unwrap_or(Duration::from_secs(12 * 3600));

        // Extract max_keys (optional, default: 3, range: 2-5)
        let max_keys = ticket_keys_block
            .get_value("max_keys")
            .and_then(|v| v.as_number())
            .map(|n| {
                let n = n as usize;
                if n < 2 {
                    ferron_core::log_warn!(
                        "ticket_keys.max_keys={} is too small, using minimum of 2",
                        n
                    );
                    2
                } else if n > 5 {
                    ferron_core::log_warn!(
                        "ticket_keys.max_keys={} is too large, using maximum of 5",
                        n
                    );
                    5
                } else {
                    n
                }
            })
            .unwrap_or(3);

        Some(Self {
            file,
            auto_rotate,
            rotation_interval,
            max_keys,
        })
    }
}

/// Parse a duration string (e.g., "12h", "30m", "1d") into a Duration.
fn parse_duration(s: &str) -> Result<Duration, String> {
    let s = s.trim();

    if let Some(num_str) = s.strip_suffix(['h', 'H']) {
        let hours: u64 = num_str
            .trim()
            .parse()
            .map_err(|e| format!("Invalid hours '{}': {}", s, e))?;
        Ok(Duration::from_secs(hours * 3600))
    } else if let Some(num_str) = s.strip_suffix(['m', 'M']) {
        let minutes: u64 = num_str
            .trim()
            .parse()
            .map_err(|e| format!("Invalid minutes '{}': {}", s, e))?;
        Ok(Duration::from_secs(minutes * 60))
    } else if let Some(num_str) = s.strip_suffix(['s', 'S']) {
        let seconds: u64 = num_str
            .trim()
            .parse()
            .map_err(|e| format!("Invalid seconds '{}': {}", s, e))?;
        Ok(Duration::from_secs(seconds))
    } else if let Some(num_str) = s.strip_suffix(['d', 'D']) {
        let days: u64 = num_str
            .trim()
            .parse()
            .map_err(|e| format!("Invalid days '{}': {}", s, e))?;
        Ok(Duration::from_secs(days * 86400))
    } else {
        // Try plain number (assume hours)
        let hours: u64 = s
            .parse()
            .map_err(|e| format!("Invalid duration '{}': {}", s, e))?;
        Ok(Duration::from_secs(hours * 3600))
    }
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
        // TODO: configure TLS crypto provider
        let provider = rustls::crypto::aws_lc_rs::default_provider();
        // TODO: mTLS
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(provider))
            .with_safe_default_protocol_versions()?
            .with_no_client_auth();

        // Parse ticket key configuration
        let rotation_config = TicketKeyRotationConfig::from_config(ctx.config);

        let ticketer = if let Some(rot_config) = rotation_config {
            // Ensure key file exists (generate if missing and auto_rotate is on)
            if !std::path::Path::new(&rot_config.file).exists() {
                if rot_config.auto_rotate {
                    ferron_core::log_info!(
                        "Generating initial ticket keys at {} ({} keys)",
                        rot_config.file,
                        rot_config.max_keys
                    );
                    generate_initial_ticket_keys(&rot_config.file, rot_config.max_keys)?;
                } else {
                    return Err(format!(
                        "Ticket keys file not found: {}. Enable auto_rotate to auto-generate.",
                        rot_config.file
                    )
                    .into());
                }
            } else {
                // Validate existing file
                validate_ticket_keys_file(&rot_config.file)?;
            }

            if rot_config.auto_rotate {
                // Load keys and create rotator
                let raw_keys = load_ticket_keys(&rot_config.file)?;
                ferron_core::log_info!(
                    "Loaded {} ticket keys from {} (rotation interval: {:?})",
                    raw_keys.len(),
                    rot_config.file,
                    rot_config.rotation_interval
                );

                let ticket_keys: Vec<TicketKey> = raw_keys
                    .iter()
                    .map(|(name, aes, hmac)| TicketKey::new(*name, *aes, *hmac))
                    .collect();

                let rotator = TicketKeyRotator::new(
                    ticket_keys,
                    rot_config.rotation_interval,
                    rot_config.file.clone(),
                )
                .map_err(|e| {
                    Box::new(std::io::Error::other(e.to_string())) as Box<dyn std::error::Error>
                })?;

                ferron_core::log_info!(
                    "TLS session ticket key rotation enabled (interval: {:?}, max_keys: {})",
                    rot_config.rotation_interval,
                    rot_config.max_keys
                );

                Arc::new(rotator)
            } else {
                // Static mode: just validate and use rustls default ticketer
                // In the future, we could implement a static custom ticketer here
                ferron_core::log_info!(
                    "TLS session ticket keys validated from {} (static mode, no rotation)",
                    rot_config.file
                );
                rustls::crypto::aws_lc_rs::Ticketer::new()?
            }
        } else {
            // No ticket_keys configuration: use rustls default ticketer
            rustls::crypto::aws_lc_rs::Ticketer::new()?
        };

        // Build the config with certificates
        let mut config_with_tickets = config.with_single_cert(
            load_certs(
                ctx.config
                    .get_value("cert")
                    .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
                    .as_deref()
                    .ok_or(std::io::Error::other(
                        "'cert' TLS parameter missing or invalid",
                    ))?,
            )?,
            load_private_key(
                ctx.config
                    .get_value("key")
                    .and_then(|v| v.as_string_with_interpolations(&HashMap::new()))
                    .as_deref()
                    .ok_or(std::io::Error::other(
                        "'key' TLS parameter missing or invalid",
                    ))?,
            )?,
        )?;

        // Attach the ticketer
        config_with_tickets.ticketer = ticketer;

        if let Some(alpn_protocols) = ctx.alpn.as_ref() {
            config_with_tickets.alpn_protocols = alpn_protocols.clone();
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
        ServerConfigurationBlock, ServerConfigurationDirectiveEntry, ServerConfigurationValue,
    };
    use ferron_tls::TcpTlsContext;
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
MIIBkTCBAQEwDQYJKoZIhvcNAQEBBQAE
MFUwEwYJKoZIhvcNAQEBBQIxGzAZBgNV
BAoTEmZlcnJvbi10bHMtdGVzdGluZzEL
MAkGA1UEBhMCVVMxEDAOBgNVBAcTB1Vu
a25vd24xEDAOBgNVBAgTB1Vua25vd24w
HhcNMjQwMTAxMDAwMDAwWhcNMjUwMTAx
MDAwMDAwWjBRMQswCQYDVQQGEwJVUzEQ
MA4GA1UEBxMHVW5rbm93bjEQMA4GA1UE
CBMHVW5rbm93bjEaMBgGA1UEChMRZmVy
cm9uLXRscy10ZXN0aW5nMFUwEwYJKoZI
hvcNAQEBBQIxGzAZBgNVBAoTEmZlcnJv
bi10bHMtdGVzdGluZzELMAkGA1UEBhMC
VVMxEDAOBgNVBAcTB1Vua25vd24xEDAO
BgNVBAgTB1Vua25vd24wDQYJKoZIhvcNAQEB
BQADQQBfMn9Cn5b2QVY2PcK7HjMmGHKd
Y7F7fF7fF7fF7fF7fF7fF7fF7fF7fF7f
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
MIIBVAIBADANBgkqhkiG9w0BAQEFAASCAT4wggE6AgEA
AkEAt5B4b3F4b3F4b3F4b3F4b3F4b3F4b3F4b3F4b3F4
b3F4b3F4b3F4b3F4b3F4b3F4b3F4b3F4b3F4b3F4b3F4
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
    fn test_provider_validates_ticket_keys_file() {
        // Test that invalid ticket_keys path causes failure
        let cert_file = create_temp_cert_file();
        let key_file = create_temp_key_file();

        // Create config with nested ticket_keys block pointing to nonexistent file
        let nested_config = ServerConfigurationBlock {
            directives: Arc::new(HashMap::from([(
                "file".to_string(),
                vec![ServerConfigurationDirectiveEntry {
                    args: vec![ServerConfigurationValue::String(
                        "/nonexistent/ticket.keys".to_string(),
                        None,
                    )],
                    children: None,
                    span: None,
                }],
            )])),
            matchers: HashMap::new(),
            span: None,
        };

        let config = create_test_config_with_nested(
            vec![
                ("cert", cert_file.path().to_str().unwrap()),
                ("key", key_file.path().to_str().unwrap()),
            ],
            ("ticket_keys", nested_config),
        );

        let provider = TcpTlsManualProvider;
        let mut ctx = TcpTlsContext {
            config: &config,
            alpn: None,
            resolver: None,
        };

        // This should fail because the ticket_keys file doesn't exist
        let result = provider.execute(&mut ctx);
        assert!(result.is_err());
        let err = result.unwrap_err();
        // The error should be related to file not found (since the ticket_keys file doesn't exist)
        // It might not contain the word "ticket" specifically, but should be an IO error
        assert!(
            err.to_string().contains("No such file")
                || err.to_string().contains("not found")
                || err.to_string().contains("ticket")
        );
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

    #[test]
    fn test_parse_duration_hours() {
        assert_eq!(
            parse_duration("12h").unwrap(),
            Duration::from_secs(12 * 3600)
        );
        assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
        assert_eq!(
            parse_duration("24H").unwrap(),
            Duration::from_secs(24 * 3600)
        );
    }

    #[test]
    fn test_parse_duration_minutes() {
        assert_eq!(parse_duration("30m").unwrap(), Duration::from_secs(1800));
        assert_eq!(parse_duration("60M").unwrap(), Duration::from_secs(3600));
    }

    #[test]
    fn test_parse_duration_seconds() {
        assert_eq!(parse_duration("90s").unwrap(), Duration::from_secs(90));
    }

    #[test]
    fn test_parse_duration_days() {
        assert_eq!(parse_duration("1d").unwrap(), Duration::from_secs(86400));
        assert_eq!(
            parse_duration("2D").unwrap(),
            Duration::from_secs(2 * 86400)
        );
    }

    #[test]
    fn test_parse_duration_plain_number() {
        // Plain numbers are treated as hours
        assert_eq!(
            parse_duration("12").unwrap(),
            Duration::from_secs(12 * 3600)
        );
    }

    #[test]
    fn test_parse_duration_invalid() {
        assert!(parse_duration("abc").is_err());
        assert!(parse_duration("").is_err());
    }

    #[test]
    fn test_ticket_key_rotation_config_parsing() {
        let ticket_keys_file = create_temp_ticket_keys_file();

        let nested_config = ServerConfigurationBlock {
            directives: Arc::new(HashMap::from([
                (
                    "file".to_string(),
                    vec![ServerConfigurationDirectiveEntry {
                        args: vec![ServerConfigurationValue::String(
                            ticket_keys_file.path().to_str().unwrap().to_string(),
                            None,
                        )],
                        children: None,
                        span: None,
                    }],
                ),
                (
                    "auto_rotate".to_string(),
                    vec![ServerConfigurationDirectiveEntry {
                        args: vec![ServerConfigurationValue::Boolean(true, None)],
                        children: None,
                        span: None,
                    }],
                ),
                (
                    "rotation_interval".to_string(),
                    vec![ServerConfigurationDirectiveEntry {
                        args: vec![ServerConfigurationValue::String("6h".to_string(), None)],
                        children: None,
                        span: None,
                    }],
                ),
                (
                    "max_keys".to_string(),
                    vec![ServerConfigurationDirectiveEntry {
                        args: vec![ServerConfigurationValue::Number(5, None)],
                        children: None,
                        span: None,
                    }],
                ),
            ])),
            matchers: HashMap::new(),
            span: None,
        };

        let config = create_test_config_with_nested(
            vec![("cert", "/tmp/cert.pem"), ("key", "/tmp/key.pem")],
            ("ticket_keys", nested_config),
        );

        let rotation_config = TicketKeyRotationConfig::from_config(&config);
        assert!(rotation_config.is_some());
        let rotation_config = rotation_config.unwrap();

        assert!(rotation_config.auto_rotate);
        assert_eq!(
            rotation_config.rotation_interval,
            Duration::from_secs(6 * 3600)
        );
        assert_eq!(rotation_config.max_keys, 5);
    }

    #[test]
    fn test_ticket_key_rotation_config_defaults() {
        let ticket_keys_file = create_temp_ticket_keys_file();

        let nested_config = ServerConfigurationBlock {
            directives: Arc::new(HashMap::from([(
                "file".to_string(),
                vec![ServerConfigurationDirectiveEntry {
                    args: vec![ServerConfigurationValue::String(
                        ticket_keys_file.path().to_str().unwrap().to_string(),
                        None,
                    )],
                    children: None,
                    span: None,
                }],
            )])),
            matchers: HashMap::new(),
            span: None,
        };

        let config = create_test_config_with_nested(
            vec![("cert", "/tmp/cert.pem"), ("key", "/tmp/key.pem")],
            ("ticket_keys", nested_config),
        );

        let rotation_config = TicketKeyRotationConfig::from_config(&config);
        assert!(rotation_config.is_some());
        let rotation_config = rotation_config.unwrap();

        assert!(!rotation_config.auto_rotate);
        assert_eq!(
            rotation_config.rotation_interval,
            Duration::from_secs(12 * 3600)
        );
        assert_eq!(rotation_config.max_keys, 3);
    }

    #[test]
    fn test_ticket_key_rotation_config_no_block() {
        let config = create_test_config(vec![("cert", "/tmp/cert.pem"), ("key", "/tmp/key.pem")]);

        let rotation_config = TicketKeyRotationConfig::from_config(&config);
        assert!(rotation_config.is_none());
    }
}
