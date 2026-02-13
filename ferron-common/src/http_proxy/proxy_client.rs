use std::cell::UnsafeCell;
use std::error::Error;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use http_body_util::BodyExt;
use hyper::body::Body;
use hyper::{Request, Response, StatusCode};
#[cfg(feature = "runtime-tokio")]
use hyper_util::rt::{TokioExecutor, TokioIo};
#[cfg(feature = "runtime-monoio")]
use monoio_compat::hyper::{MonoioExecutor, MonoioIo};
use tokio::io::{AsyncRead, AsyncWrite};

use super::{ConnectionPoolItem, DropGuard};
use crate::http_proxy::send_request::{SendRequest, SendRequestWrapper};
use crate::logging::ErrorLogger;
use crate::modules::ResponseData;

/// A tracked response body.
struct TrackedBody<B> {
  inner: B,
  _tracker: Option<Arc<()>>,
  _tracker_pool: Option<Arc<UnsafeCell<ConnectionPoolItem>>>,
}

impl<B> TrackedBody<B> {
  fn new(inner: B, tracker: Option<Arc<()>>, tracker_pool: Option<Arc<UnsafeCell<ConnectionPoolItem>>>) -> Self {
    Self {
      inner,
      _tracker: tracker,
      _tracker_pool: tracker_pool,
    }
  }
}

impl<B> Body for TrackedBody<B>
where
  B: Body + Unpin,
{
  type Data = B::Data;
  type Error = B::Error;

  #[inline]
  fn poll_frame(
    mut self: Pin<&mut Self>,
    cx: &mut Context<'_>,
  ) -> Poll<Option<Result<hyper::body::Frame<Self::Data>, Self::Error>>> {
    Pin::new(&mut self.inner).poll_frame(cx)
  }

  #[inline]
  fn is_end_stream(&self) -> bool {
    self.inner.is_end_stream()
  }

  #[inline]
  fn size_hint(&self) -> hyper::body::SizeHint {
    self.inner.size_hint()
  }
}

// Safety: after construction, the value inside `UnsafeCell` is never mutated.
// All accesses after sharing are read-only, so sharing across threads is safe.
unsafe impl<B> Send for TrackedBody<B> where B: Send {}
unsafe impl<B> Sync for TrackedBody<B> where B: Sync {}

/// Establishes a new HTTP connection to a backend server.
pub(super) async fn http_proxy_handshake(
  stream: impl AsyncRead + AsyncWrite + Send + Unpin + 'static,
  use_http2: bool,
  #[cfg(feature = "runtime-monoio")] drop_guard: DropGuard,
) -> Result<SendRequest, Box<dyn Error + Send + Sync>> {
  #[cfg(feature = "runtime-monoio")]
  let io = MonoioIo::new(stream);
  #[cfg(feature = "runtime-tokio")]
  let io = TokioIo::new(stream);

  Ok(if use_http2 {
    #[cfg(feature = "runtime-monoio")]
    let executor = MonoioExecutor;
    #[cfg(feature = "runtime-tokio")]
    let executor = TokioExecutor::new();

    let (sender, conn) = hyper::client::conn::http2::handshake(executor, io).await?;

    crate::runtime::spawn(async move {
      conn.await.unwrap_or_default();
      #[cfg(feature = "runtime-monoio")]
      drop(drop_guard);
    });

    SendRequest::Http2(sender)
  } else {
    let (sender, conn) = hyper::client::conn::http1::handshake(io).await?;

    let conn_with_upgrades = conn.with_upgrades();
    crate::runtime::spawn(async move {
      conn_with_upgrades.await.unwrap_or_default();
      #[cfg(feature = "runtime-monoio")]
      drop(drop_guard);
    });

    SendRequest::Http1(sender)
  })
}

/// Forwards an HTTP request to a backend server.
pub(super) async fn http_proxy(
  mut sender: SendRequest,
  connection_pool_item: ConnectionPoolItem,
  proxy_request: Request<BoxBody<Bytes, std::io::Error>>,
  error_logger: &ErrorLogger,
  proxy_intercept_errors: bool,
  tracked_connection: Option<Arc<()>>,
  enable_keepalive: bool,
) -> Result<ResponseData, Box<dyn Error + Send + Sync>> {
  let (proxy_request_parts, proxy_request_body) = proxy_request.into_parts();
  let proxy_request_cloned = Request::from_parts(proxy_request_parts.clone(), ());
  let proxy_request = Request::from_parts(proxy_request_parts, proxy_request_body);

  let send_request_result = sender.send_request(proxy_request).await;
  #[allow(clippy::arc_with_non_send_sync)]
  let connection_pool_item = Arc::new(UnsafeCell::new(connection_pool_item));

  let proxy_response = match send_request_result {
    Ok(response) => response,
    Err(err) => {
      error_logger.log(&format!("Bad gateway: {err}")).await;
      return Ok(ResponseData {
        request: None,
        response: None,
        response_status: Some(StatusCode::BAD_GATEWAY),
        response_headers: None,
        new_remote_address: None,
      });
    }
  };

  let status_code = proxy_response.status();

  let (proxy_response_parts, proxy_response_body) = proxy_response.into_parts();
  if proxy_response_parts.status == StatusCode::SWITCHING_PROTOCOLS {
    let proxy_response_cloned = Response::from_parts(proxy_response_parts.clone(), ());
    match hyper::upgrade::on(proxy_response_cloned).await {
      Ok(upgraded_backend) => {
        let error_logger = error_logger.clone();
        let connection_pool_item = connection_pool_item.clone();
        crate::runtime::spawn(async move {
          match hyper::upgrade::on(proxy_request_cloned).await {
            Ok(upgraded_proxy) => {
              #[cfg(feature = "runtime-monoio")]
              let mut upgraded_backend = MonoioIo::new(upgraded_backend);
              #[cfg(feature = "runtime-tokio")]
              let mut upgraded_backend = TokioIo::new(upgraded_backend);

              #[cfg(feature = "runtime-monoio")]
              let mut upgraded_proxy = MonoioIo::new(upgraded_proxy);
              #[cfg(feature = "runtime-tokio")]
              let mut upgraded_proxy = TokioIo::new(upgraded_proxy);

              crate::runtime::spawn(async move {
                tokio::io::copy_bidirectional(&mut upgraded_backend, &mut upgraded_proxy)
                  .await
                  .unwrap_or_default();
                drop(connection_pool_item);
              });
            }
            Err(err) => {
              error_logger.log(&format!("HTTP upgrade error: {err}")).await;
            }
          }
        });
      }
      Err(err) => {
        error_logger.log(&format!("HTTP upgrade error: {err}")).await;
      }
    }
  }
  let proxy_response = Response::from_parts(proxy_response_parts, proxy_response_body);

  let response = if proxy_intercept_errors && status_code.as_u16() >= 400 {
    ResponseData {
      request: None,
      response: None,
      response_status: Some(status_code),
      response_headers: None,
      new_remote_address: None,
    }
  } else {
    let (response_parts, response_body) = proxy_response.into_parts();
    let boxed_body = TrackedBody::new(
      response_body.map_err(|e| std::io::Error::other(e.to_string())),
      tracked_connection,
      if enable_keepalive && !sender.is_closed() {
        None
      } else {
        Some(connection_pool_item.clone())
      },
    )
    .boxed();
    ResponseData {
      request: None,
      response: Some(Response::from_parts(response_parts, boxed_body)),
      response_status: None,
      response_headers: None,
      new_remote_address: None,
    }
  };

  if enable_keepalive && !sender.is_closed() {
    let connection_pool_item = unsafe { &mut *connection_pool_item.get() };
    connection_pool_item
      .inner_mut()
      .replace(SendRequestWrapper::new(sender));
  }

  drop(connection_pool_item);

  Ok(response)
}
