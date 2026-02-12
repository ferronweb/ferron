use std::time::Duration;

use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use hyper::{Request, Response};
use tokio::time::Instant;

/// A wrapper around Hyper's SendRequest that can be used with multiple HTTP versions
pub enum SendRequest {
  Http1(hyper::client::conn::http1::SendRequest<BoxBody<Bytes, std::io::Error>>),
  Http2(hyper::client::conn::http2::SendRequest<BoxBody<Bytes, std::io::Error>>),
}

impl SendRequest {
  /// Checks whether the related connection is closed.
  #[inline]
  pub fn is_closed(&self) -> bool {
    match self {
      SendRequest::Http1(sender) => sender.is_closed(),
      SendRequest::Http2(sender) => sender.is_closed(),
    }
  }

  /// Waits until the related connection is ready.
  #[inline]
  pub async fn ready(&mut self) -> bool {
    match self {
      SendRequest::Http1(sender) => !sender.is_closed() && sender.ready().await.is_ok(),
      SendRequest::Http2(sender) => !sender.is_closed() && sender.ready().await.is_ok(),
    }
  }

  /// Checks whether the related connection is ready.
  #[inline]
  pub fn is_ready(&self) -> bool {
    match self {
      SendRequest::Http1(sender) => sender.is_ready() && !sender.is_closed(),
      SendRequest::Http2(sender) => sender.is_ready() && !sender.is_closed(),
    }
  }

  /// Send an HTTP request to the related connection.
  #[inline]
  pub async fn send_request(
    &mut self,
    request: Request<BoxBody<Bytes, std::io::Error>>,
  ) -> Result<Response<hyper::body::Incoming>, hyper::Error> {
    match self {
      SendRequest::Http1(sender) => sender.send_request(request).await,
      SendRequest::Http2(sender) => sender.send_request(request).await,
    }
  }
}

/// A wrapper around `SendRequest`, with idle keep-alive timeout support.
pub struct SendRequestWrapper {
  inner: Option<SendRequest>,
  instant: Instant,
}

impl SendRequestWrapper {
  /// Creates a new `SendRequestWrapper`
  #[inline]
  pub fn new(inner: SendRequest) -> Self {
    Self {
      inner: Some(inner),
      instant: Instant::now(),
    }
  }

  /// Gets the inner `SendRequest`, along with information on whether to put back the connection to the pool.
  #[inline]
  pub fn get(&mut self, timeout: Option<Duration>) -> (Option<SendRequest>, bool) {
    let inner_mut = if let Some(inner) = self.inner.as_mut() {
      inner
    } else {
      return (None, false);
    };
    if inner_mut.is_closed() || (inner_mut.is_ready() && timeout.is_some_and(|t| self.instant.elapsed() > t)) {
      return (None, false);
    }
    (
      if inner_mut.is_ready() {
        self.inner.take()
      } else {
        self.instant = Instant::now();
        None
      },
      true,
    )
  }

  /// Waits until the inner `SendRequest` is ready. Return information on whether to put back the connection to the pool.
  #[inline]
  pub async fn wait_ready(&mut self, timeout: Option<Duration>) -> bool {
    match self.inner.as_mut() {
      None => false,
      Some(inner) => {
        if inner.is_ready() && timeout.is_some_and(|t| self.instant.elapsed() > t) {
          return false;
        }
        inner.ready().await
      }
    }
  }
}
