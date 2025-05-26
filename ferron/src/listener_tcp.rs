use std::net::SocketAddr;
use std::{error::Error, time::Duration};

use async_channel::Sender;
use monoio::net::{ListenerOpts, TcpListener};
use monoio::utils::detect_uring;

use crate::listener_handler_communication::{Connection, ConnectionData};
use crate::logging::LogMessage;

/// Creates a TCP listener
pub fn create_tcp_listener(
  address: SocketAddr,
  encrypted: bool,
  tx: Sender<ConnectionData>,
  enable_uring: bool,
  logging_tx: Sender<LogMessage>,
  first_startup: bool,
) -> Result<Sender<()>, Box<dyn Error + Send + Sync>> {
  let (shutdown_tx, shutdown_rx) = async_channel::unbounded();
  let (listen_error_tx, listen_error_rx) = async_channel::unbounded();
  std::thread::Builder::new()
        .name(format!("TCP listener for {}", address))
        .spawn(move || {
            if enable_uring && detect_uring() {
                #[cfg(target_os = "linux")]
                let mut rt = monoio::RuntimeBuilder::<monoio::IoUringDriver>::new()
                    .enable_all()
                    .build()
                    .unwrap();
                #[cfg(not(target_os = "linux"))]
                let mut rt = monoio::RuntimeBuilder::<monoio::LegacyDriver>::new()
                    .enable_all()
                    .build()
                    .unwrap();
                rt.block_on(async move {
          monoio::select! {
          result = tcp_listener_fn(address, encrypted, tx, &listen_error_tx, logging_tx, first_startup) => {
              if let Some(error) = result.err() {
                  listen_error_tx.send(Some(error)).await.unwrap_or_default();
              }
            }
            _ = shutdown_rx.recv() => {

            }
          }
        });
            } else {
                let mut rt = monoio::RuntimeBuilder::<monoio::LegacyDriver>::new()
                    .enable_all()
                    .build()
                    .unwrap();
                rt.block_on(async move {
          monoio::select! {
          result = tcp_listener_fn(address, encrypted, tx, &listen_error_tx, logging_tx, first_startup) => {
              if let Some(error) = result.err() {
                  listen_error_tx.send(Some(error)).await.unwrap_or_default();
              }
            }
            _ = shutdown_rx.recv() => {

            }
          }
        });
            }
        })?;

  if let Some(error) = listen_error_rx.recv_blocking()? {
    Err(error)?;
  }

  Ok(shutdown_tx)
}

/// TCP listener function
async fn tcp_listener_fn(
  address: SocketAddr,
  encrypted: bool,
  tx: Sender<ConnectionData>,
  listen_error_tx: &Sender<Option<Box<dyn Error + Send + Sync>>>,
  logging_tx: Sender<LogMessage>,
  first_startup: bool,
) -> Result<(), Box<dyn Error + Send + Sync>> {
  let listener_opts = ListenerOpts::new()
    .reuse_addr(false)
    .reuse_port(false)
    .backlog(-1);
  let mut listener_result;
  let mut tries: u64 = 0;
  loop {
    listener_result = TcpListener::bind_with_config(address, &listener_opts);
    if first_startup || listener_result.is_ok() {
      break;
    }
    tries += 1;
    if tries >= 10 {
      if encrypted {
        println!("HTTPS port is used at try #{}, skipping...", tries);
      } else {
        println!("HTTP port is used at try #{}, skipping...", tries);
      }
      listen_error_tx.send(None).await.unwrap_or_default();
      break;
    }
    let duration = Duration::from_millis(1000);
    if encrypted {
      println!(
        "HTTPS port is used at try #{}, retrying in {:?}...",
        tries, duration
      );
    } else {
      println!(
        "HTTP port is used at try #{}, retrying in {:?}...",
        tries, duration
      );
    }
    monoio::time::sleep(duration).await;
  }
  let listener = match listener_result {
    Ok(listener) => listener,
    Err(err) => {
      if encrypted {
        Err(anyhow::anyhow!(format!(
          "Cannot listen to HTTPS port: {}",
          err
        )))?
      } else {
        Err(anyhow::anyhow!(format!(
          "Cannot listen to HTTP port: {}",
          err
        )))?
      }
    }
  };

  if encrypted {
    println!("HTTPS server is listening on {}...", address);
  } else {
    println!("HTTP server is listening on {}...", address);
  }
  listen_error_tx.send(None).await.unwrap_or_default();

  loop {
    let (tcp, remote_address) = match listener.accept().await {
      Ok(data) => data,
      Err(err) => {
        logging_tx
          .send(LogMessage::new(
            format!("Cannot accept a connection: {}", err),
            true,
          ))
          .await
          .unwrap_or_default();
        continue;
      }
    };
    let local_address: SocketAddr = match tcp.local_addr() {
      Ok(data) => data,
      Err(err) => {
        logging_tx
          .send(LogMessage::new(
            format!("Cannot accept a connection: {}", err),
            true,
          ))
          .await
          .unwrap_or_default();
        continue;
      }
    };

    #[cfg(unix)]
    let tcp_std = {
      use std::os::fd::{FromRawFd, IntoRawFd};
      let raw_fd = tcp.into_raw_fd();
      // Safety: We just extracted the raw file descriptor from the Monoio TcpStream,
      // and we are immediately wrapping it in a std::net::TcpStream. No other code
      // has access to the raw_fd in the interim, so we uphold the invariant that
      // the fd is owned by only one entity at a time.
      unsafe { std::net::TcpStream::from_raw_fd(raw_fd) }
    };
    #[cfg(windows)]
    let tcp_std = {
      use std::os::windows::io::{FromRawSocket, IntoRawSocket};
      let raw_fd = tcp.into_raw_socket();
      // Safety: We extracted the raw socket from the Monoio TcpStream and are
      // immediately converting it into a std::net::TcpStream. No duplication or
      // other use of the raw socket occurs, so ownership and safety invariants are preserved.
      unsafe { std::net::TcpStream::from_raw_socket(raw_fd) }
    };

    // Set SO_LINGER and TCP_NODELAY
    let tcp_socket2 = socket2::Socket::from(tcp_std);
    tcp_socket2
      .set_linger(Some(Duration::ZERO))
      .unwrap_or_default();
    tcp_socket2.set_nodelay(true).unwrap_or_default();

    let tcp_std = tcp_socket2.into();
    let tcp_data = ConnectionData {
      connection: Connection::Tcp(tcp_std),
      client_address: remote_address,
      server_address: local_address,
    };
    let tcp_tx = tx.clone();
    monoio::spawn(async move {
      // Send the `TcpStream` and socket addresses to the request handlers
      tcp_tx.send(tcp_data).await.unwrap_or_default();
    });
  }
}
