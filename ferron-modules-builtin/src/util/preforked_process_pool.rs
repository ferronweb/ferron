//! A preforked process pool implementation for efficient IPC communication.
//!
//! This module provides a process pool that forks worker processes upfront and communicates
//! with them via unnamed pipes. It's designed for scenarios where you need to distribute
//! work across multiple processes while maintaining low-latency communication.

use std::error::Error;
use std::io::{Read, Write};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::sync::Arc;

use async_io::{Async, IoSafe};
use interprocess::os::unix::unnamed_pipe::UnnamedPipeExt;
use interprocess::unnamed_pipe::{Recver, Sender};
use nix::sys::signal::{SigSet, SigmaskHow};
use nix::unistd::{ForkResult, Pid};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;
use tokio_util::bytes::BufMut;
use tokio_util::compat::{Compat, FuturesAsyncReadCompatExt, FuturesAsyncWriteCompatExt};

/// A wrapper around `interprocess::unnamed_pipe::Sender` that implements additional traits
/// required for async I/O operations.
///
/// This wrapper enables the sender to be used with `async_io::Async` by implementing
/// the necessary traits like `Write`, `AsFd`, and `IoSafe`.
pub struct SenderWrapped {
  inner: Sender,
}

impl Write for SenderWrapped {
  fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
    self.inner.write(buf)
  }

  fn write_vectored(&mut self, bufs: &[std::io::IoSlice<'_>]) -> std::io::Result<usize> {
    self.inner.write_vectored(bufs)
  }

  fn flush(&mut self) -> std::io::Result<()> {
    self.inner.flush()
  }
}

impl AsFd for SenderWrapped {
  fn as_fd(&self) -> BorrowedFd<'_> {
    self.inner.as_fd()
  }
}

// Safety: it's possible to convert `interprocess::unnamed_pipe::Sender` to `OwnedFd`
unsafe impl IoSafe for SenderWrapped {}

/// A wrapper around `interprocess::unnamed_pipe::Recver` that implements additional traits
/// required for async I/O operations.
///
/// This wrapper enables the receiver to be used with `async_io::Async` by implementing
/// the necessary traits like `Read`, `AsFd`, and `IoSafe`.
pub struct RecverWrapped {
  inner: Recver,
}

impl Read for RecverWrapped {
  fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
    self.inner.read(buf)
  }

  fn read_vectored(&mut self, bufs: &mut [std::io::IoSliceMut<'_>]) -> std::io::Result<usize> {
    self.inner.read_vectored(bufs)
  }
}

impl AsFd for RecverWrapped {
  fn as_fd(&self) -> BorrowedFd<'_> {
    self.inner.as_fd()
  }
}

// Safety: it's possible to convert `interprocess::unnamed_pipe::Recver` to `OwnedFd`
unsafe impl IoSafe for RecverWrapped {}

/// A pool of preforked worker processes with bidirectional IPC communication.
///
/// This structure manages a collection of worker processes that are forked during
/// initialization. Each worker process communicates with the parent through unnamed
/// pipes, allowing for efficient task distribution and result collection.
///
/// # Safety
///
/// This implementation uses `fork()` which has inherent safety considerations in
/// multi-threaded environments. The pool should be created before spawning any
/// additional threads.
///
/// # Examples
///
/// ```no_run
/// use ferron::util::preforked_process_pool::*;
/// use interprocess::unnamed_pipe::{Sender, Recver};
///
/// fn worker_function(mut tx: Sender, mut rx: Recver) {
///     // Worker process logic here
///     while let Ok(message) = read_ipc_message(&mut rx) {
///         // Process message and send response
///         let _ = write_ipc_message(&mut tx, &message);
///     }
/// }
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let pool = unsafe { PreforkedProcessPool::new(4, worker_function)? };
///     let process = pool.obtain_process().await?;
///     // Use the process for communication
///     Ok(())
/// }
/// ```
#[allow(clippy::type_complexity)]
pub struct PreforkedProcessPool {
  inner: Vec<(
    Arc<Mutex<(Compat<Async<SenderWrapped>>, Compat<Async<RecverWrapped>>)>>,
    Pid,
  )>,
}

impl PreforkedProcessPool {
  /// Creates a new preforked process pool with the specified number of worker processes.
  ///
  /// # Arguments
  ///
  /// * `num_processes` - The number of worker processes to fork
  /// * `pool_fn` - The function that each worker process will execute. This function
  ///   receives a sender and receiver for IPC communication with the parent process.
  ///
  /// # Returns
  ///
  /// Returns a `PreforkedProcessPool` on success, or an error if process forking or
  /// pipe creation fails.
  ///
  /// # Safety
  ///
  /// This function is unsafe because it uses `fork()`, which can have undefined behavior
  /// in multi-threaded programs. It should be called before spawning any threads.
  /// Additionally, the function blocks all signals in child processes and sets up
  /// process death signals on Linux systems.
  ///
  /// # Errors
  ///
  /// * Returns an error if pipe creation fails
  /// * Returns an error if the fork operation fails
  /// * Returns an error if async I/O setup fails
  ///
  /// # Examples
  ///
  /// ```no_run
  /// # use ferron::util::preforked_process_pool::*;
  /// # use interprocess::unnamed_pipe::{Sender, Recver};
  /// fn echo_worker(mut tx: Sender, mut rx: Recver) {
  ///     while let Ok(msg) = read_ipc_message(&mut rx) {
  ///         let _ = write_ipc_message(&mut tx, &msg);
  ///     }
  /// }
  ///
  /// let pool = unsafe { PreforkedProcessPool::new(2, echo_worker) }?;
  /// # Ok::<(), Box<dyn std::error::Error>>(())
  /// ```
  pub unsafe fn new(
    num_processes: usize,
    pool_fn: impl Fn(Sender, Recver),
  ) -> Result<Self, Box<dyn Error + Send + Sync>> {
    let mut processes = Vec::new();
    for _ in 0..num_processes {
      // Create unnamed pipes
      let (tx_parent, rx_child) = interprocess::unnamed_pipe::pipe()?;
      let (tx_child, rx_parent) = interprocess::unnamed_pipe::pipe()?;

      // Set parent pipes to be non-blocking, because they'll be used in an asynchronous context
      tx_parent.set_nonblocking(true).unwrap_or_default();
      rx_parent.set_nonblocking(true).unwrap_or_default();

      // Obtain the file descriptors of the pipes
      let tx_parent_fd: OwnedFd = tx_parent.into();
      let rx_parent_fd: OwnedFd = rx_parent.into();
      let tx_child_fd: OwnedFd = tx_child.into();
      let rx_child_fd: OwnedFd = rx_child.into();

      match nix::unistd::fork() {
        Ok(ForkResult::Parent { child }) => {
          processes.push((
            Arc::new(Mutex::new((
              Async::new_nonblocking(SenderWrapped {
                inner: tx_parent_fd.into(),
              })?
              .compat_write(),
              Async::new_nonblocking(RecverWrapped {
                inner: rx_parent_fd.into(),
              })?
              .compat(),
            ))),
            child,
          ));
        }
        Ok(ForkResult::Child) => {
          // Block all the signals
          nix::sys::signal::sigprocmask(SigmaskHow::SIG_SETMASK, Some(&SigSet::all()), None).unwrap_or_default();

          #[cfg(target_os = "linux")]
          {
            // Stop child process after the parent process is stopped on Linux systems
            nix::sys::prctl::set_pdeathsig(nix::sys::signal::SIGKILL).unwrap_or_default();
          }

          pool_fn(tx_child_fd.into(), rx_child_fd.into());

          // Exit the process in the process pool
          std::process::exit(0);
        }
        Err(errno) => {
          Err(errno)?;
        }
      }
    }
    Ok(Self { inner: processes })
  }

  /// Obtains a worker process from the pool for communication.
  ///
  /// This method implements a load-balancing strategy by selecting the process
  /// with the fewest active references. For pools with more than one process,
  /// it randomly selects two processes and returns the one with fewer references.
  ///
  /// # Returns
  ///
  /// Returns an `Arc<Mutex<...>>` containing the communication channels (sender and receiver)
  /// for the selected worker process.
  ///
  /// # Errors
  ///
  /// Returns an error if the process pool is empty (contains no worker processes).
  ///
  /// # Examples
  ///
  /// ```no_run
  /// # use ferron::util::preforked_process_pool::*;
  /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
  /// # let pool = unsafe { PreforkedProcessPool::new(2, |_,_| {}) }?;
  /// let process = pool.obtain_process().await?;
  /// let mut channels = process.lock().await;
  /// let (tx, rx) = &mut *channels;
  ///
  /// // Send a message to the worker
  /// write_ipc_message_async(tx, b"Hello, worker!").await?;
  ///
  /// // Read the response
  /// let response = read_ipc_message_async(rx).await?;
  /// # Ok(())
  /// # }
  /// ```
  pub async fn obtain_process(
    &self,
  ) -> Result<Arc<Mutex<(Compat<Async<SenderWrapped>>, Compat<Async<RecverWrapped>>)>>, Box<dyn Error + Send + Sync>>
  {
    if self.inner.is_empty() {
      Err(anyhow::anyhow!("The process pool doesn't have any processes"))?
    } else if self.inner.len() == 1 {
      Ok(self.inner[0].0.clone())
    } else {
      let first_random_choice = rand::random_range(0..self.inner.len());
      let second_random_choice_reduced = rand::random_range(0..self.inner.len() - 1);
      let second_random_choice = if second_random_choice_reduced < first_random_choice {
        second_random_choice_reduced
      } else {
        second_random_choice_reduced + 1
      };
      let first_random_process = &self.inner[first_random_choice].0;
      let second_random_process = &self.inner[second_random_choice].0;
      let first_random_process_reference = Arc::strong_count(first_random_process);
      let second_random_process_reference = Arc::strong_count(second_random_process);
      if first_random_process_reference < second_random_process_reference {
        Ok(first_random_process.clone())
      } else {
        Ok(second_random_process.clone())
      }
    }
  }
}

impl Drop for PreforkedProcessPool {
  fn drop(&mut self) {
    for inner_process in &self.inner {
      // Kill processes in the process pool when dropping the process pool
      nix::sys::signal::kill(inner_process.1, nix::sys::signal::SIGCHLD).unwrap_or_default();
    }
  }
}

/// Reads a message from an IPC receiver in synchronous mode.
///
/// This function reads a message that was sent using the protocol established by
/// `write_ipc_message`. It first reads a 4-byte big-endian message size, then
/// reads the message content of that size.
///
/// # Arguments
///
/// * `rx` - A mutable reference to the receiver
///
/// # Returns
///
/// Returns the message content as a `Vec<u8>` on success, or an I/O error if
/// reading fails.
///
/// # Protocol
///
/// The message protocol consists of:
/// 1. 4 bytes: message length as big-endian u32
/// 2. N bytes: message content
///
/// # Examples
///
/// ```no_run
/// # use ferron::util::preforked_process_pool::*;
/// # use interprocess::unnamed_pipe::Recver;
/// # fn example(mut rx: Recver) -> Result<(), std::io::Error> {
/// let message = read_ipc_message(&mut rx)?;
/// println!("Received: {:?}", String::from_utf8_lossy(&message));
/// # Ok(())
/// # }
/// ```
pub fn read_ipc_message(rx: &mut Recver) -> Result<Vec<u8>, std::io::Error> {
  let mut message_size_buffer = [0u8; 4];
  rx.read_exact(&mut message_size_buffer)?;
  let message_size = u32::from_be_bytes(message_size_buffer);

  let mut buffer = vec![0u8; message_size as usize];
  rx.read_exact(&mut buffer)?;
  Ok(buffer)
}

/// Reads a message from an IPC receiver in asynchronous mode.
///
/// This is the async version of `read_ipc_message`. It reads a message using the
/// same protocol but operates asynchronously, making it suitable for use in
/// async contexts without blocking.
///
/// # Arguments
///
/// * `rx` - A mutable reference to the async-compatible receiver
///
/// # Returns
///
/// Returns the message content as a `Vec<u8>` on success, or an I/O error if
/// reading fails.
///
/// # Examples
///
/// ```no_run
/// # use ferron::util::preforked_process_pool::*;
/// # async fn example(rx: &mut tokio_util::compat::Compat<async_io::Async<RecverWrapped>>) -> Result<(), std::io::Error> {
/// let message = read_ipc_message_async(rx).await?;
/// println!("Received: {:?}", String::from_utf8_lossy(&message));
/// # Ok(())
/// # }
/// ```
pub async fn read_ipc_message_async(rx: &mut Compat<Async<RecverWrapped>>) -> Result<Vec<u8>, std::io::Error> {
  let mut message_size_buffer = [0u8; 4];
  rx.read_exact(&mut message_size_buffer).await?;
  let message_size = u32::from_be_bytes(message_size_buffer);

  let mut buffer = vec![0u8; message_size as usize];
  rx.read_exact(&mut buffer).await?;
  Ok(buffer)
}

/// Writes a message to an IPC sender in synchronous mode.
///
/// This function writes a message using a length-prefixed protocol. It first
/// writes the message length as a 4-byte big-endian integer, followed by the
/// message content.
///
/// # Arguments
///
/// * `tx` - A mutable reference to the sender
/// * `message` - The message content to send
///
/// # Returns
///
/// Returns `Ok(())` on success, or an I/O error if writing fails.
///
/// # Protocol
///
/// The message protocol consists of:
/// 1. 4 bytes: message length as big-endian u32
/// 2. N bytes: message content
///
/// # Examples
///
/// ```no_run
/// # use ferron::util::preforked_process_pool::*;
/// # use interprocess::unnamed_pipe::Sender;
/// # fn example(mut tx: Sender) -> Result<(), std::io::Error> {
/// let message = b"Hello, parent process!";
/// write_ipc_message(&mut tx, message)?;
/// # Ok(())
/// # }
/// ```
pub fn write_ipc_message(tx: &mut Sender, message: &[u8]) -> Result<(), std::io::Error> {
  let mut packet = Vec::new();
  packet.put_slice(&(message.len() as u32).to_be_bytes());
  packet.put_slice(message);
  tx.write_all(&packet)?;
  Ok(())
}

/// Writes a message to an IPC sender in asynchronous mode.
///
/// This is the async version of `write_ipc_message`. It writes a message using
/// the same length-prefixed protocol but operates asynchronously, making it
/// suitable for use in async contexts without blocking.
///
/// # Arguments
///
/// * `tx` - A mutable reference to the async-compatible sender
/// * `message` - The message content to send
///
/// # Returns
///
/// Returns `Ok(())` on success, or an I/O error if writing fails.
///
/// # Examples
///
/// ```no_run
/// # use ferron::util::preforked_process_pool::*;
/// # async fn example(tx: &mut tokio_util::compat::Compat<async_io::Async<SenderWrapped>>) -> Result<(), std::io::Error> {
/// let message = b"Hello, worker process!";
/// write_ipc_message_async(tx, message).await?;
/// # Ok(())
/// # }
/// ```
pub async fn write_ipc_message_async(
  tx: &mut Compat<Async<SenderWrapped>>,
  message: &[u8],
) -> Result<(), std::io::Error> {
  let mut packet = Vec::new();
  packet.put_slice(&(message.len() as u32).to_be_bytes());
  packet.put_slice(message);
  tx.write_all(&packet).await?;
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::time::Duration;
  use tokio::time::timeout;

  fn dummy_pool_fn(mut tx: Sender, mut rx: Recver) {
    // Simulate child doing some work and echoing a message
    while let Ok(message) = read_ipc_message(&mut rx) {
      let _ = write_ipc_message(&mut tx, &message);
    }
  }

  #[tokio::test]
  async fn test_process_pool_creation() {
    let pool = unsafe { PreforkedProcessPool::new(2, dummy_pool_fn) }.unwrap();
    assert_eq!(pool.inner.len(), 2);
  }

  #[tokio::test]
  async fn test_obtain_process_and_communication() {
    let pool = unsafe { PreforkedProcessPool::new(1, dummy_pool_fn) }.unwrap();
    let proc = pool.obtain_process().await.unwrap();
    let mut proc = proc.lock().await;
    let (tx, rx) = &mut *proc;

    // Write and read a message
    write_ipc_message_async(tx, b"hello").await.unwrap();
    let message = timeout(Duration::from_secs(2), read_ipc_message_async(rx))
      .await
      .expect("Timed out reading")
      .unwrap();

    assert_eq!(&message, b"hello");
  }

  #[tokio::test]
  async fn test_obtain_process_balancing() {
    let pool = unsafe { PreforkedProcessPool::new(3, dummy_pool_fn) }.unwrap();

    let _p1 = pool.obtain_process().await.unwrap();
    let _p2 = pool.obtain_process().await.unwrap();
    let _p3 = pool.obtain_process().await.unwrap();

    // This ensures reference counts differ
    let chosen = pool.obtain_process().await;
    assert!(chosen.is_ok());
  }

  #[tokio::test]
  async fn test_obtain_process_empty_pool() {
    let empty_pool = PreforkedProcessPool { inner: Vec::new() };
    let result = empty_pool.obtain_process().await;
    assert!(result.is_err());
  }
}
