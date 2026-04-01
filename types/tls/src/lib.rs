use std::sync::Arc;

use ferron_core::config::ServerConfigurationBlock;
use tokio_rustls::TlsStream;
use vibeio::net::PollTcpStream;

pub struct TcpTlsContext {
    pub config: ServerConfigurationBlock,
    pub resolver: Option<Arc<dyn TcpTlsResolver>>,
}

pub trait TcpTlsResolver {
    fn handshake(
        &self,
        io: PollTcpStream,
    ) -> Result<Option<TlsStream<PollTcpStream>>, std::io::Error>;
}
