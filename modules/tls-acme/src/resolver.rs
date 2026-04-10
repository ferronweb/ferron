//! TLS resolvers for ACME-managed certificates.
//!
//! Provides:
//! - `AcmeResolver`: Resolves the certified key obtained via ACME.
//! - `TcpTlsAcmeResolver`: Wraps an `AcmeResolver` and optionally TLS-ALPN-01
//!   challenge locks to implement `TcpTlsResolver`.

use std::sync::Arc;

use ferron_tls::TcpTlsResolver;
use rustls::{
    server::{ResolvesServerCert, ServerConfig},
    sign::CertifiedKey,
};
use tokio::sync::RwLock;
use tokio_rustls::server::TlsStream;
use tokio_rustls::StartHandshake;
use vibeio::net::PollTcpStream;

use crate::challenge::ACME_TLS_ALPN_NAME;

/// An ACME resolver that resolves a single certified key.
///
/// Used as the inner resolver in `TcpTlsAcmeResolver`.
#[derive(Debug)]
pub struct AcmeResolver {
    pub(crate) certified_key_lock: Arc<RwLock<Option<Arc<CertifiedKey>>>>,
}

impl AcmeResolver {
    /// Creates a new `AcmeResolver` from a certified key lock.
    pub fn new(certified_key_lock: Arc<RwLock<Option<Arc<CertifiedKey>>>>) -> Self {
        Self { certified_key_lock }
    }
}

impl ResolvesServerCert for AcmeResolver {
    fn resolve(&self, _client_hello: rustls::server::ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        self.certified_key_lock
            .try_read()
            .ok()
            .and_then(|g| g.clone())
    }
}

/// Configuration for OCSP stapling.
#[derive(Debug, Clone, Default)]
pub struct OcspConfig {
    /// Whether OCSP stapling is enabled (default: true).
    pub enabled: bool,
}

/// Configuration for automatic ticket key rotation.
#[derive(Debug, Clone)]
pub struct TicketKeyRotationConfig {
    /// Path to the ticket key file
    pub file: String,
    /// Whether automatic rotation is enabled
    pub auto_rotate: bool,
    /// How often to rotate keys (default: 12 hours)
    pub rotation_interval: std::time::Duration,
    /// Maximum number of keys to keep (default: 3)
    pub max_keys: usize,
}

impl Default for TicketKeyRotationConfig {
    fn default() -> Self {
        Self {
            file: String::new(),
            auto_rotate: false,
            rotation_interval: std::time::Duration::from_secs(12 * 3600),
            max_keys: 3,
        }
    }
}

/// A `TcpTlsResolver` implementation for ACME-managed TLS.
///
/// Uses `LazyConfigAcceptor` to inspect the ClientHello before committing to a
/// `ServerConfig`. If the client sends the `acme-tls/1` ALPN, the resolver looks
/// up the matching challenge certificate from the shared TLS-ALPN-01 locks.
pub struct TcpTlsAcmeResolver {
    /// The main resolver for the ACME-obtained certificate.
    acme_resolver: Arc<AcmeResolver>,
    /// Optional shared list of TLS-ALPN-01 data locks.
    tls_alpn_01_resolvers: Option<Arc<RwLock<Vec<crate::challenge::TlsAlpn01DataLock>>>>,
    /// ALPN protocols to advertise (e.g. h2, http/1.1).
    alpn_protocols: Vec<Vec<u8>>,
    /// OCSP stapling configuration.
    ocsp_config: OcspConfig,
    /// OCSP service handle (if available).
    ocsp_handle: OcspHandle,
    /// Ticket key ticketer.
    ticketer: Option<Arc<dyn rustls::server::ProducesTickets>>,
}

/// OCSP service handle type alias.
#[cfg(feature = "ocsp")]
type OcspHandle = Option<ferron_ocsp::OcspServiceHandle>;
#[cfg(not(feature = "ocsp"))]
type OcspHandle = Option<()>;

impl TcpTlsAcmeResolver {
    /// Creates a new `TcpTlsAcmeResolver`.
    ///
    /// # Arguments
    /// * `certified_key_lock` - Lock for the ACME-obtained certified key.
    /// * `tls_alpn_01_resolvers` - Optional shared list of TLS-ALPN-01 data locks.
    /// * `alpn_protocols` - ALPN protocols to advertise in the TLS handshake.
    /// * `ocsp_config` - OCSP stapling configuration.
    /// * `ocsp_handle` - Optional OCSP service handle for stapling.
    /// * `ticketer` - Optional ticket key ticketer.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        certified_key_lock: Arc<RwLock<Option<Arc<CertifiedKey>>>>,
        tls_alpn_01_resolvers: Option<Arc<RwLock<Vec<crate::challenge::TlsAlpn01DataLock>>>>,
        alpn_protocols: Vec<Vec<u8>>,
        ocsp_config: OcspConfig,
        ocsp_handle: OcspHandle,
        ticketer: Option<Arc<dyn rustls::server::ProducesTickets>>,
    ) -> Self {
        let acme_resolver = Arc::new(AcmeResolver::new(certified_key_lock));

        Self {
            acme_resolver,
            tls_alpn_01_resolvers,
            alpn_protocols,
            ocsp_config,
            ocsp_handle,
            ticketer,
        }
    }

    /// Looks up a challenge certificate for TLS-ALPN-01 from the shared locks.
    fn find_challenge_cert(&self, server_name: &str) -> Option<Arc<CertifiedKey>> {
        let locks = self.tls_alpn_01_resolvers.as_ref()?.try_read().ok()?;
        for lock in locks.iter() {
            if let Some(data) = lock.try_read().ok().and_then(|g| g.clone()) {
                if data.1 == server_name {
                    return Some(data.0);
                }
            }
        }
        None
    }

    /// Builds a ServerConfig for the TLS-ALPN-01 challenge response.
    fn build_challenge_config(&self, certified_key: Arc<CertifiedKey>) -> ServerConfig {
        let provider = rustls::crypto::aws_lc_rs::default_provider();
        let mut config = ServerConfig::builder_with_provider(Arc::new(provider))
            .with_safe_default_protocol_versions()
            .expect("valid protocol versions")
            .with_no_client_auth()
            .with_cert_resolver(Arc::new(SingleCertResolver(certified_key)));
        config.alpn_protocols = vec![ACME_TLS_ALPN_NAME.to_vec()];
        config
    }

    /// Builds a ServerConfig for normal ACME certificate serving.
    fn build_normal_config(&self) -> ServerConfig {
        let provider = rustls::crypto::aws_lc_rs::default_provider();
        let mut config = ServerConfig::builder_with_provider(Arc::new(provider))
            .with_safe_default_protocol_versions()
            .expect("valid protocol versions")
            .with_no_client_auth()
            .with_cert_resolver(self.acme_resolver.clone());
        config.alpn_protocols.clone_from(&self.alpn_protocols);

        // Attach OCSP stapler if enabled
        if self.ocsp_config.enabled {
            if let Some(ref handle) = self.ocsp_handle {
                #[cfg(feature = "ocsp")]
                {
                    config.cert_resolver = Arc::new(ferron_ocsp::OcspStapler::new(
                        config.cert_resolver.clone(),
                        handle,
                    ));
                }
                let _ = handle; // suppress unused warning when ocsp feature is disabled
            }
        }

        // Attach ticket key rotator if configured
        if let Some(ref ticketer) = self.ticketer {
            config.ticketer = ticketer.clone();
        }

        config
    }
}

#[async_trait::async_trait(?Send)]
impl TcpTlsResolver for TcpTlsAcmeResolver {
    fn get_tls_config(&self) -> Arc<ServerConfig> {
        Arc::new(self.build_normal_config())
    }

    async fn handshake(
        &self,
        io: StartHandshake<PollTcpStream>,
    ) -> Result<Option<TlsStream<PollTcpStream>>, std::io::Error> {
        let client_hello = io.client_hello();

        // Check for TLS-ALPN-01 ACME challenge
        let is_acme_challenge = self.tls_alpn_01_resolvers.is_some()
            && client_hello
                .alpn()
                .into_iter()
                .flatten()
                .eq([ACME_TLS_ALPN_NAME]);

        if is_acme_challenge {
            // Look up the matching challenge certificate
            let server_name = client_hello.server_name();
            let challenge_cert = server_name.and_then(|name| self.find_challenge_cert(name));

            match challenge_cert {
                Some(cert) => {
                    let _ = io
                        .into_stream(Arc::new(self.build_challenge_config(cert)))
                        .await;
                    return Ok(None);
                }
                None => {
                    ferron_core::log_warn!("TLS-ALPN-01 challenge requested for unknown domain");
                }
            }
        }

        match io.into_stream(Arc::new(self.build_normal_config())).await {
            Ok(stream) => Ok(Some(stream)),
            Err(err) => {
                ferron_core::log_warn!("Error during TLS handshake: {err}");
                Ok(None)
            }
        }
    }
}

/// Parse OCSP stapling configuration from a TLS configuration block.
///
/// Looks for a nested `ocsp` block:
/// ```text
/// tls {
///     provider "acme"
///     ocsp {
///         enabled true
///     }
/// }
/// ```
///
/// Returns `OcspConfig { enabled: true }` if no `ocsp` block is found (enabled by default).
pub fn parse_ocsp_config(config: &ferron_core::config::ServerConfigurationBlock) -> OcspConfig {
    let Some(ocsp_directive) = config.directives.get("ocsp") else {
        return OcspConfig { enabled: true };
    };
    let Some(ocsp_entry) = ocsp_directive.first() else {
        return OcspConfig { enabled: true };
    };

    if let Some(ref ocsp_block) = ocsp_entry.children {
        let enabled = ocsp_block
            .get_value("enabled")
            .and_then(|v| v.as_boolean())
            .unwrap_or(true);
        OcspConfig { enabled }
    } else {
        // Bare `ocsp` directive (no children) — treat as enabled
        OcspConfig { enabled: true }
    }
}

/// Parse ticket key rotation configuration from a TLS configuration block.
pub fn parse_ticket_key_config(
    config: &ferron_core::config::ServerConfigurationBlock,
) -> Option<TicketKeyRotationConfig> {
    let ticket_keys_directive = config.directives.get("ticket_keys")?;
    let ticket_keys_entry = ticket_keys_directive.first()?;
    let ticket_keys_block = ticket_keys_entry.children.as_ref()?;

    // Extract file path (required)
    let file = ticket_keys_block
        .get_value("file")
        .and_then(|v| v.as_string_with_interpolations(&std::collections::HashMap::new()))?;

    // Extract auto_rotate (optional, default: false)
    let auto_rotate = ticket_keys_block
        .get_value("auto_rotate")
        .and_then(|v| v.as_boolean())
        .unwrap_or(false);

    // Extract rotation_interval (optional, default: 12h)
    let rotation_interval = ticket_keys_block
        .get_value("rotation_interval")
        .and_then(|v| {
            if let Some(si) = v.as_string_with_interpolations(&std::collections::HashMap::new()) {
                Some(
                    ferron_core::util::parse_duration(&si)
                        .unwrap_or(std::time::Duration::from_secs(12 * 3600)),
                )
            } else {
                v.as_number()
                    .map(|n| std::time::Duration::from_secs(n as u64))
            }
        })
        .unwrap_or(std::time::Duration::from_secs(12 * 3600));

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

    Some(TicketKeyRotationConfig {
        file,
        auto_rotate,
        rotation_interval,
        max_keys,
    })
}

/// Gets the OCSP service handle if OCSP stapling is enabled.
#[cfg(feature = "ocsp")]
pub fn get_ocsp_handle_if_enabled(ocsp_config: &OcspConfig) -> OcspHandle {
    if ocsp_config.enabled {
        ferron_ocsp::get_service_handle()
    } else {
        None
    }
}

/// Gets the OCSP service handle (disabled when feature is not enabled).
#[cfg(not(feature = "ocsp"))]
pub fn get_ocsp_handle_if_enabled(_ocsp_config: &OcspConfig) -> OcspHandle {
    None
}

/// Builds a ticketer from the TLS configuration block.
///
/// If a `ticket_keys` block is found, attempts to set up a `TicketKeyRotator`.
/// Falls back to the default rustls ticketer if no block is found or if setup fails.
pub fn build_ticketer(
    config: &ferron_core::config::ServerConfigurationBlock,
) -> Option<Arc<dyn rustls::server::ProducesTickets>> {
    let Some(rot_config) = parse_ticket_key_config(config) else {
        // No ticket_keys block — use rustls default ticketer
        return rustls::crypto::aws_lc_rs::Ticketer::new().ok();
    };

    if rot_config.auto_rotate {
        // Ensure key file exists (generate if missing)
        if !std::path::Path::new(&rot_config.file).exists() {
            ferron_core::log_info!(
                "Generating initial ticket keys at {} ({} keys)",
                rot_config.file,
                rot_config.max_keys
            );
            if let Err(e) = ferron_tls::tickets::generate_initial_ticket_keys(
                &rot_config.file,
                rot_config.max_keys,
            ) {
                ferron_core::log_warn!("Failed to generate initial ticket keys: {e}");
                return rustls::crypto::aws_lc_rs::Ticketer::new().ok();
            }
        } else {
            // Validate existing file
            if let Err(e) = ferron_tls::tickets::validate_ticket_keys_file(&rot_config.file) {
                ferron_core::log_warn!("Invalid ticket keys file: {e}");
                return rustls::crypto::aws_lc_rs::Ticketer::new().ok();
            }
        }

        // Load keys and create rotator
        match ferron_tls::tickets::load_ticket_keys(&rot_config.file) {
            Ok(raw_keys) => {
                ferron_core::log_info!(
                    "Loaded {} ticket keys from {} (rotation interval: {:?})",
                    raw_keys.len(),
                    rot_config.file,
                    rot_config.rotation_interval
                );

                let ticket_keys: Vec<ferron_tls::tickets::TicketKey> = raw_keys
                    .iter()
                    .map(|(name, aes, hmac)| {
                        ferron_tls::tickets::TicketKey::new(*name, *aes, *hmac)
                    })
                    .collect();

                match ferron_tls::tickets::TicketKeyRotator::new(
                    ticket_keys,
                    rot_config.rotation_interval,
                    rot_config.file.clone(),
                ) {
                    Ok(rotator) => {
                        ferron_core::log_info!(
                            "TLS session ticket key rotation enabled (interval: {:?}, max_keys: {})",
                            rot_config.rotation_interval,
                            rot_config.max_keys
                        );
                        Some(Arc::new(rotator))
                    }
                    Err(e) => {
                        ferron_core::log_warn!("Failed to create ticket key rotator: {e}");
                        rustls::crypto::aws_lc_rs::Ticketer::new().ok()
                    }
                }
            }
            Err(e) => {
                ferron_core::log_warn!("Failed to load ticket keys: {e}");
                rustls::crypto::aws_lc_rs::Ticketer::new().ok()
            }
        }
    } else {
        // Static mode: just validate and use default ticketer
        if std::path::Path::new(&rot_config.file).exists() {
            if let Err(e) = ferron_tls::tickets::validate_ticket_keys_file(&rot_config.file) {
                ferron_core::log_warn!("Invalid ticket keys file: {e}");
                return rustls::crypto::aws_lc_rs::Ticketer::new().ok();
            }
            ferron_core::log_info!(
                "TLS session ticket keys validated from {} (static mode, no rotation)",
                rot_config.file
            );
        }
        rustls::crypto::aws_lc_rs::Ticketer::new().ok()
    }
}

/// A resolver that always returns the same certified key.
#[derive(Debug)]
struct SingleCertResolver(Arc<CertifiedKey>);

impl ResolvesServerCert for SingleCertResolver {
    fn resolve(&self, _client_hello: rustls::server::ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(self.0.clone())
    }
}
