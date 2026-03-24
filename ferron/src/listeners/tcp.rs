use std::net::SocketAddr;
use std::{error::Error, time::Duration};

use async_channel::Sender;
use ferron_common::logging::LogMessage;
#[cfg(feature = "runtime-monoio")]
use monoio::net::TcpListener;
#[cfg(feature = "runtime-tokio")]
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
#[cfg(feature = "runtime-vibeio")]
use vibeio::net::TcpListener;

use crate::listener_handler_communication::{Connection, ConnectionData};

type ListenerError = Box<dyn Error + Send + Sync>;
type ListenerResult = Result<TcpListener, std::io::Error>;

#[inline]
fn protocol_name(encrypted: bool) -> &'static str {
  if encrypted {
    "HTTPS"
  } else {
    "HTTP"
  }
}

#[inline]
fn listen_error_message(encrypted: bool, err: &std::io::Error) -> anyhow::Error {
  anyhow::anyhow!("Cannot listen to {} port: {err}", protocol_name(encrypted))
}

#[inline]
fn log_retry(encrypted: bool, tries: u64, duration: Duration) {
  println!(
    "{} port is used at try #{tries}, retrying in {duration:?}...",
    protocol_name(encrypted)
  );
}

#[inline]
fn log_skip(encrypted: bool, tries: u64) {
  println!("{} port is used at try #{tries}, skipping...", protocol_name(encrypted));
}

#[inline]
fn log_listening(encrypted: bool, address: SocketAddr) {
  println!("{} server is listening on {address}...", protocol_name(encrypted));
}

#[inline]
fn build_tcp_listener(address: SocketAddr, tcp_buffer_sizes: (Option<usize>, Option<usize>)) -> ListenerResult {
  // Create a new socket
  let listener_socket2 = socket2::Socket::new(
    if address.is_ipv6() {
      socket2::Domain::IPV6
    } else {
      socket2::Domain::IPV4
    },
    socket2::Type::STREAM,
    Some(socket2::Protocol::TCP),
  )?;

  // Set socket options
  listener_socket2.set_reuse_address(!cfg!(windows)).unwrap_or_default();
  #[cfg(unix)]
  listener_socket2.set_reuse_port(false).unwrap_or_default();
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
  if address.is_ipv6() {
    listener_socket2.set_only_v6(false).unwrap_or_default();
  }

  #[cfg(feature = "runtime-monoio")]
  let is_poll_io = monoio::utils::is_legacy();
  #[cfg(feature = "runtime-vibeio")]
  let is_poll_io = !vibeio::util::supports_completion();
  #[cfg(feature = "runtime-tokio")]
  let is_poll_io = true;

  if is_poll_io {
    listener_socket2.set_nonblocking(true).unwrap_or_default();
  }

  // Bind the socket to the address
  listener_socket2.bind(&address.into())?;
  listener_socket2.listen(-1)?;

  // Wrap the socket into a TcpListener
  TcpListener::from_std(listener_socket2.into())
}

#[inline]
async fn log_accept_error(logging_tx: &Option<Sender<LogMessage>>, err: &std::io::Error) {
  if let Some(logging_tx) = logging_tx {
    logging_tx
      .send(LogMessage::new(format!("Cannot accept a connection: {err}"), true))
      .await
      .unwrap_or_default();
  }
}

/// Creates a TCP listener
#[allow(clippy::too_many_arguments)]
pub fn create_tcp_listener(
  address: SocketAddr,
  encrypted: bool,
  tx: Sender<ConnectionData>,
  enable_uring: Option<bool>,
  logging_tx: Option<Sender<LogMessage>>,
  first_startup: bool,
  tcp_buffer_sizes: (Option<usize>, Option<usize>),
  io_uring_disabled: Sender<Option<std::io::Error>>,
) -> Result<CancellationToken, ListenerError> {
  let shutdown_tx = CancellationToken::new();
  let shutdown_rx = shutdown_tx.clone();
  let (listen_error_tx, listen_error_rx) = async_channel::unbounded();
  std::thread::Builder::new()
    .name(format!("TCP listener for {address}"))
    .spawn(move || {
      let mut rt = match crate::runtime::Runtime::new_runtime(enable_uring) {
        Ok(rt) => rt,
        Err(error) => {
          listen_error_tx
            .send_blocking(Some(
              anyhow::anyhow!("Can't create async runtime: {error}").into_boxed_dyn_error(),
            ))
            .unwrap_or_default();
          return;
        }
      };
      io_uring_disabled
        .send_blocking(rt.return_io_uring_error())
        .unwrap_or_default();
      rt.run(async move {
        if let Err(error) = tcp_listener_fn(
          address,
          encrypted,
          tx,
          &listen_error_tx,
          logging_tx,
          first_startup,
          tcp_buffer_sizes,
          shutdown_rx,
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

  Ok(shutdown_tx)
}

/// TCP listener function
#[allow(clippy::too_many_arguments)]
async fn tcp_listener_fn(
  address: SocketAddr,
  encrypted: bool,
  tx: Sender<ConnectionData>,
  listen_error_tx: &Sender<Option<ListenerError>>,
  logging_tx: Option<Sender<LogMessage>>,
  first_startup: bool,
  tcp_buffer_sizes: (Option<usize>, Option<usize>),
  shutdown_rx: CancellationToken,
) -> Result<(), ListenerError> {
  let mut listener_result;
  let mut tries: u64 = 0;
  loop {
    listener_result = build_tcp_listener(address, tcp_buffer_sizes);
    if first_startup || listener_result.is_ok() {
      break;
    }
    tries += 1;
    if tries >= 10 {
      log_skip(encrypted, tries);
      listen_error_tx.send(None).await.unwrap_or_default();
      break;
    }
    let duration = Duration::from_millis(1000);
    log_retry(encrypted, tries, duration);
    crate::runtime::sleep(duration).await;
  }
  let listener = match listener_result {
    Ok(listener) => listener,
    Err(err) => Err(listen_error_message(encrypted, &err))?,
  };

  log_listening(encrypted, address);
  listen_error_tx.send(None).await.unwrap_or_default();

  #[cfg(unix)]
  let mut handle_exhaustion_backoff = Duration::from_millis(10);

  loop {
    let (tcp, remote_address) = match crate::runtime::select! {
      result = listener.accept() => {
        result
      }
      _ = shutdown_rx.cancelled() => {
        return Ok(());
      }
    } {
      Ok(data) => {
        #[cfg(unix)]
        {
          handle_exhaustion_backoff = Duration::from_millis(10);
        }
        data
      }
      Err(err) => {
        log_accept_error(&logging_tx, &err).await;

        // 24 = EMFILE
        #[cfg(unix)]
        if err.raw_os_error() == Some(24) {
          crate::runtime::sleep(handle_exhaustion_backoff).await;
          handle_exhaustion_backoff *= 2;
          if handle_exhaustion_backoff > Duration::from_secs(1) {
            handle_exhaustion_backoff = Duration::from_secs(1);
          }
        }

        continue;
      }
    };
    let local_address: SocketAddr = match tcp.local_addr() {
      Ok(data) => data,
      Err(err) => {
        log_accept_error(&logging_tx, &err).await;
        continue;
      }
    };

    #[cfg(any(feature = "runtime-vibeio", feature = "runtime-monoio"))]
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

      // Set TCP_NODELAY
      let tcp_socket2 = socket2::Socket::from(tcp_std);
      tcp_socket2.set_tcp_nodelay(true).unwrap_or_default();

      let tcp_std = tcp_socket2.into();
      ConnectionData {
        connection: Connection::Tcp(tcp_std),
        client_address: remote_address,
        server_address: local_address,
      }
    };
    #[cfg(feature = "runtime-tokio")]
    let tcp_data = {
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
