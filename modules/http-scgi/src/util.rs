use tokio::io::{AsyncRead, AsyncWrite};

use pin_project_lite::pin_project;

pin_project! {
    #[project = ConnectedSocketProj]
    pub enum ConnectedSocket {
        Tcp {
            #[pin]
            socket: vibeio::net::PollTcpStream,
        },
        #[cfg(unix)]
        Unix {
            #[pin]
            socket: vibeio::net::PollUnixStream,
        },
    }
}

impl ConnectedSocket {
    pub async fn connect_tcp(addr: &str) -> Result<Self, std::io::Error> {
        let socket = vibeio::net::TcpStream::connect(addr).await?;
        socket.set_nodelay(true)?;
        Ok(Self::Tcp {
            socket: socket.into_poll()?,
        })
    }

    #[allow(dead_code)]
    #[cfg(unix)]
    pub async fn connect_unix(path: &str) -> Result<Self, std::io::Error> {
        Ok(Self::Unix {
            socket: vibeio::net::UnixStream::connect(path).await?.into_poll()?,
        })
    }

    #[allow(dead_code)]
    #[cfg(not(unix))]
    pub async fn connect_unix(_path: &str) -> Result<Self, std::io::Error> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Unix sockets are not supports on non-Unix platforms.",
        ))
    }
}

impl AsyncRead for ConnectedSocket {
    #[inline]
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.project() {
            ConnectedSocketProj::Tcp { socket } => socket.poll_read(cx, buf),
            #[cfg(unix)]
            ConnectedSocketProj::Unix { socket } => socket.poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for ConnectedSocket {
    #[inline]
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        match self.project() {
            ConnectedSocketProj::Tcp { socket } => socket.poll_write(cx, buf),
            #[cfg(unix)]
            ConnectedSocketProj::Unix { socket } => socket.poll_write(cx, buf),
        }
    }

    #[inline]
    fn poll_write_vectored(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> std::task::Poll<std::io::Result<usize>> {
        match self.project() {
            ConnectedSocketProj::Tcp { socket } => socket.poll_write_vectored(cx, bufs),
            #[cfg(unix)]
            ConnectedSocketProj::Unix { socket } => socket.poll_write_vectored(cx, bufs),
        }
    }

    #[inline]
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.project() {
            ConnectedSocketProj::Tcp { socket } => socket.poll_flush(cx),
            #[cfg(unix)]
            ConnectedSocketProj::Unix { socket } => socket.poll_flush(cx),
        }
    }

    #[inline]
    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.project() {
            ConnectedSocketProj::Tcp { socket } => socket.poll_shutdown(cx),
            #[cfg(unix)]
            ConnectedSocketProj::Unix { socket } => socket.poll_shutdown(cx),
        }
    }

    #[inline]
    fn is_write_vectored(&self) -> bool {
        match self {
            ConnectedSocket::Tcp { socket } => socket.is_write_vectored(),
            #[cfg(unix)]
            ConnectedSocket::Unix { socket } => socket.is_write_vectored(),
        }
    }
}

pub struct SendWrapBody<B> {
    inner: send_wrapper::SendWrapper<std::pin::Pin<Box<B>>>,
}

impl<B> SendWrapBody<B> {
    #[inline]
    pub fn new(inner: B) -> Self {
        Self {
            inner: send_wrapper::SendWrapper::new(Box::pin(inner)),
        }
    }
}

impl<B> http_body::Body for SendWrapBody<B>
where
    B: http_body::Body + 'static,
{
    type Data = B::Data;
    type Error = B::Error;

    #[inline]
    fn poll_frame(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        self.inner.as_mut().poll_frame(cx)
    }
}
