use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, RawFd};
use std::pin::Pin;
use std::task::{Context, Poll};
use std::thread::ThreadId;

use monoio::io::{IntoCompIo, IntoPollIo};
use monoio::net::unix::stream_poll::UnixStreamPoll;
use monoio::net::UnixStream;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// SendUnixStream is a wrapper around Monoio's UnixStream.
pub struct SendUnixStreamPoll {
  thread_id: ThreadId,
  inner: Option<UnixStreamPoll>,
  prev_inner: Option<UnixStreamPoll>,
  is_write_vectored: bool,
  inner_fd: RawFd,
}

impl SendUnixStreamPoll {
  /// Creates a new SendUnixStreamPoll wrapper around the given UnixStream.
  #[inline]
  pub fn new_comp_io(inner: UnixStream) -> Result<Self, std::io::Error> {
    let inner_fd = inner.as_raw_fd();
    let inner = inner.into_poll_io()?;
    let is_write_vectored = inner.is_write_vectored();
    Ok(SendUnixStreamPoll {
      thread_id: std::thread::current().id(),
      inner: Some(inner),
      prev_inner: None,
      is_write_vectored,
      inner_fd,
    })
  }
}

impl SendUnixStreamPoll {
  #[inline]
  fn populate_if_different_thread(&mut self) {
    let current_thread_id = std::thread::current().id();
    if current_thread_id != self.thread_id {
      if self.prev_inner.is_some() {
        panic!("the UnixStreamPoll can be moved across threads only once");
      }
      // Safety: The inner UnixStreamPoll is manually dropped, so it's safe to use the raw fd/socket
      let std_tcp_stream = unsafe { std::os::unix::net::UnixStream::from_raw_fd(self.inner_fd) };
      let tcp_stream_poll_result = UnixStream::from_std(std_tcp_stream)
        .expect("failed to create UnixStream")
        .try_into_poll_io();
      let tcp_stream_poll = match tcp_stream_poll_result {
        Ok(stream) => stream,
        Err((_, stream)) => {
          if let Some(prev_stream) = self.inner.take() {
            let _ = prev_stream.into_comp_io();
          }
          stream.into_poll_io().expect("failed to create UnixStreamPoll")
        }
      };
      self.is_write_vectored = tcp_stream_poll.is_write_vectored();
      self.prev_inner = self.inner.take();
      self.inner = Some(tcp_stream_poll);
      self.thread_id = current_thread_id;
    }
  }
}

impl AsyncRead for SendUnixStreamPoll {
  #[inline]
  fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
    self.populate_if_different_thread();
    Pin::new(self.inner.as_mut().expect("inner element not present")).poll_read(cx, buf)
  }
}

impl AsyncWrite for SendUnixStreamPoll {
  #[inline]
  fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
    self.populate_if_different_thread();
    Pin::new(self.inner.as_mut().expect("inner element not present")).poll_write(cx, buf)
  }

  #[inline]
  fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
    self.populate_if_different_thread();
    Pin::new(self.inner.as_mut().expect("inner element not present")).poll_flush(cx)
  }

  #[inline]
  fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
    self.populate_if_different_thread();
    Pin::new(self.inner.as_mut().expect("inner element not present")).poll_shutdown(cx)
  }

  #[inline]
  fn poll_write_vectored(
    mut self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    bufs: &[std::io::IoSlice<'_>],
  ) -> Poll<Result<usize, std::io::Error>> {
    self.populate_if_different_thread();
    Pin::new(self.inner.as_mut().expect("inner element not present")).poll_write_vectored(cx, bufs)
  }

  #[inline]
  fn is_write_vectored(&self) -> bool {
    if std::thread::current().id() != self.thread_id {
      return self.is_write_vectored;
    }
    self
      .inner
      .as_ref()
      .expect("inner element not present")
      .is_write_vectored()
  }
}

impl AsRawFd for SendUnixStreamPoll {
  #[inline]
  fn as_raw_fd(&self) -> RawFd {
    self.inner_fd
  }
}

impl AsFd for SendUnixStreamPoll {
  #[inline]
  fn as_fd(&self) -> BorrowedFd<'_> {
    // Safety: inner_fd is valid, as it is taken from the inner value
    unsafe { BorrowedFd::borrow_raw(self.inner_fd) }
  }
}

impl Drop for SendUnixStreamPoll {
  fn drop(&mut self) {
    if let Some(prev_inner) = self.prev_inner.take() {
      let prev_inner_comp_io = prev_inner
        .try_into_comp_io()
        .expect("failed to convert inner UnixStreamPoll to comp_io");

      let _ = prev_inner_comp_io.into_raw_fd();
    }
  }
}

// Safety: As far as we read from Monoio's source, inner Rc in SharedFd is cloned only during async operations.
unsafe impl Send for SendUnixStreamPoll {}
