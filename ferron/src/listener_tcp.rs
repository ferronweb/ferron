use std::net::SocketAddr;
use std::{error::Error, time::Duration};

use async_channel::Sender;
#[cfg(feature = "runtime-monoio")]
use monoio::net::{ListenerOpts, TcpListener};
#[cfg(feature = "runtime-tokio")]
use tokio::net::TcpListener;

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
  tcp_buffer_sizes: (Option<usize>, Option<usize>),
) -> Result<Sender<()>, Box<dyn Error + Send + Sync>> {
  let (shutdown_tx, shutdown_rx) = async_channel::unbounded();
  let (listen_error_tx, listen_error_rx) = async_channel::unbounded();
  std::thread::Builder::new()
        .name(format!("TCP listener for {}", address))
        .spawn(move || {
            crate::runtime::new_runtime(async move {
      crate::runtime::select! {
      result = tcp_listener_fn(address, encrypted, tx, &listen_error_tx, logging_tx, first_startup, tcp_buffer_sizes) => {
          if let Some(error) = result.err() {
              listen_error_tx.send(Some(error)).await.unwrap_or_default();
          }
        }
        _ = shutdown_rx.recv() => {

        }
      }
    }, enable_uring).unwrap();
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
  tcp_buffer_sizes: (Option<usize>, Option<usize>),
) -> Result<(), Box<dyn Error + Send + Sync>> {
  let mut listener_result;
  let mut tries: u64 = 0;
  loop {
    #[cfg(feature = "runtime-monoio")]
    let listener_opts = {
      let mut listener_opts = ListenerOpts::new()
        .reuse_addr(false)
        .reuse_port(false)
        .backlog(-1);
      if let Some(tcp_send_buffer_size) = tcp_buffer_sizes.0 {
        listener_opts = listener_opts.send_buf_size(tcp_send_buffer_size);
      }
      if let Some(tcp_recv_buffer_size) = tcp_buffer_sizes.1 {
        listener_opts = listener_opts.recv_buf_size(tcp_recv_buffer_size);
      }
      listener_opts
    };
    #[cfg(feature = "runtime-monoio")]
    let listener_result2 = TcpListener::bind_with_config(address, &listener_opts);
    #[cfg(feature = "runtime-tokio")]
    let listener_result2 = (|| {
      let listener = std::net::TcpListener::bind(address)?;
      listener.set_nonblocking(true).unwrap_or_default();
      let listener_socket2 = socket2::Socket::from(listener);
      if let Some(tcp_send_buffer_size) = tcp_buffer_sizes.0 {
        listener_socket2
          .set_send_buffer_size(tcp_send_buffer_size)
          .unwrap_or_default();
      }
      if let Some(tcp_recv_buffer_size) = tcp_buffer_sizes.1 {
        listener_socket2
          .set_recv_buffer_size(tcp_recv_buffer_size)
          .unwrap_or_default();
      }
      TcpListener::from_std(listener_socket2.into())
    })();
    listener_result = listener_result2;
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
    crate::runtime::sleep(duration).await;
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

    #[cfg(feature = "runtime-monoio")]
    let tcp_data = {
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
      ConnectionData {
        connection: Connection::Tcp(tcp_std),
        client_address: remote_address,
        server_address: local_address,
      }
    };
    #[cfg(feature = "runtime-tokio")]
    let tcp_data = {
      tcp.set_linger(Some(Duration::ZERO)).unwrap_or_default();
      tcp.set_nodelay(true).unwrap_or_default();

      ConnectionData {
        connection: Connection::Tcp(tcp),
        client_address: remote_address,
        server_address: local_address,
      }
    };

    let tcp_tx = tx.clone();
    crate::runtime::spawn(async move {
      // Send the `TcpStream` and socket addresses to the request handlers
      tcp_tx.send(tcp_data).await.unwrap_or_default();
    });
  }
}
