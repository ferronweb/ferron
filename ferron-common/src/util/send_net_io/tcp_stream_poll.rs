#[cfg(unix)]
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, RawFd};
#[cfg(windows)]
use std::os::windows::io::{AsRawSocket, AsSocket, BorrowedSocket, FromRawSocket, IntoRawSocket, RawSocket};
use std::pin::Pin;
use std::task::{Context, Poll};
use std::thread::ThreadId;

use monoio::io::{IntoCompIo, IntoPollIo};
use monoio::net::tcp::stream_poll::TcpStreamPoll;
use monoio::net::TcpStream;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// SendTcpStream is a wrapper around Monoio's TcpStream.
pub struct SendTcpStreamPoll {
  thread_id: ThreadId,
  inner: Option<TcpStreamPoll>,
  prev_inner: Option<TcpStreamPoll>,
  is_write_vectored: bool,
  #[cfg(unix)]
  inner_fd: RawFd,
  #[cfg(windows)]
  inner_socket: RawSocket,
}

impl SendTcpStreamPoll {
  /// Creates a new SendTcpStreamPoll wrapper around the given TcpStreamPoll.
  #[inline]
  pub fn new(inner: TcpStreamPoll) -> Self {
    #[cfg(unix)]
    let inner_fd = inner.as_raw_fd();
    #[cfg(windows)]
    let inner_socket = inner.as_raw_socket();
    let is_write_vectored = inner.is_write_vectored();
    SendTcpStreamPoll {
      thread_id: std::thread::current().id(),
      inner: Some(inner),
      prev_inner: None,
      is_write_vectored,
      #[cfg(unix)]
      inner_fd,
      #[cfg(windows)]
      inner_socket,
    }
  }

  /// Creates a new SendTcpStreamPoll wrapper around the given TcpStream.
  #[inline]
  pub fn new_comp_io(inner: TcpStream) -> Result<Self, std::io::Error> {
    #[cfg(unix)]
    let inner_fd = inner.as_raw_fd();
    #[cfg(windows)]
    let inner_socket = inner.as_raw_socket();
    let inner = inner.into_poll_io()?;
    let is_write_vectored = inner.is_write_vectored();
    Ok(SendTcpStreamPoll {
      thread_id: std::thread::current().id(),
      inner: Some(inner),
      prev_inner: None,
      is_write_vectored,
      #[cfg(unix)]
      inner_fd,
      #[cfg(windows)]
      inner_socket,
    })
  }
}

impl SendTcpStreamPoll {
  #[inline]
  fn populate_if_different_thread(&mut self) {
    let current_thread_id = std::thread::current().id();
    if current_thread_id != self.thread_id {
      if self.prev_inner.is_some() {
        panic!("the TcpStreamPoll can be moved across threads only once");
      }
      // Safety: The inner TcpStreamPoll is manually dropped, so it's safe to use the raw fd/socket
      #[cfg(unix)]
      let std_tcp_stream = unsafe { std::net::TcpStream::from_raw_fd(self.inner_fd) };
      #[cfg(windows)]
      let std_tcp_stream = unsafe { std::net::TcpStream::from_raw_socket(self.inner_socket) };
      let tcp_stream_poll_result = TcpStream::from_std(std_tcp_stream)
        .expect("failed to create TcpStream")
        .try_into_poll_io();
      let tcp_stream_poll = match tcp_stream_poll_result {
        Ok(stream) => stream,
        Err((_, stream)) => {
          if let Some(prev_stream) = self.inner.take() {
            let _ = prev_stream.into_comp_io();
          }
          stream.into_poll_io().expect("failed to create TcpStreamPoll")
        }
      };
      self.is_write_vectored = tcp_stream_poll.is_write_vectored();
      self.prev_inner = self.inner.take();
      self.inner = Some(tcp_stream_poll);
      self.thread_id = current_thread_id;
    }
  }
}

impl AsyncRead for SendTcpStreamPoll {
  #[inline]
  fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
    self.populate_if_different_thread();
    Pin::new(self.inner.as_mut().expect("inner element not present")).poll_read(cx, buf)
  }
}

impl AsyncWrite for SendTcpStreamPoll {
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

#[cfg(unix)]
impl AsRawFd for SendTcpStreamPoll {
  #[inline]
  fn as_raw_fd(&self) -> RawFd {
    self.inner_fd
  }
}

#[cfg(unix)]
impl AsFd for SendTcpStreamPoll {
  #[inline]
  fn as_fd(&self) -> BorrowedFd<'_> {
    // Safety: inner_fd is valid, as it is taken from the inner value
    unsafe { BorrowedFd::borrow_raw(self.inner_fd) }
  }
}

#[cfg(windows)]
impl AsRawSocket for SendTcpStreamPoll {
  #[inline]
  fn as_raw_socket(&self) -> RawSocket {
    self.inner_socket
  }
}

#[cfg(windows)]
impl AsSocket for SendTcpStreamPoll {
  #[inline]
  fn as_socket(&self) -> BorrowedSocket<'_> {
    // Safety: inner_socket is valid, as it is taken from the inner value
    unsafe { BorrowedSocket::borrow_raw(self.inner_socket) }
  }
}

impl Drop for SendTcpStreamPoll {
  fn drop(&mut self) {
    if let Some(prev_inner) = self.prev_inner.take() {
      let prev_inner_comp_io = prev_inner
        .try_into_comp_io()
        .expect("failed to convert inner TcpStreamPoll to comp_io");

      #[cfg(unix)]
      let _ = prev_inner_comp_io.into_raw_fd();
      #[cfg(windows)]
      let _ = prev_inner_comp_io.into_raw_socket();
    }
  }
}

// Safety: As far as we read from Monoio's source, inner Rc in SharedFd is cloned only during async operations.
unsafe impl Send for SendTcpStreamPoll {}
