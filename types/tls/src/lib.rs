use std::sync::Arc;

use ferron_core::config::{ServerConfigurationBlock, ServerConfigurationHostFilters};
use rustls::ServerConfig;
use tokio_rustls::server::TlsStream;
use tokio_rustls::StartHandshake;
use vibeio::net::PollTcpStream;

pub mod builder;
pub mod config;
#[cfg(feature = "observability")]
pub mod observability;
pub mod tickets;

/// TLS connection parameters extracted after a successful handshake.
///
/// Carries the negotiated protocol version and cipher suite so they can be
/// propagated to trace spans and per-host metrics without requiring the TLS
/// stream to be kept alive.
#[derive(Clone, Debug)]
pub struct TlsConnectionParams {
    /// Negotiated protocol version, e.g. `"TLSv1.3"` or `"TLSv1.2"`.
    pub protocol_version: String,
    /// Negotiated cipher suite IANA name, e.g. `"TLS_AES_256_GCM_SHA384"`.
    pub cipher_suite: String,
}

pub struct TcpTlsContext<'a> {
    pub config: &'a ServerConfigurationBlock,
    pub alpn: Option<Vec<Vec<u8>>>,
    pub domain: ServerConfigurationHostFilters,
    pub port: u16,
    pub resolver: Option<Arc<dyn TcpTlsResolver>>,
}

#[async_trait::async_trait(?Send)]
pub trait TcpTlsResolver: Send + Sync {
    #[inline]
    async fn handshake(
        &self,
        io: StartHandshake<PollTcpStream>,
    ) -> Result<Option<TlsStream<PollTcpStream>>, std::io::Error> {
        Ok(Some(io.into_stream(self.get_tls_config()).await?))
    }

    fn get_tls_config(&self) -> Arc<ServerConfig>;

    #[inline]
    fn get_tls_background_error(&self) -> Option<String> {
        None
    }
}
