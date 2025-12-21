use std::pin::Pin;
use std::task::{Context, Poll};
use std::thread::ThreadId;

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// SendAsyncIo is a wrapper around an AsyncRead or AsyncWrite that ensures that all operations are performed on the same thread.
pub struct SendAsyncIo<T> {
  thread_id: ThreadId,
  inner: T,
}

impl<T> SendAsyncIo<T> {
  /// Creates a new SendAsyncIo wrapper around the given AsyncRead or AsyncWrite.
  pub fn new(inner: T) -> Self {
    SendAsyncIo {
      thread_id: std::thread::current().id(),
      inner,
    }
  }
}

impl<T: AsyncRead + Unpin> AsyncRead for SendAsyncIo<T> {
  fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
    if std::thread::current().id() != self.thread_id {
      panic!("SendAsyncIo can only be used from the same thread it was created on");
    }
    Pin::new(&mut self.inner).poll_read(cx, buf)
  }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for SendAsyncIo<T> {
  fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
    if std::thread::current().id() != self.thread_id {
      panic!("SendAsyncIo can only be used from the same thread it was created on");
    }
    Pin::new(&mut self.inner).poll_write(cx, buf)
  }

  fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
    if std::thread::current().id() != self.thread_id {
      panic!("SendAsyncIo can only be used from the same thread it was created on");
    }
    Pin::new(&mut self.inner).poll_flush(cx)
  }

  fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
    if std::thread::current().id() != self.thread_id {
      panic!("SendAsyncIo can only be used from the same thread it was created on");
    }
    Pin::new(&mut self.inner).poll_shutdown(cx)
  }

  fn poll_write_vectored(
    mut self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    bufs: &[std::io::IoSlice<'_>],
  ) -> Poll<Result<usize, std::io::Error>> {
    if std::thread::current().id() != self.thread_id {
      panic!("SendAsyncIo can only be used from the same thread it was created on");
    }
    Pin::new(&mut self.inner).poll_write_vectored(cx, bufs)
  }

  fn is_write_vectored(&self) -> bool {
    if std::thread::current().id() != self.thread_id {
      panic!("SendAsyncIo can only be used from the same thread it was created on");
    }
    self.inner.is_write_vectored()
  }
}

impl<T> Drop for SendAsyncIo<T> {
  fn drop(&mut self) {
    if std::thread::current().id() != self.thread_id {
      panic!("SendAsyncIo can only be used from the same thread it was created on");
    }
  }
}

// Safety: SendAsyncIo would panic if used from a different thread, instead of having undefined behavior.
unsafe impl<T> Send for SendAsyncIo<T> {}
