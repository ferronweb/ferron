//! TLS types shared by all TLS provider modules.
//!
//! This crate defines the traits and configuration types that TLS providers
//! (manual, ACME, HTTP, local) implement to deliver TLS termination.
//!
//! # Key types
//!
//! - [`TlsContext`] — configuration context passed to TLS providers.
//! - [`TlsResolver`] — trait for resolving TLS configuration at handshake time.
//! - [`TlsInnerSocket`] — the underlying TCP or Unix socket before TLS wrapping.
//!
//! # For module authors
//!
//! Implement [`TlsResolver`] to provide TLS certificates and configuration
//! for a specific host. The resolver is called during the TLS handshake to
//! obtain the `rustls::ServerConfig`.

use std::sync::Arc;

use ferron_core::config::{ServerConfigurationBlock, ServerConfigurationHostFilters};
use rustls::ServerConfig;
use tokio_rustls::server::TlsStream;
use tokio_rustls::StartHandshake;

/// Reusable TLS builder utilities (crypto providers, ticketers, client verifiers).
pub mod builder;
/// Shared TLS configuration types and parsing.
pub mod config;
/// TLS directive registration for the configuration system.
pub mod directives;
/// Unified TLS observability helpers shared by every TLS provider.
#[cfg(feature = "observability")]
pub mod observability;
/// TLS session ticket key management.
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

/// Configuration context passed to TLS provider implementations.
///
/// A TLS provider reads the configuration block, ALPN protocols, host
/// filters, and port to produce a [`TlsResolver`] that rustls calls
/// during each TLS handshake.
pub struct TlsContext<'a> {
    /// The server configuration block for the TLS provider.
    pub config: &'a ServerConfigurationBlock,
    /// ALPN protocol negotiation values (e.g. `["h2", "http/1.1"]`).
    pub alpn: Option<Vec<Vec<u8>>>,
    /// Host filter rules for this TLS configuration.
    pub domain: ServerConfigurationHostFilters,
    /// The port this TLS configuration applies to.
    pub port: u16,
    /// The resolver that provides TLS configuration at handshake time.
    pub resolver: Option<Arc<dyn TlsResolver>>,
}

/// Resolves TLS configuration for incoming connections.
///
/// Implement this trait to provide per-host or per-connection TLS
/// configuration. The resolver is called during the TLS handshake to
/// obtain the `rustls::ServerConfig` and optional TLS connection parameters.
#[async_trait::async_trait(?Send)]
pub trait TlsResolver: Send + Sync {
    /// Complete a TLS handshake on the given connection.
    ///
    /// The default implementation calls [`get_tls_config`](TlsResolver::get_tls_config)
    /// and completes the handshake. Override this to customize handshake behavior
    /// (e.g. SNI-based certificate selection, logging).
    #[inline]
    async fn handshake(
        &self,
        io: StartHandshake<TlsInnerSocket>,
    ) -> Result<Option<TlsStream<TlsInnerSocket>>, std::io::Error> {
        Ok(Some(io.into_stream(self.get_tls_config()).await?))
    }

    /// Get the `rustls::ServerConfig` for this connection.
    fn get_tls_config(&self) -> Arc<ServerConfig>;

    /// Get a background error message, if any.
    ///
    /// Returns a diagnostic message when the resolver encounters a
    /// non-fatal error (e.g. certificate reload failure).
    #[inline]
    fn get_tls_background_error(&self) -> Option<String> {
        None
    }
}

/// The underlying transport socket before TLS wrapping.
///
/// Supports both TCP and Unix domain sockets. This is passed to the
/// [`TlsResolver::handshake`] method to complete the TLS handshake.
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
