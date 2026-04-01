use ferron_core::config::ServerConfigurationBlock;
use tokio_rustls::TlsStream;

pub trait TlsResolver {
    fn handshake<Io>(&self, io: Io) -> Result<Option<TlsStream<Io>>, std::io::Error>
    where
        Io: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + 'static;
}

pub trait TlsProvider {
    fn resolver(
        &self,
        parameters: &ServerConfigurationBlock,
    ) -> Result<impl TlsResolver, Box<dyn std::error::Error>>;
}
