use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_util::Stream;
use monoio::fs::File;
use send_wrapper::SendWrapper;

const MAX_BUFFER_SIZE: usize = 16384;

/// A wrapper over Monoio's `File` that implements a `Stream` trait and doesn't spawn a background task.
#[allow(clippy::type_complexity)]
pub struct MonoioFileStreamNoSpawn {
  file: Arc<SendWrapper<File>>,
  current_pos: u64,
  end: Option<u64>,
  read_future: Option<Pin<Box<dyn Future<Output = Option<Result<Bytes, std::io::Error>>> + Send + Sync>>>,
}

impl MonoioFileStreamNoSpawn {
  /// Creates a new stream from Monoio's `File`, with specified start and end positions
  pub fn new(file: File, start: Option<u64>, end: Option<u64>) -> Self {
    Self {
      file: Arc::new(SendWrapper::new(file)),
      current_pos: start.unwrap_or(0),
      end,
      read_future: None,
    }
  }
}

impl Stream for MonoioFileStreamNoSpawn {
  type Item = Result<Bytes, std::io::Error>;

  fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
    if let Some(end) = &self.end {
      if self.current_pos >= *end {
        // EOF
        return Poll::Ready(None);
      }
    }
    if self.read_future.is_none() {
      self.read_future = Some(Box::pin(SendWrapper::new(read_chunk(
        self.file.clone(),
        self.current_pos,
        self.end,
      ))));
    }
    match Pin::new(
      self
        .read_future
        .as_mut()
        .expect("file stream read future is not initialized"),
    )
    .poll(cx)
    {
      Poll::Ready(Some(Ok(chunk))) => {
        let _ = self.read_future.take();
        self.current_pos += chunk.len() as u64;
        Poll::Ready(Some(Ok(chunk)))
      }
      Poll::Ready(option) => {
        let _ = self.read_future.take();
        Poll::Ready(option)
      }
      Poll::Pending => Poll::Pending,
    }
  }
}

async fn read_chunk(file: Arc<SendWrapper<File>>, pos: u64, end: Option<u64>) -> Option<Result<Bytes, std::io::Error>> {
  let buffer_sz = end.map_or(MAX_BUFFER_SIZE, |n| ((n - pos) as usize).min(MAX_BUFFER_SIZE));
  if buffer_sz == 0 {
    return None;
  }
  let buffer_uninit = Box::new_uninit_slice(buffer_sz);
  // Safety: The buffer is a boxed slice of uninitialized `u8` values. `u8` is a primitive type.
  let buffer: Box<[u8]> = unsafe { buffer_uninit.assume_init() };
  let result = file.read_at(buffer, pos).await;
  match result {
    (Ok(n), buffer) => {
      if n == 0 {
        None
      } else {
        let mut bytes = Bytes::from_owner(buffer);
        bytes.truncate(n);
        Some(Ok(bytes))
      }
    }
    (Err(e), _) => Some(Err(e)),
  }
}
