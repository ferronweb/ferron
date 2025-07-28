use std::pin::Pin;
use std::task::{Context, Poll};

use async_channel::Receiver;
use bytes::{Bytes, BytesMut};
use futures_util::Stream;
use monoio::fs::File;
use tokio_util::sync::CancellationToken;

const MAX_BUFFER_SIZE: usize = 16384;
const MAX_CHANNEL_CAPACITY: usize = 2;

/// A wrapper over Monoio's `File` that implements a `Stream` trait.
#[allow(clippy::type_complexity)]
pub struct MonoioFileStream {
  rx: Pin<Box<Receiver<Result<Bytes, std::io::Error>>>>,
  read_cancel: CancellationToken,
}

impl MonoioFileStream {
  /// Creates a new stream from Monoio's `File`, with specified start and end positions
  pub fn new(file: File, start: Option<u64>, end: Option<u64>) -> Self {
    let (tx, rx) = async_channel::bounded(MAX_CHANNEL_CAPACITY);
    let read_cancel = CancellationToken::new();
    let read_cancel_clone = read_cancel.clone();
    monoio::spawn(async move {
      let mut current_pos = start.unwrap_or(0);
      loop {
        let buffer_sz = end.map_or(MAX_BUFFER_SIZE, |n| ((n - current_pos) as usize).min(MAX_BUFFER_SIZE));
        if buffer_sz == 0 {
          break;
        }
        let buffer = BytesMut::with_capacity(buffer_sz);
        let (io_result, mut buffer) = monoio::select! {
          biased;

          _ = read_cancel_clone.cancelled() => {
            break;
          }
          result = file.read_at(buffer, current_pos) => {
            result
          }
        };
        if let Ok(n) = io_result.as_ref() {
          if n == &0 {
            break;
          }
          current_pos += *n as u64;
        }
        let is_err = io_result.is_err();
        if tx
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

impl Stream for MonoioFileStream {
  type Item = Result<Bytes, std::io::Error>;

  fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
    Pin::new(&mut self.rx).poll_next(cx)
  }
}

impl Drop for MonoioFileStream {
  fn drop(&mut self) {
    self.rx.close();
    self.read_cancel.cancel();
  }
}
