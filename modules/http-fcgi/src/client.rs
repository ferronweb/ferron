//! HTTP client and connection pooling for FastCGI.

use std::collections::HashMap;
use std::sync::Arc;

use bytes::Bytes;
use cegla_fcgi::client::SendRequest;
use http_body_util::combinators::UnsyncBoxBody;
use tokio::sync::RwLock;

use crate::util::ConnectedSocket;
use crate::{ConnpoolItem, ProxyBody};

/// FastCGI handshake using vibeio executor.
pub async fn fcgi_handshake<I>(
    io: I,
    keepalive: bool,
) -> Result<
    SendRequest<UnsyncBoxBody<Bytes, std::io::Error>>,
    Box<dyn std::error::Error + Send + Sync>,
>
where
    I: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + 'static,
{
    let (sender, conn) = cegla_fcgi::client::handshake(io, keepalive).await?;
    vibeio::spawn(async move {
        let _ = conn.await;
    });
    Ok(sender)
}

#[derive(Debug)]
pub enum ClientError {
    ServiceUnavailable(Box<dyn std::error::Error + Send + Sync>),
    Other(Box<dyn std::error::Error + Send + Sync>),
}

impl std::error::Error for ClientError {}
impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::ServiceUnavailable(err) => write!(f, "Service unavailable: {err}"),
            ClientError::Other(err) => write!(f, "{err}"),
        }
    }
}

/// Establish a new connection to the FastCGI backend.
pub async fn establish_connection(
    url: &str,
    keepalive: bool,
) -> Result<SendRequest<UnsyncBoxBody<Bytes, std::io::Error>>, ClientError> {
    let scgi_to_url = url
        .parse::<http::Uri>()
        .map_err(|e| ClientError::Other(Box::new(e)))?;
    let scheme_str = scgi_to_url.scheme_str();

    let connected_socket = match scheme_str {
        Some("tcp") => {
            let host = match scgi_to_url.host() {
                Some(host) => host,
                None => {
                    return Err(ClientError::Other(
                        anyhow::anyhow!("The FastCGI URL doesn't include the port")
                            .into_boxed_dyn_error(),
                    ));
                }
            };

            let port = match scgi_to_url.port_u16() {
                Some(port) => port,
                None => {
                    return Err(ClientError::Other(
                        anyhow::anyhow!("The FastCGI URL doesn't include the port")
                            .into_boxed_dyn_error(),
                    ));
                }
            };

            let addr = format!("{host}:{port}");

            match ConnectedSocket::connect_tcp(&addr).await {
                Ok(data) => data,
                Err(err) => match err.kind() {
                    std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::NotFound
                    | std::io::ErrorKind::HostUnreachable => {
                        return Err(ClientError::ServiceUnavailable(Box::new(err)));
                    }
                    _ => return Err(ClientError::Other(Box::new(err))),
                },
            }
        }
        Some("unix") => {
            let path = scgi_to_url.path();
            match ConnectedSocket::connect_unix(path).await {
                Ok(data) => data,
                Err(err) => match err.kind() {
                    std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::NotFound
                    | std::io::ErrorKind::HostUnreachable => {
                        return Err(ClientError::ServiceUnavailable(Box::new(err)));
                    }
                    _ => return Err(ClientError::Other(Box::new(err))),
                },
            }
        }
        _ => {
            return Err(ClientError::Other(
                anyhow::anyhow!("Only TCP and Unix socket URLs are supported.")
                    .into_boxed_dyn_error(),
            ))
        }
    };

    fcgi_handshake(connected_socket, keepalive)
        .await
        .map_err(ClientError::Other)
}

/// Shared state for the FastCGI client.
pub struct FcgiClient {
    /// Connection pool
    pool: Arc<connpool::Pool<String, SendRequest<ProxyBody>>>,
    /// Global connection limit
    global_limit: usize,
    /// Per-upstream local limits
    local_limits: Arc<RwLock<HashMap<String, usize>>>,
}

impl FcgiClient {
    /// Create a new FastCGI client.
    pub fn new(global_limit: usize) -> Self {
        Self {
            pool: Arc::new(connpool::Pool::new(global_limit)),
            global_limit,
            local_limits: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Establish a new connection to the FastCGI backend.
    pub async fn establish_connection(
        &self,
        url: &str,
        keep_alive: bool,
    ) -> Result<SendRequest<UnsyncBoxBody<Bytes, std::io::Error>>, ClientError> {
        establish_connection(url, keep_alive).await
    }

    /// Get or create a connection from the pool.
    pub async fn get_connection(
        &self,
        url: &str,
        keep_alive: bool,
        local_limit: Option<usize>,
    ) -> Result<ConnpoolItem, ClientError> {
        // Try to get from pool first
        let mut item = self
            .pool
            .pull_with_wait_local_limit(url.to_string(), local_limit)
            .await;
        if let Some(inner) = item.inner_mut() {
            // Check if connection is closed
            if !inner.is_closed() {
                return Ok(item);
            }
        }

        // Establish new connection
        let established = self.establish_connection(url, keep_alive).await?;

        // Fill the item
        *item.inner_mut() = Some(established);

        Ok(item)
    }

    /// Update global connection limit.
    pub fn update_global_limit(&mut self, new_limit: usize) {
        self.global_limit = new_limit;
    }

    /// Set local connection limit for a specific upstream.
    pub async fn set_local_limit(&self, url: &str, limit: usize) {
        let mut limits = self.local_limits.write().await;
        if let std::collections::hash_map::Entry::Vacant(e) = limits.entry(url.to_string()) {
            e.insert(self.pool.set_local_limit(limit));
        }
    }

    /// Get local connection limit for a specific upstream.
    pub async fn get_local_limit(&self, url: &str) -> Option<usize> {
        let limits = self.local_limits.read().await;
        limits.get(url).copied()
    }
}

impl Clone for FcgiClient {
    fn clone(&self) -> Self {
        Self {
            pool: self.pool.clone(),
            global_limit: self.global_limit,
            local_limits: self.local_limits.clone(),
        }
    }
}
