use std::pin::Pin;
use std::task::{Context, Poll};

use async_channel::{Receiver, Sender};
use bytes::{Bytes, BytesMut};
use futures_util::{Sink, Stream};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

const MAX_BUFFER_SIZE: usize = 16384;
const MAX_READ_CHANNEL_CAPACITY: usize = 2;
const MAX_WRITE_CHANNEL_CAPACITY: usize = 2;

/// A wrapper over struct implementing Tokio's `AsyncRead` and `AsyncWrite` (no need for struct to be `Send`) that implements `Stream` and `Sink` trait.
#[allow(clippy::type_complexity)]
pub struct SendRwStream {
  rx: Pin<Box<Receiver<Result<Bytes, std::io::Error>>>>,
  tx: Pin<Box<dyn Sink<Bytes, Error = std::io::Error> + Send>>,
  read_cancel: CancellationToken,
}

impl SendRwStream {
  /// Creates a new stream and sink from a struct implementing Tokio's `AsyncRead` and `AsyncWrite`
  pub fn new(stream: impl AsyncRead + AsyncWrite + Unpin + 'static) -> Self {
    let (inner_tx, rx) = async_channel::bounded(MAX_READ_CHANNEL_CAPACITY);
    let (tx, inner_rx) = async_channel::bounded(MAX_WRITE_CHANNEL_CAPACITY);
    let (mut reader, mut writer) = tokio::io::split(stream);
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

          result = reader.read_buf(&mut buffer) => {
            result
          }
          _ = read_cancel_clone.cancelled() => {
            break;
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
    monoio::spawn(async move {
      while let Ok(mut bytes) = inner_rx.recv().await {
        if writer.write_all_buf(&mut bytes).await.is_err() {
          break;
        }
        if writer.flush().await.is_err() {
          break;
        }
      }
      inner_rx.close();
    });
    let tx = futures_util::sink::unfold(SenderWrap { inner: tx }, async move |tx, data: Bytes| {
      tx.send(data)
        .await
        .map_err(std::io::Error::other)
        .map(|_| tx)
    });
    Self {
      rx: Box::pin(rx),
      tx: Box::pin(tx),
      read_cancel,
    }
  }
}

impl Sink<Bytes> for SendRwStream {
  type Error = std::io::Error;

  fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
    Pin::new(&mut self.tx).poll_close(cx)
  }

  fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
    Pin::new(&mut self.tx).poll_flush(cx)
  }

  fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
    Pin::new(&mut self.tx).poll_ready(cx)
  }

  fn start_send(mut self: Pin<&mut Self>, item: Bytes) -> Result<(), Self::Error> {
    Pin::new(&mut self.tx).start_send(item)
  }
}

impl Stream for SendRwStream {
  type Item = Result<Bytes, std::io::Error>;

  fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
    Pin::new(&mut self.rx).poll_next(cx)
  }
}

impl Drop for SendRwStream {
  fn drop(&mut self) {
    self.rx.close();
    self.read_cancel.cancel();
  }
}

struct SenderWrap<T> {
  inner: Sender<T>,
}

impl<T> SenderWrap<T> {
  fn send(&self, data: T) -> async_channel::Send<'_, T> {
    self.inner.send(data)
  }
}

impl<T> Drop for SenderWrap<T> {
  fn drop(&mut self) {
    self.inner.close();
  }
}
