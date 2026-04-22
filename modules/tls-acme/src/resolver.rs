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

use crate::{challenge::ACME_TLS_ALPN_NAME, config::SniResolverLock, on_demand::OnDemandRequest};

/// The inner resolver for `AcmeResolver`.
///
/// Used to store either an eager-loaded certified key or an on-demand resolver.
#[derive(Debug)]
pub enum AcmeResolverInner {
    Eager(Arc<RwLock<Option<Arc<CertifiedKey>>>>),
    OnDemand(SniResolverLock),
}

/// An ACME resolver that resolves a single certified key.
///
/// Used as the inner resolver in `TcpTlsAcmeResolver`.
#[derive(Debug)]
pub struct AcmeResolver {
    pub(crate) certified_key_lock: AcmeResolverInner,
    on_demand_tx: Option<(async_channel::Sender<OnDemandRequest>, u16)>,
}

impl AcmeResolver {
    /// Creates a new `AcmeResolver` from a certified key lock.
    pub fn new(
        certified_key_lock: AcmeResolverInner,
        on_demand_tx: Option<(async_channel::Sender<OnDemandRequest>, u16)>,
    ) -> Self {
        Self {
            certified_key_lock,
            on_demand_tx,
        }
    }
}

impl ResolvesServerCert for AcmeResolver {
    fn resolve(&self, client_hello: rustls::server::ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        let server_name_string = client_hello.server_name().map(String::from);
        let certified_key_option = match &self.certified_key_lock {
            AcmeResolverInner::Eager(i) => i.try_read().ok().and_then(|g| g.clone()),
            AcmeResolverInner::OnDemand(i) => i
                .try_read()
                .ok()
                .and_then(|g| g.get(client_hello.server_name().unwrap_or("")).cloned())
                .and_then(move |r| r.resolve(client_hello)),
        };

        if certified_key_option.is_none() {
            if let Some((tx, port)) = &self.on_demand_tx {
                if let Some(server_name) = server_name_string {
                    // On-demand TLS channel would be unbounded...
                    let _ = tx.try_send((server_name, *port));
                }
            }
        }

        certified_key_option
    }
}

use ferron_tls::config::OcspConfig;

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
        certified_key_lock: AcmeResolverInner,
        tls_alpn_01_resolvers: Option<Arc<RwLock<Vec<crate::challenge::TlsAlpn01DataLock>>>>,
        alpn_protocols: Vec<Vec<u8>>,
        ocsp_config: OcspConfig,
        ocsp_handle: OcspHandle,
        ticketer: Option<Arc<dyn rustls::server::ProducesTickets>>,
        on_demand_tx: Option<(async_channel::Sender<OnDemandRequest>, u16)>,
    ) -> Self {
        let acme_resolver = Arc::new(AcmeResolver::new(certified_key_lock, on_demand_tx));

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

/// A resolver that always returns the same certified key.
#[derive(Debug)]
struct SingleCertResolver(Arc<CertifiedKey>);

impl ResolvesServerCert for SingleCertResolver {
    fn resolve(&self, _client_hello: rustls::server::ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(self.0.clone())
    }
}
