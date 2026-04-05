//! Async file streaming utilities.
//!
//! Provides `FileStream` which wraps a `vibeio::fs::File` and implements
//! `futures_core::Stream` for position-based async reads without spawning blocking threads.

use std::io;
use std::sync::Arc;

use bytes::Bytes;
use futures_core::Stream;
use send_wrapper::SendWrapper;
use std::pin::Pin;
use std::task::{Context, Poll};

const MAX_BUFFER_SIZE: usize = 16384;

/// A wrapper over `vibeio::fs::File` that implements `futures_core::Stream`.
///
/// Uses `read_at` for position-based async reads without spawning blocking threads.
/// `SendWrapper` ensures the non-`Send` `vibeio::fs::File` can safely cross thread boundaries
/// as long as it's only polled on the same thread (guaranteed by the single-threaded runtime).
pub struct FileStream {
    file: Arc<SendWrapper<vibeio::fs::File>>,
    current_pos: u64,
    end: Option<u64>,
    finished: bool,
    #[allow(clippy::type_complexity)]
    read_future: Option<
        Pin<Box<dyn std::future::Future<Output = Option<Result<Bytes, io::Error>>> + Send + Sync>>,
    >,
}

impl FileStream {
    /// Create a new `FileStream` reading from `start` to `end` (exclusive).
    /// If `end` is `None`, reads until EOF.
    pub fn new(file: vibeio::fs::File, start: u64, end: Option<u64>) -> Self {
        Self {
            file: Arc::new(SendWrapper::new(file)),
            current_pos: start,
            end,
            finished: false,
            read_future: None,
        }
    }
}

impl Stream for FileStream {
    type Item = Result<Bytes, io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.finished {
            return Poll::Ready(None);
        }

        if self.read_future.is_none() {
            self.read_future = Some(Box::pin(SendWrapper::new(read_chunk(
                self.file.clone(),
                self.current_pos,
                self.end,
            ))));
        }

        match self
            .read_future
            .as_mut()
            .expect("file stream read future is not initialized")
            .as_mut()
            .poll(cx)
        {
            Poll::Ready(Some(Ok(chunk))) => {
                let _ = self.read_future.take();
                self.current_pos += chunk.len() as u64;
                if let Some(end) = &self.end {
                    if self.current_pos >= *end {
                        self.finished = true;
                    }
                }
                Poll::Ready(Some(Ok(chunk)))
            }
            Poll::Ready(option) => {
                let _ = self.read_future.take();
                if option.is_none() {
                    self.finished = true;
                }
                Poll::Ready(option)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Reads a single chunk from a vibeio file at the given position.
async fn read_chunk(
    file: Arc<SendWrapper<vibeio::fs::File>>,
    pos: u64,
    end: Option<u64>,
) -> Option<Result<Bytes, io::Error>> {
    let buffer_sz = end.map_or(MAX_BUFFER_SIZE, |n| {
        ((n - pos) as usize).min(MAX_BUFFER_SIZE)
    });
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
