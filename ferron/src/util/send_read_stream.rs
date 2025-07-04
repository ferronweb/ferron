use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use async_channel::Receiver;
use bytes::{Bytes, BytesMut};
use futures_util::Stream;
use tokio::io::{AsyncRead, AsyncReadExt};

const MAX_BUFFER_SIZE: usize = 16384;
const MAX_CHANNEL_CAPACITY: usize = 2;

/// A wrapper over struct implementing Tokio's `AsyncRead`  (no need for struct to be `Send`) that implements `Stream` trait.
#[allow(clippy::type_complexity)]
pub struct SendReadStream {
  rx: Pin<Box<Receiver<Result<Bytes, std::io::Error>>>>,
}

impl SendReadStream {
  /// Creates a new stream and sink from a struct implementing Tokio's `AsyncRead` and `AsyncWrite`
  pub fn new(mut reader: impl AsyncRead + Unpin + 'static) -> Self {
    let (inner_tx, rx) = async_channel::bounded(MAX_CHANNEL_CAPACITY);
    monoio::spawn(async move {
      loop {
        let buffer_sz = MAX_BUFFER_SIZE;
        if buffer_sz == 0 {
          break;
        }
        let mut buffer = BytesMut::with_capacity(buffer_sz);
        let io_future = reader.read_buf(&mut buffer);
        let mut io_future_pinned = Box::pin(io_future);
        let inner_tx_borrowed = &inner_tx;
        let io_result = futures_util::future::poll_fn(|cx| {
          if inner_tx_borrowed.is_closed() {
            return Poll::Ready(Err(std::io::Error::other("The channel is closed")));
          }
          Pin::new(&mut io_future_pinned).poll(cx)
        })
        .await;
        if let Ok(n) = io_result.as_ref() {
          if n == &0 {
            break;
          }
        } else if inner_tx.is_closed() {
          break;
        }
        let is_err = io_result.is_err();
        if inner_tx
          .send(io_result.map(move |n| {
            buffer.truncate(n);
            buffer.freeze()
          }))
          .await
          .is_err()
        {
          return;
        }
        if is_err {
          break;
        }
      }
    });
    Self { rx: Box::pin(rx) }
  }
}

impl Stream for SendReadStream {
  type Item = Result<Bytes, std::io::Error>;

  fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
    Pin::new(&mut self.rx).poll_next(cx)
  }
}

impl Drop for SendReadStream {
  fn drop(&mut self) {
    self.rx.close();
  }
}
