// Copyright (c) 2018 The quinn Developers
// Portions of this file are derived from Quinn (https://github.com/quinn-rs/quinn).
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//

use std::error::Error;
#[cfg(feature = "runtime-monoio")]
use std::fmt::{Debug, Formatter};
#[cfg(feature = "runtime-monoio")]
use std::future::Future;
#[cfg(feature = "runtime-monoio")]
use std::io;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
#[cfg(feature = "runtime-monoio")]
use std::pin::Pin;
use std::sync::Arc;
#[cfg(feature = "runtime-monoio")]
use std::task::{ready, Context, Poll};
use std::time::Duration;
#[cfg(feature = "runtime-monoio")]
use std::time::Instant;

use async_channel::{Receiver, Sender};
#[cfg(feature = "runtime-monoio")]
use async_io::Async;
#[cfg(feature = "runtime-monoio")]
use pin_project_lite::pin_project;
use quinn::crypto::rustls::QuicServerConfig;
#[cfg(feature = "runtime-monoio")]
use quinn::{udp, AsyncTimer, AsyncUdpSocket, Runtime, UdpPoller};
use rustls::ServerConfig;

use crate::listener_handler_communication::{Connection, ConnectionData};
use crate::logging::LogMessage;

#[cfg(feature = "runtime-monoio")]
pin_project_lite::pin_project! {
    /// Helper adapting a function `MakeFut` that constructs a single-use future `Fut` into a
    /// [`UdpPoller`] that may be reused indefinitely
    struct UdpPollHelper<MakeFut, Fut> {
        make_fut: MakeFut,
        #[pin]
        fut: Option<Fut>,
    }
}

#[cfg(feature = "runtime-monoio")]
impl<MakeFut, Fut> UdpPollHelper<MakeFut, Fut> {
  /// Construct a [`UdpPoller`] that calls `make_fut` to get the future to poll, storing it until
  /// it yields [`Poll::Ready`], then creating a new one on the next
  /// [`poll_writable`](UdpPoller::poll_writable)
  fn new(make_fut: MakeFut) -> Self {
    Self {
      make_fut,
      fut: None,
    }
  }
}

#[cfg(feature = "runtime-monoio")]
impl<MakeFut, Fut> UdpPoller for UdpPollHelper<MakeFut, Fut>
where
  MakeFut: Fn() -> Fut + Send + Sync + 'static,
  Fut: Future<Output = io::Result<()>> + Send + Sync + 'static,
{
  fn poll_writable(self: Pin<&mut Self>, cx: &mut Context) -> Poll<io::Result<()>> {
    let mut this = self.project();
    if this.fut.is_none() {
      this.fut.set(Some((this.make_fut)()));
    }
    // We're forced to `unwrap` here because `Fut` may be `!Unpin`, which means we can't safely
    // obtain an `&mut Fut` after storing it in `self.fut` when `self` is already behind `Pin`,
    // and if we didn't store it then we wouldn't be able to keep it alive between
    // `poll_writable` calls.
    let result = this.fut.as_mut().as_pin_mut().unwrap().poll(cx);
    if result.is_ready() {
      // Polling an arbitrary `Future` after it becomes ready is a logic error, so arrange for
      // a new `Future` to be created on the next call.
      this.fut.set(None);
    }
    result
  }
}

#[cfg(feature = "runtime-monoio")]
impl<MakeFut, Fut> Debug for UdpPollHelper<MakeFut, Fut> {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("UdpPollHelper").finish_non_exhaustive()
  }
}

/// A runtime for Quinn that utilizes Monoio and async_io
#[derive(Debug)]
#[cfg(feature = "runtime-monoio")]
struct MonoioAsyncioRuntime;

#[cfg(feature = "runtime-monoio")]
impl Runtime for MonoioAsyncioRuntime {
  fn new_timer(&self, t: Instant) -> Pin<Box<dyn AsyncTimer>> {
    Box::pin(Timer {
      inner: async_io::Timer::at(t),
    })
  }

  fn spawn(&self, future: Pin<Box<dyn Future<Output = ()> + Send>>) {
    monoio::spawn(future);
  }

  fn wrap_udp_socket(&self, sock: std::net::UdpSocket) -> io::Result<Arc<dyn AsyncUdpSocket>> {
    Ok(Arc::new(UdpSocket::new(sock)?))
  }
}

#[cfg(feature = "runtime-monoio")]
pin_project! {
    struct Timer {
        #[pin]
        inner: async_io::Timer
    }
}

#[cfg(feature = "runtime-monoio")]
impl AsyncTimer for Timer {
  fn reset(mut self: Pin<&mut Self>, t: Instant) {
    self.inner.set_at(t)
  }

  fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<()> {
    Future::poll(self.project().inner, cx).map(|_| ())
  }
}

#[cfg(feature = "runtime-monoio")]
impl Debug for Timer {
  fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
    self.inner.fmt(f)
  }
}

#[cfg(feature = "runtime-monoio")]
#[derive(Debug)]
struct UdpSocket {
  io: Async<std::net::UdpSocket>,
  inner: udp::UdpSocketState,
}

#[cfg(feature = "runtime-monoio")]
impl UdpSocket {
  fn new(sock: std::net::UdpSocket) -> io::Result<Self> {
    Ok(Self {
      inner: udp::UdpSocketState::new((&sock).into())?,
      io: Async::new_nonblocking(sock)?,
    })
  }
}

#[cfg(feature = "runtime-monoio")]
impl AsyncUdpSocket for UdpSocket {
  fn create_io_poller(self: Arc<Self>) -> Pin<Box<dyn UdpPoller>> {
    Box::pin(UdpPollHelper::new(move || {
      let socket = self.clone();
      async move { socket.io.writable().await }
    }))
  }

  fn try_send(&self, transmit: &udp::Transmit) -> io::Result<()> {
    self.inner.send((&self.io).into(), transmit)
  }

  fn poll_recv(
    &self,
    cx: &mut Context,
    bufs: &mut [io::IoSliceMut<'_>],
    meta: &mut [udp::RecvMeta],
  ) -> Poll<io::Result<usize>> {
    loop {
      ready!(self.io.poll_readable(cx))?;
      if let Ok(res) = self.inner.recv((&self.io).into(), bufs, meta) {
        return Poll::Ready(Ok(res));
      }
    }
  }

  fn local_addr(&self) -> io::Result<std::net::SocketAddr> {
    self.io.as_ref().local_addr()
  }

  fn may_fragment(&self) -> bool {
    self.inner.may_fragment()
  }

  fn max_transmit_segments(&self) -> usize {
    self.inner.max_gso_segments()
  }

  fn max_receive_segments(&self) -> usize {
    self.inner.gro_segments()
  }
}

/// Creates a QUIC listener
#[allow(clippy::type_complexity)]
pub fn create_quic_listener(
  address: SocketAddr,
  tls_config: Arc<ServerConfig>,
  tx: Sender<ConnectionData>,
  enable_uring: bool,
  logging_tx: Sender<LogMessage>,
  first_startup: bool,
) -> Result<(Sender<()>, Sender<Arc<ServerConfig>>), Box<dyn Error + Send + Sync>> {
  let (shutdown_tx, shutdown_rx) = async_channel::unbounded();
  let (rustls_config_tx, rustls_config_rx) = async_channel::unbounded();
  let (listen_error_tx, listen_error_rx) = async_channel::unbounded();
  std::thread::Builder::new()
    .name(format!("QUIC listener for {}", address))
    .spawn(move || {
      crate::runtime::new_runtime(
        async move {
          if let Err(error) = quic_listener_fn(
            address,
            tls_config,
            tx,
            &listen_error_tx,
            logging_tx,
            first_startup,
            shutdown_rx,
            rustls_config_rx,
          )
          .await
          {
            listen_error_tx.send(Some(error)).await.unwrap_or_default();
          }
        },
        enable_uring,
      )
      .unwrap();
    })?;

  if let Some(error) = listen_error_rx.recv_blocking()? {
    Err(error)?;
  }

  Ok((shutdown_tx, rustls_config_tx))
}

/// QUIC listener function
#[allow(clippy::too_many_arguments)]
async fn quic_listener_fn(
  address: SocketAddr,
  tls_config: Arc<ServerConfig>,
  tx: Sender<ConnectionData>,
  listen_error_tx: &Sender<Option<Box<dyn Error + Send + Sync>>>,
  logging_tx: Sender<LogMessage>,
  first_startup: bool,
  shutdown_rx: Receiver<()>,
  rustls_config_rx: Receiver<Arc<ServerConfig>>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
  let quic_server_config = Arc::new(match QuicServerConfig::try_from(tls_config) {
    Ok(config) => config,
    Err(err) => Err(anyhow::anyhow!(format!(
      "Cannot prepare the QUIC server configuration: {}",
      err
    )))?,
  });
  let server_config = quinn::ServerConfig::with_crypto(quic_server_config);
  let udp_port = address.port();
  let mut udp_socket_result;
  let mut tries: u64 = 0;
  loop {
    udp_socket_result = std::net::UdpSocket::bind(address);
    if first_startup || udp_socket_result.is_ok() {
      break;
    }
    tries += 1;
    let duration = Duration::from_millis(1000);
    if tries >= 10 {
      println!("HTTP/3 port is used at try #{}, skipping...", tries);
      listen_error_tx.send(None).await.unwrap_or_default();
      break;
    }
    println!(
      "HTTP/3 port is used at try #{}, retrying in {:?}...",
      tries, duration
    );
    if shutdown_rx.try_recv().is_ok() {
      break;
    }
    crate::runtime::sleep(duration).await;
  }
  let udp_socket = match udp_socket_result {
    Ok(socket) => socket,
    Err(err) => Err(anyhow::anyhow!(format!(
      "Cannot listen to HTTP/3 port: {}",
      err
    )))?,
  };
  let endpoint = match quinn::Endpoint::new(
    quinn::EndpointConfig::default(),
    Some(server_config),
    udp_socket,
    {
      #[cfg(feature = "runtime-monoio")]
      let runtime = Arc::new(MonoioAsyncioRuntime);
      #[cfg(feature = "runtime-tokio")]
      let runtime = Arc::new(quinn::TokioRuntime);

      runtime
    },
  ) {
    Ok(endpoint) => endpoint,
    Err(err) => Err(anyhow::anyhow!(format!(
      "Cannot listen to HTTP/3 port: {}",
      err
    )))?,
  };
  println!("HTTP/3 server is listening on {}...", address);
  listen_error_tx.send(None).await.unwrap_or_default();

  loop {
    let rustls_receive_future = async {
      if let Ok(rustls_server_config) = rustls_config_rx.recv().await {
        rustls_server_config
      } else {
        futures_util::future::pending().await
      }
    };

    let new_conn = crate::runtime::select! {
      result = endpoint.accept() => {
          match result {
              Some(conn) => conn,
              None => {
                  logging_tx
                      .send(LogMessage::new(
                          "HTTP/3 connections can't be accepted anymore".to_string(),
                          true,
                      ))
                      .await
                      .unwrap_or_default();
                  break;
              }
          }
      }
      tls_config = rustls_receive_future => {
          let quic_server_config = Arc::new(match QuicServerConfig::try_from(tls_config) {
              Ok(config) => config,
              Err(_) => continue,
          });
          let server_config = quinn::ServerConfig::with_crypto(quic_server_config);
          endpoint.set_server_config(Some(server_config));
          continue;
      }
      _ = shutdown_rx.recv() => {
          break;
      }
    };
    let remote_address = new_conn.remote_address();
    let local_address = SocketAddr::new(
      new_conn
        .local_ip()
        .unwrap_or(IpAddr::V6(Ipv6Addr::UNSPECIFIED)),
      udp_port,
    );
    let quic_data = ConnectionData {
      connection: Connection::Quic(new_conn),
      client_address: remote_address,
      server_address: local_address,
    };
    let quic_tx = tx.clone();
    crate::runtime::spawn(async move {
      quic_tx.send(quic_data).await.unwrap_or_default();
    });
  }

  endpoint.wait_idle().await;

  Ok(())
}
