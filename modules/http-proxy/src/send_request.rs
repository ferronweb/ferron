//! Hyper SendRequest wrapper for connection pooling.

use std::sync::Arc;
use std::task::Context;
use std::task::Poll;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::combinators::UnsyncBoxBody;
use hyper::body::Incoming;
use hyper::Request;
use hyper::Response;
use tokio::io::{AsyncRead, AsyncWrite};
use vibeio_hyper::VibeioIo;

/// Body type used for proxied requests.
pub type ProxyBody = UnsyncBoxBody<Bytes, std::io::Error>;

enum SendRequestInner {
    Http1(hyper::client::conn::http1::SendRequest<ProxyBody>),
    Http2(hyper::client::conn::http2::SendRequest<ProxyBody>),
}

#[allow(dead_code)]
impl SendRequestInner {
    fn is_closed(&self) -> bool {
        match self {
            SendRequestInner::Http1(s) => s.is_closed(),
            SendRequestInner::Http2(s) => s.is_closed(),
        }
    }

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
            Some(SendRequestInner::Http1(inner)) => inner.is_closed(),
            Some(SendRequestInner::Http2(inner)) => inner.is_closed(),
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
    drop_guard: crate::send_net_io::SendTcpStreamPollDropGuard,
) -> Result<SendRequestWrapper, Box<dyn std::error::Error + Send + Sync>>
where
    I: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let io = VibeioIo::new(io);
    let (sender, conn) = hyper::client::conn::http1::handshake(io).await?;
    let conn_with_upgrades = conn.with_upgrades();
    vibeio::spawn(async move {
        let _ = conn_with_upgrades.await;
        drop(drop_guard);
    });
    Ok(SendRequestWrapper::http1(sender))
}

/// HTTP/1.x handshake for Unix sockets.
#[cfg(unix)]
pub async fn http1_handshake_unix<I>(
    io: I,
    drop_guard: crate::send_net_io::SendUnixStreamPollDropGuard,
) -> Result<SendRequestWrapper, Box<dyn std::error::Error + Send + Sync>>
where
    I: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let io = VibeioIo::new(io);
    let (sender, conn) = hyper::client::conn::http1::handshake(io).await?;
    let conn_with_upgrades = conn.with_upgrades();
    vibeio::spawn(async move {
        let _ = conn_with_upgrades.await;
        drop(drop_guard);
    });
    Ok(SendRequestWrapper::http1(sender))
}

/// HTTP/2 handshake using vibeio executor.
pub async fn http2_handshake<I>(
    io: I,
    drop_guard: crate::send_net_io::SendTcpStreamPollDropGuard,
) -> Result<SendRequestWrapper, Box<dyn std::error::Error + Send + Sync>>
where
    I: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let io = VibeioIo::new(io);
    let executor = vibeio_hyper::VibeioExecutor;
    let (sender, conn) = hyper::client::conn::http2::handshake(executor, io).await?;
    vibeio::spawn(async move {
        let _ = conn.await;
        drop(drop_guard);
    });
    Ok(SendRequestWrapper::http2(sender))
}

/// HTTP/2 handshake for Unix sockets.
#[cfg(unix)]
pub async fn http2_handshake_unix<I>(
    io: I,
    drop_guard: crate::send_net_io::SendUnixStreamPollDropGuard,
) -> Result<SendRequestWrapper, Box<dyn std::error::Error + Send + Sync>>
where
    I: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let io = VibeioIo::new(io);
    let executor = vibeio_hyper::VibeioExecutor;
    let (sender, conn) = hyper::client::conn::http2::handshake(executor, io).await?;
    vibeio::spawn(async move {
        let _ = conn.await;
        drop(drop_guard);
    });
    Ok(SendRequestWrapper::http2(sender))
}

/// Information needed to return a connection back to the thread-local pool.
pub struct PoolReturnInfo {
    /// The upstream and client IP key.
    key: Option<crate::connections::PoolKey>,
    /// The connection wrapper to return.
    wrapper: Option<SendRequestWrapper>,
    /// Local limit index, if one was applied.
    local_limit_idx: Option<usize>,
    /// Whether this is a Unix pool connection.
    is_unix: bool,
}

impl PoolReturnInfo {
    /// Creates a new `PoolReturnInfo` from a pool item and wrapper.
    ///
    /// This consumes the item without running its Drop impl (via `ManuallyDrop`),
    /// allowing the wrapper to be stored separately and returned later.
    pub fn from_item(
        item: crate::connpool_single::PoolItem<crate::connections::PoolKey, SendRequestWrapper>,
        wrapper: SendRequestWrapper,
        is_unix: bool,
    ) -> Self {
        // Prevent item's Drop from running (we'll handle return manually)
        let item = std::mem::ManuallyDrop::new(item);

        Self {
            key: item.key().cloned(),
            wrapper: Some(wrapper),
            local_limit_idx: item.local_limit_index(),
            is_unix,
        }
    }
}

impl Drop for PoolReturnInfo {
    fn drop(&mut self) {
        if let Some(wrapper) = self.wrapper.take() {
            if let Some(ref key) = self.key {
                // Return the connection to the thread-local pool.
                // This is safe because we're on the same thread that pulled it.
                crate::connections::return_connection_to_pool(
                    key,
                    wrapper,
                    self.local_limit_idx,
                    self.is_unix,
                );
            }
        }
    }
}

/// A tracked response body that returns the connection to the pool
/// after the body is fully consumed, and decrements the connection
/// tracker for LeastConnections/TwoRandomChoices algorithms.
pub struct TrackedBody<B> {
    inner: B,
    _tracker: Option<Arc<()>>,
    _tracker_pool: Option<PoolReturnInfo>,
}

impl<B> TrackedBody<B> {
    pub fn new(inner: B, tracker: Option<Arc<()>>, tracker_pool: Option<PoolReturnInfo>) -> Self {
        Self {
            inner,
            _tracker: tracker,
            _tracker_pool: tracker_pool,
        }
    }
}

impl<B> hyper::body::Body for TrackedBody<B>
where
    B: hyper::body::Body + Unpin,
{
    type Data = B::Data;
    type Error = B::Error;

    fn poll_frame(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<hyper::body::Frame<Self::Data>, Self::Error>>> {
        std::pin::Pin::new(&mut self.inner).poll_frame(cx)
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> hyper::body::SizeHint {
        self.inner.size_hint()
    }
}
