use std::pin::Pin;
use std::task::{Context, Poll};

use async_channel::Receiver;
use bytes::{Bytes, BytesMut};
use futures_util::Stream;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio_util::sync::CancellationToken;

const MAX_BUFFER_SIZE: usize = 16384;
const MAX_CHANNEL_CAPACITY: usize = 2;

/// A wrapper over struct implementing Tokio's `AsyncRead`  (no need for struct to be `Send`) that implements `Stream` trait.
#[allow(clippy::type_complexity)]
pub struct SendReadStream {
  rx: Pin<Box<Receiver<Result<Bytes, std::io::Error>>>>,
  read_cancel: CancellationToken,
}

impl SendReadStream {
  /// Creates a new stream and sink from a struct implementing Tokio's `AsyncRead` and `AsyncWrite`
  pub fn new(mut reader: impl AsyncRead + Unpin + 'static) -> Self {
    let (inner_tx, rx) = async_channel::bounded(MAX_CHANNEL_CAPACITY);
    let read_cancel = CancellationToken::new();
    let read_cancel_clone = read_cancel.clone();
    monoio::spawn(async move {
      loop {
        let buffer_sz = MAX_BUFFER_SIZE;
        if buffer_sz == 0 {
          break;
        }
        let mut buffer = BytesMut::with_capacity(buffer_sz);
        let io_result = monoio::select! {
          biased;

          _ = read_cancel_clone.cancelled() => {
            break;
          }
          result = reader.read_buf(&mut buffer) => {
            result
          }
        };
        if let Ok(n) = io_result.as_ref() {
          if n == &0 {
            break;
          }
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
    Self {
      rx: Box::pin(rx),
      read_cancel,
    }
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
    self.read_cancel.cancel();
  }
}
