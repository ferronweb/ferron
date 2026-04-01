use std::sync::Arc;

use ferron_core::config::ServerConfigurationBlock;
use tokio_rustls::server::TlsStream;
use tokio_rustls::StartHandshake;
use vibeio::net::PollTcpStream;

pub struct TcpTlsContext {
    pub config: ServerConfigurationBlock,
    pub resolver: Option<Arc<dyn TcpTlsResolver>>,
}

#[async_trait::async_trait(?Send)]
pub trait TcpTlsResolver {
    async fn handshake(
        &self,
        io: StartHandshake<PollTcpStream>,
    ) -> Result<Option<TlsStream<PollTcpStream>>, std::io::Error>;
}
