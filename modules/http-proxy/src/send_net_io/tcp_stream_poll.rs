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
use vibeio::net::TcpStream;

/// A wrapper around vibeio's `PollTcpStream` that implements
/// `tokio::io::AsyncRead + AsyncWrite + Send` for use with hyper's client API.
///
/// This wrapper handles cross-thread safety by reconstructing the stream
/// from a raw file descriptor when moved between threads.
pub struct SendTcpStreamPoll {
    thread_id: ThreadId,
    inner: Option<PollTcpStream>,
    prev_inner: Option<ManuallyDrop<PollTcpStream>>,
    is_write_vectored: bool,
    #[cfg(unix)]
    inner_fd: RawFd,
    #[cfg(windows)]
    inner_fd: RawSocket,
    obtained_dropped: bool,
    marked_dropped: Arc<AtomicBool>,
}

impl SendTcpStreamPoll {
    /// Creates a new wrapper from a vibeio `TcpStream` by converting it to poll mode.
    #[inline]
    pub fn new_comp_io(inner: TcpStream) -> Result<Self, std::io::Error> {
        #[cfg(unix)]
        let inner_fd = inner.as_raw_fd();
        #[cfg(not(unix))]
        let inner_fd = inner.as_raw_socket();
        let inner = inner.into_poll()?;
        let is_write_vectored = inner.is_write_vectored();
        Ok(SendTcpStreamPoll {
            thread_id: std::thread::current().id(),
            inner: Some(inner),
            prev_inner: None,
            is_write_vectored,
            #[cfg(unix)]
            inner_fd,
            obtained_dropped: false,
            marked_dropped: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Obtains a drop guard for the inner `PollTcpStream`.
    ///
    /// # Safety
    ///
    /// This method is unsafe because it allows the caller to drop the inner
    /// `PollTcpStream` without marking it as dropped. The drop guard must be
    /// used exactly once.
    #[inline]
    pub unsafe fn get_drop_guard(&mut self) -> SendTcpStreamPollDropGuard {
        if self.obtained_dropped {
            panic!("the TcpStreamPoll's get_drop_guard method can be used only once");
        }
        self.obtained_dropped = true;
        let inner = if let Some(inner) = self.inner.as_ref() {
            // Copy the inner TcpStreamPoll
            let mut inner_data = std::mem::MaybeUninit::uninit();
            std::ptr::copy(inner as *const _, inner_data.as_mut_ptr(), 1);
            Some(ManuallyDrop::new(inner_data.assume_init()))
        } else {
            None
        };
        SendTcpStreamPollDropGuard {
            inner,
            marked_dropped: self.marked_dropped.clone(),
        }
    }

    #[inline]
    fn populate_if_different_thread_or_marked_dropped(&mut self, dropped: bool) {
        let current_thread_id = std::thread::current().id();
        let marked_dropped = !dropped
            && self.marked_dropped.swap(false, Ordering::Relaxed)
            && self.prev_inner.is_none();
        if marked_dropped || current_thread_id != self.thread_id {
            if !self.obtained_dropped {
                panic!("the TcpStreamPoll can be used only once if drop guard is not obtained")
            }
            if self.prev_inner.is_some() {
                panic!("the TcpStreamPoll can be moved only once across threads or if it is marked as dropped");
            }

            // Safety: The inner TcpStreamPoll is manually dropped, so it's safe to use the raw fd
            #[cfg(unix)]
            let std_tcp_stream = unsafe { std::net::TcpStream::from_raw_fd(self.inner_fd) };
            #[cfg(windows)]
            let std_tcp_stream = unsafe { std::net::TcpStream::from_raw_socket(self.inner_fd) };
            let _ = std_tcp_stream.set_nonblocking(true);

            let tcp_stream_poll = TcpStream::from_std(std_tcp_stream)
                .expect("failed to create TcpStream")
                .into_poll()
                .expect("failed to create TcpStreamPoll");
            self.is_write_vectored = tcp_stream_poll.is_write_vectored();
            self.prev_inner = self.inner.take().map(ManuallyDrop::new);
            self.inner = Some(tcp_stream_poll);
            self.thread_id = current_thread_id;
        }
    }
}

impl AsyncRead for SendTcpStreamPoll {
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

impl AsyncWrite for SendTcpStreamPoll {
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

#[cfg(not(unix))]
impl AsRawSocket for SendTcpStreamPoll {
    #[inline]
    fn as_raw_socket(&self) -> RawSocket {
        self.inner_socket
    }
}

#[cfg(not(unix))]
impl AsSocket for SendTcpStreamPoll {
    #[inline]
    fn as_socket(&self) -> BorrowedSocket<'_> {
        // Safety: inner_fd is valid, as it is taken from the inner value
        unsafe { BorrowedSocket::borrow_raw(self.inner_fd) }
    }
}

impl Drop for SendTcpStreamPoll {
    fn drop(&mut self) {
        if !self.marked_dropped.swap(true, Ordering::Relaxed) {
            self.populate_if_different_thread_or_marked_dropped(true);
        } else {
            let _ = ManuallyDrop::new(self.inner.take());
        }
    }
}

// Safety: vibeio's internal Rc in InnerRawFd is only cloned during async operations.
unsafe impl Send for SendTcpStreamPoll {}

/// Drop guard for `SendTcpStreamPoll`.
///
/// Ensures the inner stream is properly marked as dropped to prevent double-free
/// when the stream is returned to the connection pool.
pub struct SendTcpStreamPollDropGuard {
    inner: Option<ManuallyDrop<PollTcpStream>>,
    marked_dropped: Arc<AtomicBool>,
}

impl Drop for SendTcpStreamPollDropGuard {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.take() {
            if !self.marked_dropped.swap(true, Ordering::Relaxed) {
                // Drop if not marked as dropped
                #[cfg(unix)]
                let _ = ManuallyDrop::into_inner(inner)
                    .into_completion()
                    .map(|c| c.into_raw_fd());
                #[cfg(not(unix))]
                let _ = ManuallyDrop::into_inner(inner).into_completion();
            }
        }
    }
}
