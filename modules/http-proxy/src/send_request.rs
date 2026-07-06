//! Hyper SendRequest wrapper for connection pooling.

use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::combinators::UnsyncBoxBody;
use hyper::body::Incoming;
use hyper::{Request, Response};
use tokio::io::{AsyncRead, AsyncWrite};
use vibeio_hyper::VibeioIo;

use crate::connections::{PoolKey, PooledConnection};
use crate::types::error::ProxyError;
use crate::types::upstream::UpstreamInner;

/// Body type used for proxied requests.
pub type ProxyBody = UnsyncBoxBody<Bytes, std::io::Error>;

enum SendRequestInner {
    Http1(hyper::client::conn::http1::SendRequest<ProxyBody>),
    Http2(hyper::client::conn::http2::SendRequest<ProxyBody>),
}

impl SendRequestInner {
    #[inline]
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
    #[inline]
    pub fn http1(inner: hyper::client::conn::http1::SendRequest<ProxyBody>) -> Self {
        Self {
            inner: Some(SendRequestInner::Http1(inner)),
            last_used: std::time::Instant::now(),
        }
    }

    #[inline]
    pub fn http2(inner: hyper::client::conn::http2::SendRequest<ProxyBody>) -> Self {
        Self {
            inner: Some(SendRequestInner::Http2(inner)),
            last_used: std::time::Instant::now(),
        }
    }

    /// Check if the connection supports multiplexing (HTTP/2).
    #[inline]
    pub fn supports_multiplexing(&self) -> bool {
        matches!(self.inner.as_ref(), Some(SendRequestInner::Http2(_)))
    }

    /// Check if the connection is closed.
    #[inline]
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
    #[inline]
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
            if timeout.is_some_and(|t| self.last_used.elapsed() > t) {
                return (false, false);
            }
            return (true, true);
        }
        self.last_used = std::time::Instant::now();
        (false, true)
    }

    /// Wait until the connection becomes ready, closed, or the idle timeout elapses.
    ///
    /// Returns `true` if the connection is now ready, `false` if closed/timed out.
    #[inline]
    pub async fn wait_ready(&mut self, timeout: Option<Duration>) -> bool {
        std::future::poll_fn(|cx| match &mut self.inner {
            Some(i) => match i.poll_ready(cx) {
                Poll::Ready(Ok(_)) => {
                    if timeout.is_some_and(|t| self.last_used.elapsed() > t) {
                        return Poll::Ready(false);
                    }
                    Poll::Ready(true)
                }
                Poll::Ready(Err(_)) => Poll::Ready(false),
                Poll::Pending => Poll::Pending,
            },
            None => Poll::Ready(false),
        })
        .await
    }

    /// Send an HTTP request and receive the response.
    #[inline]
    pub async fn send_request(
        &mut self,
        req: Request<ProxyBody>,
    ) -> Result<Response<Incoming>, ProxyError> {
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
            None => Err(ProxyError::SendRequestError(
                "send_request wrapper empty".into(),
            )),
        }
    }
}

/// HTTP/1.x handshake using vibeio executor.
pub async fn http1_handshake<I>(
    io: I,
    drop_guard: crate::send_net_io::SendTcpStreamPollDropGuard,
) -> Result<SendRequestWrapper, ProxyError>
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
) -> Result<SendRequestWrapper, ProxyError>
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
) -> Result<SendRequestWrapper, ProxyError>
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
) -> Result<SendRequestWrapper, ProxyError>
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
    key: Option<PoolKey>,
    /// The connection wrapper to return.
    wrapper: Option<SendRequestWrapper>,
    /// Local limit key, if one was applied.
    local_limit_key: Option<Arc<UpstreamInner>>,
    /// Whether this is a Unix pool connection.
    is_unix: bool,
}

impl PoolReturnInfo {
    /// Creates a new `PoolReturnInfo` from a pool item and wrapper.
    ///
    /// This consumes the item without running its Drop impl (via `ManuallyDrop`),
    /// allowing the wrapper to be stored separately and returned later.
    pub fn from_item(item: PooledConnection, wrapper: SendRequestWrapper, is_unix: bool) -> Self {
        // Prevent item's Drop from running (we'll handle return manually)
        let item = std::mem::ManuallyDrop::new(item);

        Self {
            key: item.key().cloned(),
            wrapper: Some(wrapper),
            local_limit_key: item.local_limit_key().cloned(),
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
                    self.local_limit_key.take(),
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
    _truncated_tracker: Option<TruncatedTracker>,
}

impl<B> TrackedBody<B> {
    pub fn new(
        inner: B,
        tracker: Option<Arc<()>>,
        tracker_pool: Option<PoolReturnInfo>,
        truncated_tracker: Option<TruncatedTracker>,
    ) -> Self {
        Self {
            inner,
            _tracker: tracker,
            _tracker_pool: tracker_pool,
            _truncated_tracker: truncated_tracker,
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

/// Shared state for tracking upstream response body consumption.
///
/// Used by `ContentLengthTrackingBody` to record bytes received and
/// whether the body was truncated, and by `TruncatedTracker` to emit
/// metrics/logs when the body is dropped.
pub struct BodyTrackingState {
    /// Whether the body ended before the expected Content-Length.
    pub truncated: std::sync::atomic::AtomicBool,
    /// Total bytes received from the upstream body.
    pub bytes_received: std::sync::atomic::AtomicU64,
    /// Expected Content-Length from the upstream response headers.
    pub expected_length: Option<u64>,
}

impl BodyTrackingState {
    pub fn new(expected_length: Option<u64>) -> Arc<Self> {
        Arc::new(Self {
            truncated: std::sync::atomic::AtomicBool::new(false),
            bytes_received: std::sync::atomic::AtomicU64::new(0),
            expected_length,
        })
    }
}

/// A body wrapper that tracks bytes received and detects premature stream termination.
///
/// When the upstream response includes a `Content-Length` header, this wrapper counts
/// the bytes yielded by `poll_frame` and flags truncation if the stream ends before
/// the expected number of bytes have been received.
pub struct ContentLengthTrackingBody<B> {
    inner: B,
    state: Arc<BodyTrackingState>,
}

impl<B> ContentLengthTrackingBody<B> {
    pub fn new(inner: B, state: Arc<BodyTrackingState>) -> Self {
        Self { inner, state }
    }
}

impl<B> hyper::body::Body for ContentLengthTrackingBody<B>
where
    B: hyper::body::Body<Data = Bytes> + Unpin,
{
    type Data = Bytes;
    type Error = B::Error;

    fn poll_frame(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<hyper::body::Frame<Self::Data>, Self::Error>>> {
        let result = std::pin::Pin::new(&mut self.inner).poll_frame(cx);

        match &result {
            std::task::Poll::Ready(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    let len = data.len() as u64;
                    self.state
                        .bytes_received
                        .fetch_add(len, std::sync::atomic::Ordering::Relaxed);
                }
            }
            std::task::Poll::Ready(None) => {
                // Stream ended — check for truncation
                if let Some(expected) = self.state.expected_length {
                    let received = self
                        .state
                        .bytes_received
                        .load(std::sync::atomic::Ordering::Relaxed);
                    if received < expected {
                        self.state
                            .truncated
                            .store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            }
            _ => {}
        }

        result
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> hyper::body::SizeHint {
        self.inner.size_hint()
    }
}

/// Drop guard that emits a metric and warning log when an upstream response
/// body was truncated (ended before the declared Content-Length).
///
/// This runs when the response body is consumed or dropped, which happens
/// after the HTTP server framework has finished streaming to the client.
pub struct TruncatedTracker {
    state: Arc<BodyTrackingState>,
    backend_url: String,
    events: ferron_observability::CompositeEventSink,
    trace_context: Option<ferron_observability::EventTraceContext>,
}

impl TruncatedTracker {
    pub fn new(
        state: Arc<BodyTrackingState>,
        backend_url: String,
        events: ferron_observability::CompositeEventSink,
        trace_context: Option<ferron_observability::EventTraceContext>,
    ) -> Self {
        Self {
            state,
            backend_url,
            events,
            trace_context,
        }
    }
}

impl Drop for TruncatedTracker {
    fn drop(&mut self) {
        if !self
            .state
            .truncated
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return;
        }

        let bytes = self
            .state
            .bytes_received
            .load(std::sync::atomic::Ordering::Relaxed);
        let expected = self.state.expected_length.unwrap_or(0);

        use ferron_observability::{
            Event, LogAttributeValue, LogEvent, LogLevel, MetricAttributeValue, MetricEvent,
            MetricType, MetricValue,
        };

        self.events.emit(Event::Metric(MetricEvent {
            name: "ferron.proxy.upstream.response_truncated",
            attributes: vec![(
                "ferron.proxy.backend_url",
                MetricAttributeValue::String(self.backend_url.clone()),
            )],
            ty: MetricType::Counter,
            value: MetricValue::U64(1),
            unit: Some("{response}"),
            description: Some("Upstream responses that ended before the declared Content-Length."),
            trace_context: self.trace_context.clone(),
        }));

        self.events.emit(Event::Log(LogEvent {
            level: LogLevel::Warn,
            message: format!(
                "Upstream response ended prematurely: received {bytes}/{expected} bytes"
            ),
            summary: "Upstream response truncated".into(),
            target: "ferron-http-proxy",
            attributes: vec![
                (
                    "ferron.proxy.backend_url",
                    LogAttributeValue::String(self.backend_url.clone()),
                ),
                (
                    "upstream.bytes_received",
                    LogAttributeValue::I64(bytes as i64),
                ),
                (
                    "upstream.content_length",
                    LogAttributeValue::I64(expected as i64),
                ),
            ],
            trace_context: self.trace_context.clone(),
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use hyper::body::Frame;

    /// A simple test body that yields pre-configured frames.
    struct TestBody {
        frames: Vec<Bytes>,
        index: usize,
    }

    impl TestBody {
        fn new(frames: Vec<Bytes>) -> Self {
            Self { frames, index: 0 }
        }
    }

    impl hyper::body::Body for TestBody {
        type Data = Bytes;
        type Error = std::io::Error;

        fn poll_frame(
            mut self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
            if self.index < self.frames.len() {
                let frame = Frame::data(self.frames[self.index].clone());
                self.index += 1;
                std::task::Poll::Ready(Some(Ok(frame)))
            } else {
                std::task::Poll::Ready(None)
            }
        }

        fn is_end_stream(&self) -> bool {
            self.index >= self.frames.len()
        }

        fn size_hint(&self) -> hyper::body::SizeHint {
            hyper::body::SizeHint::new()
        }
    }

    /// Helper: drive a body to completion using poll_frame.
    fn drive_to_completion<B: hyper::body::Body + Unpin>(body: &mut B) {
        let waker = std::task::Waker::noop();
        let mut cx = std::task::Context::from_waker(waker);
        loop {
            let pinned = std::pin::Pin::new(&mut *body);
            match pinned.poll_frame(&mut cx) {
                std::task::Poll::Ready(Some(Ok(_))) => {}
                std::task::Poll::Ready(Some(Err(_))) => break,
                std::task::Poll::Ready(None) => break,
                std::task::Poll::Pending => break,
            }
        }
    }

    #[test]
    fn test_tracking_body_no_content_length() {
        let state = BodyTrackingState::new(None);
        let body = TestBody::new(vec![Bytes::from("hello"), Bytes::from(" world")]);

        let mut tracking = ContentLengthTrackingBody::new(body, state.clone());
        drive_to_completion(&mut tracking);

        assert_eq!(
            state
                .bytes_received
                .load(std::sync::atomic::Ordering::Relaxed),
            11
        );
        assert!(!state.truncated.load(std::sync::atomic::Ordering::Relaxed));
    }

    #[test]
    fn test_tracking_body_complete() {
        let state = BodyTrackingState::new(Some(11));
        let body = TestBody::new(vec![Bytes::from("hello"), Bytes::from(" world")]);

        let mut tracking = ContentLengthTrackingBody::new(body, state.clone());
        drive_to_completion(&mut tracking);

        assert_eq!(
            state
                .bytes_received
                .load(std::sync::atomic::Ordering::Relaxed),
            11
        );
        assert!(!state.truncated.load(std::sync::atomic::Ordering::Relaxed));
    }

    #[test]
    fn test_tracking_body_truncated() {
        let state = BodyTrackingState::new(Some(100));
        let body = TestBody::new(vec![Bytes::from("hello"), Bytes::from(" world")]);

        let mut tracking = ContentLengthTrackingBody::new(body, state.clone());
        drive_to_completion(&mut tracking);

        assert_eq!(
            state
                .bytes_received
                .load(std::sync::atomic::Ordering::Relaxed),
            11
        );
        assert!(state.truncated.load(std::sync::atomic::Ordering::Relaxed));
    }

    #[test]
    fn test_tracking_body_single_frame_exact() {
        let state = BodyTrackingState::new(Some(5));
        let body = TestBody::new(vec![Bytes::from("hello")]);

        let mut tracking = ContentLengthTrackingBody::new(body, state.clone());
        drive_to_completion(&mut tracking);

        assert_eq!(
            state
                .bytes_received
                .load(std::sync::atomic::Ordering::Relaxed),
            5
        );
        assert!(!state.truncated.load(std::sync::atomic::Ordering::Relaxed));
    }

    #[test]
    fn test_tracking_body_more_bytes_than_expected() {
        let state = BodyTrackingState::new(Some(3));
        let body = TestBody::new(vec![Bytes::from("hello")]);

        let mut tracking = ContentLengthTrackingBody::new(body, state.clone());
        drive_to_completion(&mut tracking);

        assert_eq!(
            state
                .bytes_received
                .load(std::sync::atomic::Ordering::Relaxed),
            5
        );
        assert!(!state.truncated.load(std::sync::atomic::Ordering::Relaxed));
    }
}
