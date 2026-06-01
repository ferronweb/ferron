use std::mem::ManuallyDrop;
#[cfg(unix)]
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, RawFd};
#[cfg(windows)]
use std::os::windows::io::{
    AsRawSocket, AsSocket, BorrowedSocket, FromRawSocket, IntoRawSocket, RawSocket,
};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::thread::ThreadId;

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use vibeio::net::PollTcpStream;
#[cfg(unix)]
use vibeio::net::PollUnixStream;

/// A wrapper around a `PollTcpStream` that supports cross-thread safety.
pub type SendTcpStreamPoll = SendStreamPoll<PollTcpStream>;
/// A guard that ensures the inner stream is properly marked as dropped when dropped.
pub type SendTcpStreamPollDropGuard = SendStreamPollDropGuard<PollTcpStream>;

/// A wrapper around a `PollUnixStream` that supports cross-thread safety.
#[cfg(unix)]
pub type SendUnixStreamPoll = SendStreamPoll<PollUnixStream>;
/// A guard that ensures the inner stream is properly marked as dropped when dropped.
#[cfg(unix)]
pub type SendUnixStreamPollDropGuard = SendStreamPollDropGuard<PollUnixStream>;

/// A trait that allows a stream to be wrapped in a `SendStreamPoll`.
#[cfg(unix)]
trait SendableStreamPoll: Sized + AsyncRead + AsyncWrite + IntoRawFd + AsRawFd + Unpin {
    /// Creates a `SendableStreamPoll` from a raw file descriptor.
    unsafe fn from_raw_fd(fd: RawFd) -> std::io::Result<Self>;
}

/// A trait that allows a stream to be wrapped in a `SendStreamPoll`.
#[cfg(windows)]
trait SendableStreamPoll: Sized + AsyncRead + AsyncWrite + IntoRawSocket + AsRawSocket + Unpin {
    /// Creates a `SendableStreamPoll` from a raw socket.
    unsafe fn from_raw_socket(fd: RawSocket) -> std::io::Result<Self>;
}

impl SendableStreamPoll for PollTcpStream {
    #[cfg(unix)]
    unsafe fn from_raw_fd(fd: RawFd) -> std::io::Result<Self> {
        let std_tcp_stream = unsafe { std::net::TcpStream::from_raw_fd(fd) };
        let _ = std_tcp_stream.set_nonblocking(true);
        PollTcpStream::from_std(std_tcp_stream)
    }
    #[cfg(windows)]
    unsafe fn from_raw_socket(fd: RawSocket) -> std::io::Result<Self> {
        let std_tcp_stream = unsafe { std::net::TcpStream::from_raw_socket(fd) };
        let _ = std_tcp_stream.set_nonblocking(true);
        PollTcpStream::from_std(std_tcp_stream)
    }
}

#[cfg(unix)]
impl SendableStreamPoll for PollUnixStream {
    unsafe fn from_raw_fd(fd: RawFd) -> std::io::Result<Self> {
        let std_unix_stream = unsafe { std::os::unix::net::UnixStream::from_raw_fd(fd) };
        let _ = std_unix_stream.set_nonblocking(true);
        PollUnixStream::from_std(std_unix_stream)
    }
}

/// A wrapper around vibeio's poll-based stream that implements
/// `tokio::io::AsyncRead + AsyncWrite + Send` for use with hyper's client API.
///
/// This wrapper handles cross-thread safety by reconstructing the stream
/// from a raw file descriptor when moved between threads.
#[allow(private_bounds)]
pub struct SendStreamPoll<S: SendableStreamPoll> {
    thread_id: ThreadId,
    inner: Option<S>,
    prev_inner: Option<ManuallyDrop<S>>,
    is_write_vectored: bool,
    #[cfg(unix)]
    inner_fd: RawFd,
    #[cfg(windows)]
    inner_fd: RawSocket,
    obtained_dropped: bool,
    marked_dropped: Arc<AtomicBool>,
}

#[allow(private_bounds)]
impl<S: SendableStreamPoll> SendStreamPoll<S> {
    /// Creates a new wrapper from a vibeio poll-based stream.
    #[inline]
    pub fn new(inner: S) -> Self {
        #[cfg(unix)]
        let inner_fd = inner.as_raw_fd();
        #[cfg(not(unix))]
        let inner_fd = inner.as_raw_socket();
        let is_write_vectored = inner.is_write_vectored();
        Self {
            thread_id: std::thread::current().id(),
            inner: Some(inner),
            prev_inner: None,
            is_write_vectored,
            inner_fd,
            obtained_dropped: false,
            marked_dropped: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Obtains a drop guard for the inner stream.
    ///
    /// # Safety
    ///
    /// This method is unsafe because it allows the caller to drop the inner
    /// stream without marking it as dropped. The drop guard must be
    /// used exactly once.
    #[inline]
    pub unsafe fn get_drop_guard(&mut self) -> SendStreamPollDropGuard<S> {
        if self.obtained_dropped {
            panic!("the SendStreamPoll's get_drop_guard method can be used only once");
        }
        self.obtained_dropped = true;

        // Move the current inner stream into the guard safely instead of doing a raw
        // memory copy (which is undefined behaviour for non-Copy types). Recreate
        // a replacement stream from the stored raw fd so the wrapper remains usable.
        if let Some(inner_val) = self.inner.take() {
            let guard_inner = ManuallyDrop::new(inner_val);

            // Attempt to create a replacement stream from the raw fd/socket.
            #[cfg(unix)]
            let replacement = match unsafe { S::from_raw_fd(self.inner_fd) } {
                Ok(s) => s,
                Err(e) => {
                    // Restore original inner to preserve invariants and panic consistently
                    self.inner = Some(ManuallyDrop::into_inner(guard_inner));
                    panic!("failed to create SendStreamPoll: {}", e);
                }
            };
            #[cfg(not(unix))]
            let replacement = match unsafe { S::from_raw_socket(self.inner_fd) } {
                Ok(s) => s,
                Err(e) => {
                    self.inner = Some(ManuallyDrop::into_inner(guard_inner));
                    panic!("failed to create SendStreamPoll: {}", e);
                }
            };

            self.inner = Some(replacement);
            SendStreamPollDropGuard {
                inner: Some(guard_inner),
                marked_dropped: self.marked_dropped.clone(),
            }
        } else {
            SendStreamPollDropGuard {
                inner: None,
                marked_dropped: self.marked_dropped.clone(),
            }
        }
    }

    #[inline]
    fn populate_if_different_thread_or_marked_dropped(&mut self, dropped: bool) {
        let current_thread_id = std::thread::current().id();
        // Avoid unconditional atomic swap on the hot path. First check whether the
        // previous-inner state makes it worthwhile to probe the atomic flag, then
        // only clear it if we observed it set. This reduces atomic writes when
        // the flag is not set (common case).
        let marked_dropped = if !dropped && self.prev_inner.is_none() {
            if self.marked_dropped.load(Ordering::Relaxed) {
                self.marked_dropped.swap(false, Ordering::Relaxed)
            } else {
                false
            }
        } else {
            false
        };

        if marked_dropped || current_thread_id != self.thread_id {
            if !self.obtained_dropped {
                panic!("the SendStreamPoll can be used only once if drop guard is not obtained")
            }
            if self.prev_inner.is_some() {
                panic!("the SendStreamPoll can be moved only once across threads or if it is marked as dropped");
            }

            // Safety: The inner stream is manually dropped, so it's safe to use the raw fd
            #[cfg(unix)]
            let send_stream_poll =
                unsafe { S::from_raw_fd(self.inner_fd) }.expect("failed to create SendStreamPoll");
            #[cfg(windows)]
            let send_stream_poll = unsafe { S::from_raw_socket(self.inner_fd) }
                .expect("failed to create SendStreamPoll");
            self.is_write_vectored = send_stream_poll.is_write_vectored();
            self.prev_inner = self.inner.take().map(ManuallyDrop::new);
            self.inner = Some(send_stream_poll);
            self.thread_id = current_thread_id;
        }
    }
}

impl<S: SendableStreamPoll> AsyncRead for SendStreamPoll<S> {
    #[inline]
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.populate_if_different_thread_or_marked_dropped(false);
        Pin::new(self.inner.as_mut().expect("inner element not present")).poll_read(cx, buf)
    }
}

impl<S: SendableStreamPoll> AsyncWrite for SendStreamPoll<S> {
    #[inline]
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        self.populate_if_different_thread_or_marked_dropped(false);
        Pin::new(self.inner.as_mut().expect("inner element not present")).poll_write(cx, buf)
    }

    #[inline]
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        self.populate_if_different_thread_or_marked_dropped(false);
        Pin::new(self.inner.as_mut().expect("inner element not present")).poll_flush(cx)
    }

    #[inline]
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        self.populate_if_different_thread_or_marked_dropped(false);
        Pin::new(self.inner.as_mut().expect("inner element not present")).poll_shutdown(cx)
    }

    #[inline]
    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> Poll<Result<usize, std::io::Error>> {
        self.populate_if_different_thread_or_marked_dropped(false);
        Pin::new(self.inner.as_mut().expect("inner element not present"))
            .poll_write_vectored(cx, bufs)
    }

    #[inline]
    fn is_write_vectored(&self) -> bool {
        if std::thread::current().id() != self.thread_id {
            return self.is_write_vectored;
        }
        self.inner
            .as_ref()
            .expect("inner element not present")
            .is_write_vectored()
    }
}

#[cfg(unix)]
impl<S: SendableStreamPoll> AsRawFd for SendStreamPoll<S> {
    #[inline]
    fn as_raw_fd(&self) -> RawFd {
        self.inner_fd
    }
}

#[cfg(unix)]
impl<S: SendableStreamPoll> AsFd for SendStreamPoll<S> {
    #[inline]
    fn as_fd(&self) -> BorrowedFd<'_> {
        // Safety: inner_fd is valid, as it is taken from the inner value
        unsafe { BorrowedFd::borrow_raw(self.inner_fd) }
    }
}

#[cfg(not(unix))]
impl<S: SendableStreamPoll> AsRawSocket for SendStreamPoll<S> {
    #[inline]
    fn as_raw_socket(&self) -> RawSocket {
        self.inner_fd
    }
}

#[cfg(not(unix))]
impl<S: SendableStreamPoll> AsSocket for SendStreamPoll<S> {
    #[inline]
    fn as_socket(&self) -> BorrowedSocket<'_> {
        // Safety: inner_fd is valid, as it is taken from the inner value
        unsafe { BorrowedSocket::borrow_raw(self.inner_fd) }
    }
}

impl<S: SendableStreamPoll> Drop for SendStreamPoll<S> {
    #[inline]
    fn drop(&mut self) {
        if !self.marked_dropped.swap(true, Ordering::Relaxed) {
            self.populate_if_different_thread_or_marked_dropped(true);
        } else {
            let _ = ManuallyDrop::new(self.inner.take());
        }
    }
}

// Safety: vibeio's internal Rc in InnerRawFd is only cloned during async operations.
unsafe impl<S: SendableStreamPoll> Send for SendStreamPoll<S> {}

/// Drop guard for `SendStreamPoll`.
///
/// Ensures the inner stream is properly marked as dropped to prevent double-free
/// when the stream is returned to the connection pool.
#[allow(private_bounds)]
pub struct SendStreamPollDropGuard<S: SendableStreamPoll> {
    inner: Option<ManuallyDrop<S>>,
    marked_dropped: Arc<AtomicBool>,
}

impl<S: SendableStreamPoll> Drop for SendStreamPollDropGuard<S> {
    #[inline]
    fn drop(&mut self) {
        if let Some(inner) = self.inner.take() {
            if !self.marked_dropped.swap(true, Ordering::Relaxed) {
                // Drop if not marked as dropped
                #[cfg(unix)]
                let _ = ManuallyDrop::into_inner(inner).into_raw_fd();
                #[cfg(not(unix))]
                let _ = ManuallyDrop::into_inner(inner).into_raw_socket();
            }
        }
    }
}
