//! HTTP client and connection pooling for forwarded authentication.

use std::collections::HashMap;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use hyper::body::Incoming;
use hyper::{Request, Response, Uri};
use rustls::client::danger::ServerCertVerifier;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::ClientConfig;
use tokio::sync::RwLock;
use vibeio_hyper::VibeioIo;

use crate::{
    ConnpoolItem, ConnpoolItemInner, ConnpoolKey, ProxyBody, DEFAULT_KEEPALIVE_IDLE_TIMEOUT,
};

enum SendRequestInner {
    Http1(hyper::client::conn::http1::SendRequest<ProxyBody>),
    Http2(hyper::client::conn::http2::SendRequest<ProxyBody>),
}

impl SendRequestInner {
    fn is_closed(&self) -> bool {
        match self {
            SendRequestInner::Http1(s) => s.is_closed(),
            SendRequestInner::Http2(s) => s.is_closed(),
        }
    }

    #[allow(dead_code)]
    fn is_ready(&self) -> bool {
        match self {
            SendRequestInner::Http1(s) => s.is_ready(),
            SendRequestInner::Http2(s) => s.is_ready(),
        }
    }

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), hyper::Error>> {
        match self {
            SendRequestInner::Http1(s) => s.poll_ready(cx),
            SendRequestInner::Http2(s) => s.poll_ready(cx),
        }
    }
}

/// A pooled HTTP send request.
pub struct SendRequestWrapper {
    inner: Option<SendRequestInner>,
    last_used: std::time::Instant,
}

impl SendRequestWrapper {
    pub fn http1(inner: hyper::client::conn::http1::SendRequest<ProxyBody>) -> Self {
        Self {
            inner: Some(SendRequestInner::Http1(inner)),
            last_used: std::time::Instant::now(),
        }
    }

    pub fn http2(inner: hyper::client::conn::http2::SendRequest<ProxyBody>) -> Self {
        Self {
            inner: Some(SendRequestInner::Http2(inner)),
            last_used: std::time::Instant::now(),
        }
    }

    /// Check if the connection is closed.
    pub fn is_closed(&self) -> bool {
        match &self.inner {
            Some(inner) => inner.is_closed(),
            None => true,
        }
    }

    /// Check readiness of the underlying connection.
    ///
    /// Returns `(is_ready, should_keep_in_pool)`:
    /// - `(true, true)` — ready, caller should use `take_inner()` to extract
    /// - `(false, true)` — not ready yet, keep in pool (connection is alive)
    /// - `(_, false)` — dead/stale, discard
    pub fn check_ready(&mut self, timeout: Option<Duration>) -> (bool, bool) {
        let Some(ref inner) = self.inner else {
            return (false, false);
        };
        let closed = match inner {
            SendRequestInner::Http1(s) => s.is_closed(),
            SendRequestInner::Http2(s) => s.is_closed(),
        };
        let ready = match inner {
            SendRequestInner::Http1(s) => s.is_ready(),
            SendRequestInner::Http2(s) => s.is_ready(),
        };
        if closed {
            return (false, false);
        }
        if ready {
            if let Some(t) = timeout {
                if self.last_used.elapsed() > t {
                    return (false, false);
                }
            }
            return (true, true);
        }
        self.last_used = std::time::Instant::now();
        (false, true)
    }

    /// Wait until the connection becomes ready, closed, or the idle timeout elapses.
    ///
    /// Returns `true` if the connection is now ready, `false` if closed/timed out.
    pub async fn wait_ready(&mut self, timeout: Option<Duration>) -> bool {
        let deadline = timeout.map(|t| std::time::Instant::now() + t);
        std::future::poll_fn(|cx| match &mut self.inner {
            Some(i) => match i.poll_ready(cx) {
                Poll::Ready(Ok(_)) => {
                    if let Some(dl) = deadline {
                        if self.last_used.elapsed()
                            > dl - std::time::Instant::now() + self.last_used.elapsed()
                        {
                            // Check actual timeout since creation
                        }
                    }
                    Poll::Ready(true)
                }
                Poll::Ready(Err(_)) => Poll::Ready(false),
                Poll::Pending => {
                    if let Some(dl) = deadline {
                        if std::time::Instant::now() >= dl {
                            return Poll::Ready(false);
                        }
                    }
                    Poll::Pending
                }
            },
            None => Poll::Ready(false),
        })
        .await
    }

    /// Send an HTTP request and receive the response.
    pub async fn send_request(
        &mut self,
        req: Request<ProxyBody>,
    ) -> Result<Response<Incoming>, Box<dyn std::error::Error + Send + Sync>> {
        self.last_used = std::time::Instant::now();
        match self.inner.take() {
            Some(SendRequestInner::Http1(mut inner)) => {
                let resp = inner.send_request(req).await?;
                self.inner = Some(SendRequestInner::Http1(inner));
                Ok(resp)
            }
            Some(SendRequestInner::Http2(mut inner)) => {
                let resp = inner.send_request(req).await?;
                self.inner = Some(SendRequestInner::Http2(inner));
                Ok(resp)
            }
            None => Err("send_request wrapper empty".into()),
        }
    }
}

/// HTTP/1.x handshake using vibeio executor.
pub async fn http1_handshake<I>(
    io: I,
) -> Result<SendRequestWrapper, Box<dyn std::error::Error + Send + Sync>>
where
    I: hyper::rt::Read + hyper::rt::Write + Unpin + 'static,
{
    let (sender, conn) = hyper::client::conn::http1::handshake(io).await?;
    vibeio::spawn(async move {
        let _ = conn.await;
    });
    Ok(SendRequestWrapper::http1(sender))
}

/// HTTP/2 handshake using vibeio executor.
pub async fn http2_handshake<I>(
    io: I,
) -> Result<SendRequestWrapper, Box<dyn std::error::Error + Send + Sync>>
where
    I: hyper::rt::Read + hyper::rt::Write + Unpin + 'static,
{
    let executor = vibeio_hyper::VibeioExecutor;
    let (sender, conn) = hyper::client::conn::http2::handshake(executor, io).await?;
    vibeio::spawn(async move {
        let _ = conn.await;
    });
    Ok(SendRequestWrapper::http2(sender))
}

/// Establish a new connection to the authentication backend.
pub async fn establish_connection(
    url: &str,
    no_verification: bool,
    tls_config: Arc<ClientConfig>,
) -> Result<ConnpoolItemInner, Box<dyn std::error::Error + Send + Sync>> {
    let uri: Uri = url.parse()?;

    // For HTTPS, use TLS connector
    if uri.scheme_str() == Some("https") {
        let hostname = uri
            .host()
            .ok_or_else(|| anyhow::anyhow!("missing host"))?
            .to_owned();
        let port = uri.port_u16().unwrap_or(443);
        let addr = format!("{}:{}", hostname, port);
        let stream = vibeio::net::TcpStream::connect(addr).await?.into_poll()?;

        let tls_config = if no_verification {
            // Create config without certificate verification
            Arc::new(
                ClientConfig::builder()
                    .dangerous()
                    .with_custom_certificate_verifier(Arc::new(NoServerVerifier))
                    .with_no_client_auth(),
            )
        } else {
            // Use the default config with verification
            tls_config
        };
        let tls_stream = tokio_rustls::TlsConnector::from(tls_config)
            .with_alpn(vec![b"http/1.1".to_vec(), b"h2".to_vec()])
            .connect(hostname.try_into()?, stream)
            .await?;

        let wrapper = if tls_stream.get_ref().1.alpn_protocol() == Some(b"h2") {
            http2_handshake(VibeioIo::new(tls_stream)).await?
        } else {
            http1_handshake(VibeioIo::new(tls_stream)).await?
        };

        Ok(ConnpoolItemInner {
            client: wrapper,
            is_unix: false,
        })
    } else {
        // For HTTP, connect directly
        let host = uri.host().ok_or("Missing host in URI")?;
        let port = uri.port_u16().unwrap_or(80);
        let addr = format!("{}:{}", host, port);
        let stream = vibeio::net::TcpStream::connect(addr).await?.into_poll()?;

        let wrapper = http1_handshake(VibeioIo::new(stream)).await?;

        Ok(ConnpoolItemInner {
            client: wrapper,
            is_unix: false,
        })
    }
}

/// Shared state for the forwarded authentication client.
pub struct ForwardedAuthClient {
    /// Connection pool
    pool: Arc<connpool::Pool<ConnpoolKey, ConnpoolItemInner>>,
    /// Global connection limit
    global_limit: usize,
    /// TLS client config
    tls_config: Arc<ClientConfig>,
    /// Keep-alive idle timeout
    idle_timeout: Option<Duration>,
    /// Per-upstream local limits
    local_limits: Arc<RwLock<HashMap<String, usize>>>,
}

impl ForwardedAuthClient {
    /// Create a new forwarded authentication client.
    pub fn new(global_limit: usize) -> Self {
        let mut root_store = rustls::RootCertStore::empty();
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let tls_config = Arc::new(
            ClientConfig::builder()
                .with_root_certificates(root_store)
                .with_no_client_auth(),
        );

        Self {
            pool: Arc::new(connpool::Pool::new(global_limit)),
            global_limit,
            tls_config,
            idle_timeout: Some(Duration::from_millis(DEFAULT_KEEPALIVE_IDLE_TIMEOUT)),
            local_limits: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Establish a new connection to the authentication backend.
    pub async fn establish_connection(
        &self,
        key: &ConnpoolKey,
        no_verification: bool,
    ) -> Result<ConnpoolItemInner, Box<dyn std::error::Error + Send + Sync>> {
        let url = &key.url;
        let unix_socket = &key.unix_socket;

        // Handle Unix socket connections
        if let Some(unix_path) = unix_socket {
            #[cfg(not(unix))]
            {
                let _ = unix_path; // Discard the variable to avoid unused variable warning
                Err("Unix sockets are not supported on this platform".into())
            }

            #[cfg(unix)]
            {
                // For Unix sockets, use vibeio's UnixStream directly
                let stream = vibeio::net::UnixStream::connect(unix_path)
                    .await?
                    .into_poll()?;

                let url: Uri = url.parse()?;

                // For HTTPS over Unix socket, use TLS connector
                if url.scheme_str() == Some("https") {
                    let hostname = url
                        .host()
                        .ok_or_else(|| anyhow::anyhow!("missing host"))?
                        .to_owned();
                    let tls_config = if no_verification {
                        // Create config without certificate verification
                        Arc::new(
                            ClientConfig::builder()
                                .dangerous()
                                .with_custom_certificate_verifier(Arc::new(NoServerVerifier))
                                .with_no_client_auth(),
                        )
                    } else {
                        // Use the default config with verification
                        self.tls_config.clone()
                    };
                    let tls_stream = tokio_rustls::TlsConnector::from(tls_config)
                        .with_alpn(vec![b"http/1.1".to_vec(), b"h2".to_vec()])
                        .connect(hostname.try_into()?, stream)
                        .await?;

                    let wrapper = if tls_stream.get_ref().1.alpn_protocol() == Some(b"h2") {
                        http2_handshake(VibeioIo::new(tls_stream)).await?
                    } else {
                        http1_handshake(VibeioIo::new(tls_stream)).await?
                    };

                    Ok(ConnpoolItemInner {
                        client: wrapper,
                        is_unix: true,
                    })
                } else {
                    // For HTTP over Unix socket, connect directly
                    let wrapper = http1_handshake(VibeioIo::new(stream)).await?;

                    Ok(ConnpoolItemInner {
                        client: wrapper,
                        is_unix: true,
                    })
                }
            }
        } else {
            // Handle TCP connections
            establish_connection(url, no_verification, self.tls_config.clone()).await
        }
    }

    /// Get or create a connection from the pool.
    pub async fn get_connection(
        &self,
        key: &ConnpoolKey,
        no_verification: bool,
        local_limit: Option<usize>,
    ) -> Result<ConnpoolItem, Box<dyn std::error::Error + Send + Sync>> {
        // Try to get from pool first
        let mut item = self
            .pool
            .pull_with_wait_local_limit(key.clone(), local_limit)
            .await;
        if let Some(inner) = item.inner_mut() {
            // Check if connection is still alive
            if inner.client.wait_ready(self.idle_timeout).await {
                return Ok(item);
            }
        }

        // Establish new connection
        let established = self.establish_connection(key, no_verification).await?;

        // Fill the item
        *item.inner_mut() = Some(established);

        Ok(item)
    }

    /// Return a connection to the pool.
    pub fn return_connection(&self, _key: ConnpoolKey, _item: ConnpoolItem) {
        // Item is returned to the pool on drop.
    }

    /// Update global connection limit.
    pub fn update_global_limit(&mut self, new_limit: usize) {
        self.global_limit = new_limit;
    }

    /// Set local connection limit for a specific upstream.
    pub async fn set_local_limit(&self, upstream_url: &str, limit: usize) {
        let mut limits = self.local_limits.write().await;
        if let std::collections::hash_map::Entry::Vacant(e) = limits.entry(upstream_url.to_string())
        {
            e.insert(self.pool.set_local_limit(limit));
        }
    }

    /// Get local connection limit for a specific upstream.
    pub async fn get_local_limit(&self, upstream_url: &str) -> Option<usize> {
        let limits = self.local_limits.read().await;
        limits.get(upstream_url).copied()
    }
}

impl Clone for ForwardedAuthClient {
    fn clone(&self) -> Self {
        Self {
            pool: self.pool.clone(),
            global_limit: self.global_limit,
            tls_config: self.tls_config.clone(),
            idle_timeout: self.idle_timeout,
            local_limits: self.local_limits.clone(),
        }
    }
}

/// A certificate verifier that doesn't verify anything (for no_verification mode).
#[derive(Debug)]
struct NoServerVerifier;

impl ServerCertVerifier for NoServerVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
            rustls::SignatureScheme::ED25519,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
        ]
    }
}
