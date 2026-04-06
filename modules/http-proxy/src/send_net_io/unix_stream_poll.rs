use std::mem::ManuallyDrop;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, RawFd};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::thread::ThreadId;

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use vibeio::net::PollUnixStream;
use vibeio::net::UnixStream;

/// A wrapper around vibeio's `PollUnixStream` that implements
/// `tokio::io::AsyncRead + AsyncWrite + Send` for use with hyper's client API.
pub struct SendUnixStreamPoll {
    thread_id: ThreadId,
    inner: Option<PollUnixStream>,
    prev_inner: Option<ManuallyDrop<PollUnixStream>>,
    is_write_vectored: bool,
    inner_fd: RawFd,
    obtained_dropped: bool,
    marked_dropped: Arc<AtomicBool>,
}

impl SendUnixStreamPoll {
    /// Creates a new wrapper from a vibeio `UnixStream` by converting it to poll mode.
    #[inline]
    pub fn new_comp_io(inner: UnixStream) -> Result<Self, std::io::Error> {
        let inner_fd = inner.as_raw_fd();
        let inner = inner.into_poll()?;
        let is_write_vectored = inner.is_write_vectored();
        Ok(SendUnixStreamPoll {
            thread_id: std::thread::current().id(),
            inner: Some(inner),
            prev_inner: None,
            is_write_vectored,
            inner_fd,
            obtained_dropped: false,
            marked_dropped: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Obtains a drop guard for the inner `PollUnixStream`.
    ///
    /// # Safety
    ///
    /// This method is unsafe because it allows the caller to drop the inner
    /// `PollUnixStream` without marking it as dropped.
    #[inline]
    pub unsafe fn get_drop_guard(&mut self) -> SendUnixStreamPollDropGuard {
        if self.obtained_dropped {
            panic!("the UnixStreamPoll's get_drop_guard method can be used only once");
        }
        self.obtained_dropped = true;
        let inner = if let Some(inner) = self.inner.as_ref() {
            // Copy the inner UnixStreamPoll
            let mut inner_data = std::mem::MaybeUninit::uninit();
            std::ptr::copy(inner as *const _, inner_data.as_mut_ptr(), 1);
            Some(ManuallyDrop::new(inner_data.assume_init()))
        } else {
            None
        };
        SendUnixStreamPollDropGuard {
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
                panic!("the UnixStreamPoll can be used only once if drop guard is not obtained")
            }
            if self.prev_inner.is_some() {
                panic!("the UnixStreamPoll can be moved only once across threads or if it is marked as dropped");
            }

            // Safety: The inner UnixStreamPoll is manually dropped, so it's safe to use the raw fd
            let std_unix_stream =
                unsafe { std::os::unix::net::UnixStream::from_raw_fd(self.inner_fd) };
            let _ = std_unix_stream.set_nonblocking(true);
            let unix_stream_poll = UnixStream::from_std(std_unix_stream)
                .expect("failed to create UnixStream")
                .into_poll()
                .expect("failed to create UnixStreamPoll");
            self.is_write_vectored = unix_stream_poll.is_write_vectored();
            self.prev_inner = self.inner.take().map(ManuallyDrop::new);
            self.inner = Some(unix_stream_poll);
            self.thread_id = current_thread_id;
        }
    }
}

impl AsyncRead for SendUnixStreamPoll {
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

impl AsyncWrite for SendUnixStreamPoll {
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
        if !self.marked_dropped.swap(true, Ordering::Relaxed) {
            self.populate_if_different_thread_or_marked_dropped(true);
        } else {
            let _ = ManuallyDrop::new(self.inner.take());
        }
    }
}

// Safety: vibeio's internal Rc in InnerRawFd is only cloned during async operations.
unsafe impl Send for SendUnixStreamPoll {}

/// Drop guard for `SendUnixStreamPoll`.
pub struct SendUnixStreamPollDropGuard {
    inner: Option<ManuallyDrop<PollUnixStream>>,
    marked_dropped: Arc<AtomicBool>,
}

impl Drop for SendUnixStreamPollDropGuard {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.take() {
            if !self.marked_dropped.swap(true, Ordering::Relaxed) {
                // Drop if not marked as dropped
                let _ = ManuallyDrop::into_inner(inner)
                    .into_completion()
                    .map(|c| c.into_raw_fd());
            }
        }
    }
}
