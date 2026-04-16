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
use tokio_util::sync::ReusableBoxFuture;

const MAX_BUFFER_SIZE: usize = 65536;

type ReadChunkResult = Option<Result<Bytes, io::Error>>;
type ReadChunkFuture = ReusableBoxFuture<'static, ReadChunkResult>;

/// A wrapper over `vibeio::fs::File` that implements `futures_core::Stream`.
///
/// Uses `read_at` for position-based async reads without spawning blocking threads.
/// `SendWrapper` ensures the non-`Send` `vibeio::fs::File` can safely cross thread boundaries
/// as long as it's only polled on the same thread (guaranteed by the single-threaded runtime).
pub struct FileStream {
    file: Arc<SendWrapper<vibeio::fs::File>>,
    current_pos: u64,
    remaining: Option<u64>,
    finished: bool,
    read_future: Option<ReadChunkFuture>,
}

impl FileStream {
    /// Create a new `FileStream` reading from `start` to `end` (exclusive).
    /// If `end` is `None`, reads until EOF.
    pub fn new(file: vibeio::fs::File, start: u64, end: Option<u64>) -> Self {
        let remaining = remaining_from_bounds(start, end);
        let finished = matches!(remaining, Some(0));

        Self {
            file: Arc::new(SendWrapper::new(file)),
            current_pos: start,
            remaining,
            finished,
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
            self.read_future = Some(ReusableBoxFuture::new(SendWrapper::new(read_chunk(
                self.file.clone(),
                self.current_pos,
                self.remaining,
            ))));
        }

        let poll_result = self
            .read_future
            .as_mut()
            .expect("file stream read future is not initialized")
            .poll(cx);

        match poll_result {
            Poll::Ready(Some(Ok(chunk))) => {
                let chunk_len = chunk.len() as u64;
                self.current_pos = self.current_pos.saturating_add(chunk_len);
                if let Some(remaining) = &mut self.remaining {
                    *remaining = remaining.saturating_sub(chunk_len);
                    self.finished = *remaining == 0;
                }

                if self.finished {
                    self.read_future = None;
                } else {
                    let file = self.file.clone();
                    let current_pos = self.current_pos;
                    let remaining = self.remaining;
                    self.read_future
                        .as_mut()
                        .expect("file stream read future is not initialized")
                        .set(SendWrapper::new(read_chunk(file, current_pos, remaining)));
                }
                Poll::Ready(Some(Ok(chunk)))
            }
            Poll::Ready(option) => {
                if option.is_none() {
                    self.finished = true;
                }
                self.read_future = None;
                Poll::Ready(option)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

#[inline]
fn remaining_from_bounds(start: u64, end: Option<u64>) -> Option<u64> {
    end.map(|end| end.saturating_sub(start))
}

#[inline]
fn buffer_size_for_read(remaining: Option<u64>) -> usize {
    remaining.map_or(MAX_BUFFER_SIZE, |remaining| {
        remaining.min(MAX_BUFFER_SIZE as u64) as usize
    })
}

/// Reads a single chunk from a vibeio file at the given position.
async fn read_chunk(
    file: Arc<SendWrapper<vibeio::fs::File>>,
    pos: u64,
    remaining: Option<u64>,
) -> ReadChunkResult {
    let buffer_sz = buffer_size_for_read(remaining);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remaining_from_bounds_saturates_when_end_precedes_start() {
        assert_eq!(remaining_from_bounds(10, Some(5)), Some(0));
    }

    #[test]
    fn remaining_from_bounds_returns_unbounded_for_open_ended_streams() {
        assert_eq!(remaining_from_bounds(10, None), None);
    }

    #[test]
    fn buffer_size_for_read_uses_full_chunk_for_unbounded_streams() {
        assert_eq!(buffer_size_for_read(None), MAX_BUFFER_SIZE);
    }

    #[test]
    fn buffer_size_for_read_caps_to_max_buffer_size() {
        assert_eq!(
            buffer_size_for_read(Some((MAX_BUFFER_SIZE as u64) * 2)),
            MAX_BUFFER_SIZE
        );
    }

    #[test]
    fn buffer_size_for_read_uses_remaining_bytes_for_last_chunk() {
        assert_eq!(buffer_size_for_read(Some(123)), 123);
        assert_eq!(buffer_size_for_read(Some(0)), 0);
    }
}
