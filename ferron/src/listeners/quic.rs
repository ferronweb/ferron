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
use std::task::{Context, Poll};
use std::time::Duration;
#[cfg(feature = "runtime-monoio")]
use std::time::Instant;

use async_channel::{Receiver, Sender};
use ferron_common::logging::LogMessage;
#[cfg(feature = "runtime-monoio")]
use pin_project_lite::pin_project;
use quinn::crypto::rustls::QuicServerConfig;
#[cfg(feature = "runtime-monoio")]
use quinn::{AsyncTimer, AsyncUdpSocket, Runtime};
use rustls::ServerConfig;
use tokio_util::sync::CancellationToken;

use crate::listener_handler_communication::{Connection, ConnectionData};

/// A runtime for Quinn that utilizes Monoio and async_io
#[derive(Debug)]
#[cfg(feature = "runtime-monoio")]
struct EnterTokioRuntime;

#[cfg(feature = "runtime-monoio")]
impl Runtime for EnterTokioRuntime {
  fn new_timer(&self, t: Instant) -> Pin<Box<dyn AsyncTimer>> {
    if tokio::runtime::Handle::try_current().is_ok() {
      Box::pin(tokio::time::sleep_until(t.into()))
    } else {
      Box::pin(Timer {
        inner: async_io::Timer::at(t),
      })
    }
  }

  fn spawn(&self, future: Pin<Box<dyn Future<Output = ()> + Send>>) {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
      handle.spawn(future);
    } else {
      monoio::spawn(future);
    }
  }

  fn wrap_udp_socket(&self, sock: std::net::UdpSocket) -> io::Result<Arc<dyn AsyncUdpSocket>> {
    quinn::TokioRuntime::wrap_udp_socket(&quinn::TokioRuntime, sock)
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

/// Creates a QUIC listener
#[allow(clippy::type_complexity)]
pub fn create_quic_listener(
  address: SocketAddr,
  tls_config: Arc<ServerConfig>,
  tx: Sender<ConnectionData>,
  logging_tx: Option<Sender<LogMessage>>,
  first_startup: bool,
) -> Result<(CancellationToken, Sender<Arc<ServerConfig>>), Box<dyn Error + Send + Sync>> {
  let shutdown_tx = CancellationToken::new();
  let shutdown_rx = shutdown_tx.clone();
  let (rustls_config_tx, rustls_config_rx) = async_channel::unbounded();
  let (listen_error_tx, listen_error_rx) = async_channel::unbounded();
  std::thread::Builder::new()
    .name(format!("QUIC listener for {address}"))
    .spawn(move || {
      let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build Tokio runtime");
      rt.block_on(async move {
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
      });
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
  logging_tx: Option<Sender<LogMessage>>,
  first_startup: bool,
  shutdown_rx: CancellationToken,
  rustls_config_rx: Receiver<Arc<ServerConfig>>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
  let quic_server_config = Arc::new(match QuicServerConfig::try_from(tls_config) {
    Ok(config) => config,
    Err(err) => Err(anyhow::anyhow!("Cannot prepare the QUIC server configuration: {}", err))?,
  });
  let server_config = quinn::ServerConfig::with_crypto(quic_server_config);
  let udp_port = address.port();
  let mut udp_socket_result;
  let mut tries: u64 = 0;
  loop {
    udp_socket_result = (|| {
      // Create a new socket
      let listener_socket2 = socket2::Socket::new(
        if address.is_ipv6() {
          socket2::Domain::IPV6
        } else {
          socket2::Domain::IPV4
        },
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
      )?;

      // Set socket options
      if address.is_ipv6() {
        listener_socket2.set_only_v6(false).unwrap_or_default();
      }

      // Bind the socket to the address
      listener_socket2.bind(&address.into())?;

      // Wrap the socket into a UdpSocket
      let listener_socket: std::net::UdpSocket = listener_socket2.into();
      Ok::<_, std::io::Error>(listener_socket)
    })();
    if first_startup || udp_socket_result.is_ok() {
      break;
    }
    tries += 1;
    let duration = Duration::from_millis(1000);
    if tries >= 10 {
      println!("HTTP/3 port is used at try #{tries}, skipping...");
      listen_error_tx.send(None).await.unwrap_or_default();
      break;
    }
    println!("HTTP/3 port is used at try #{tries}, retrying in {duration:?}...");
    if shutdown_rx.is_cancelled() {
      break;
    }
    crate::runtime::sleep(duration).await;
  }
  let udp_socket = match udp_socket_result {
    Ok(socket) => socket,
    Err(err) => Err(anyhow::anyhow!("Cannot listen to HTTP/3 port: {}", err))?,
  };
  let endpoint = match quinn::Endpoint::new(quinn::EndpointConfig::default(), Some(server_config), udp_socket, {
    #[cfg(feature = "runtime-monoio")]
    let runtime = Arc::new(EnterTokioRuntime);
    #[cfg(feature = "runtime-tokio")]
    let runtime = Arc::new(quinn::TokioRuntime);

    runtime
  }) {
    Ok(endpoint) => endpoint,
    Err(err) => Err(anyhow::anyhow!("Cannot listen to HTTP/3 port: {}", err))?,
  };
  println!("HTTP/3 server is listening on {address}...");
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
                  if let Some(logging_tx) = &logging_tx {
                      logging_tx
                          .send(LogMessage::new(
                              "HTTP/3 connections can't be accepted anymore".to_string(),
                              true,
                          ))
                          .await
                          .unwrap_or_default();
                  }
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
      _ = shutdown_rx.cancelled() => {
          break;
      }
    };
    let remote_address = new_conn.remote_address();
    let local_address = SocketAddr::new(
      new_conn.local_ip().unwrap_or(IpAddr::V6(Ipv6Addr::UNSPECIFIED)),
      udp_port,
    );
    let quic_data = ConnectionData {
      connection: Connection::Quic(new_conn),
      client_address: remote_address,
      server_address: local_address,
    };
    let quic_tx = tx.clone();
    tokio::spawn(async move {
      quic_tx.send(quic_data).await.unwrap_or_default();
    });
  }

  endpoint.wait_idle().await;

  Ok(())
}
