use std::sync::Arc;

use ferron_core::config::ServerConfigurationBlock;
use rustls::ServerConfig;
use tokio_rustls::server::TlsStream;
use tokio_rustls::StartHandshake;
use vibeio::net::PollTcpStream;

pub mod tickets;

pub struct TcpTlsContext<'a> {
    pub config: &'a ServerConfigurationBlock,
    pub alpn: Option<Vec<Vec<u8>>>,
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
}
