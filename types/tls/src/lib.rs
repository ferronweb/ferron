use std::sync::Arc;

use ferron_core::config::{ServerConfigurationBlock, ServerConfigurationHostFilters};
use rustls::ServerConfig;
use tokio_rustls::server::TlsStream;
use tokio_rustls::StartHandshake;

pub mod builder;
pub mod config;
pub mod directives;
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

pub struct TlsContext<'a> {
    pub config: &'a ServerConfigurationBlock,
    pub alpn: Option<Vec<Vec<u8>>>,
    pub domain: ServerConfigurationHostFilters,
    pub port: u16,
    pub resolver: Option<Arc<dyn TlsResolver>>,
}

#[async_trait::async_trait(?Send)]
pub trait TlsResolver: Send + Sync {
    #[inline]
    async fn handshake(
        &self,
        io: StartHandshake<TlsInnerSocket>,
    ) -> Result<Option<TlsStream<TlsInnerSocket>>, std::io::Error> {
        Ok(Some(io.into_stream(self.get_tls_config()).await?))
    }

    fn get_tls_config(&self) -> Arc<ServerConfig>;

    #[inline]
    fn get_tls_background_error(&self) -> Option<String> {
        None
    }
}

pub enum TlsInnerSocket {
    Tcp(zincio::net::PollTcpStream),
    #[cfg(unix)]
    Unix(zincio::net::PollUnixStream),
}

impl tokio::io::AsyncRead for TlsInnerSocket {
    #[inline]
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.get_mut() {
            Self::Tcp(s) => std::pin::Pin::new(s).poll_read(cx, buf),
            #[cfg(unix)]
            Self::Unix(s) => std::pin::Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl tokio::io::AsyncWrite for TlsInnerSocket {
    #[inline]
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        match self.get_mut() {
            Self::Tcp(s) => std::pin::Pin::new(s).poll_write(cx, buf),
            #[cfg(unix)]
            Self::Unix(s) => std::pin::Pin::new(s).poll_write(cx, buf),
        }
    }

    #[inline]
    fn poll_write_vectored(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> std::task::Poll<std::io::Result<usize>> {
        match self.get_mut() {
            Self::Tcp(s) => std::pin::Pin::new(s).poll_write_vectored(cx, bufs),
            #[cfg(unix)]
            Self::Unix(s) => std::pin::Pin::new(s).poll_write_vectored(cx, bufs),
        }
    }

    #[inline]
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.get_mut() {
            Self::Tcp(s) => std::pin::Pin::new(s).poll_flush(cx),
            #[cfg(unix)]
            Self::Unix(s) => std::pin::Pin::new(s).poll_flush(cx),
        }
    }

    #[inline]
    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.get_mut() {
            Self::Tcp(s) => std::pin::Pin::new(s).poll_shutdown(cx),
            #[cfg(unix)]
            Self::Unix(s) => std::pin::Pin::new(s).poll_shutdown(cx),
        }
    }

    #[inline]
    fn is_write_vectored(&self) -> bool {
        match self {
            Self::Tcp(s) => s.is_write_vectored(),
            #[cfg(unix)]
            Self::Unix(s) => s.is_write_vectored(),
        }
    }
}

impl From<zincio::net::PollTcpStream> for TlsInnerSocket {
    #[inline]
    fn from(stream: zincio::net::PollTcpStream) -> Self {
        Self::Tcp(stream)
    }
}

#[cfg(unix)]
impl From<zincio::net::PollUnixStream> for TlsInnerSocket {
    #[inline]
    fn from(stream: zincio::net::PollUnixStream) -> Self {
        Self::Unix(stream)
    }
}

#[cfg(unix)]
impl std::os::fd::AsRawFd for TlsInnerSocket {
    #[inline]
    fn as_raw_fd(&self) -> std::os::fd::RawFd {
        match self {
            Self::Tcp(s) => s.as_raw_fd(),
            Self::Unix(s) => s.as_raw_fd(),
        }
    }
}

#[cfg(windows)]
impl std::os::windows::io::AsRawSocket for TlsInnerSocket {
    #[inline]
    fn as_raw_socket(&self) -> std::os::windows::io::RawSocket {
        match self {
            Self::Tcp(s) => s.as_raw_socket(),
        }
    }
}
