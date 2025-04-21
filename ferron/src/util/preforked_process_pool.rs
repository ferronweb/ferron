use std::error::Error;
use std::os::fd::OwnedFd;
use std::sync::Arc;

use interprocess::unnamed_pipe::tokio::{Recver as TokioRecver, Sender as TokioSender};
use interprocess::unnamed_pipe::{Recver, Sender};
use nix::unistd::{ForkResult, Pid};
use tokio::sync::Mutex;

pub struct PreforkedProcessPool {
  inner: Vec<(Arc<Mutex<(TokioSender, TokioRecver)>>, Pid)>,
}

impl PreforkedProcessPool {
  // This function is `unsafe`, due to forking function in `nix` crate also being `unsafe`.
  pub unsafe fn new(
    num_processes: usize,
    pool_fn: impl Fn(Sender, Recver) -> (),
  ) -> Result<Self, Box<dyn Error + Send + Sync>> {
    let mut processes = Vec::new();
    for _ in 0..num_processes {
      let (tx_parent, rx_child) = interprocess::unnamed_pipe::tokio::pipe()?;
      let (tx_child, rx_parent) = interprocess::unnamed_pipe::tokio::pipe()?;
      let tx_child_fd: OwnedFd = tx_child.try_into()?;
      let rx_child_fd: OwnedFd = rx_child.try_into()?;

      match nix::unistd::fork() {
        Ok(ForkResult::Parent { child }) => {
          processes.push((Arc::new(Mutex::new((tx_parent, rx_parent))), child));
        }
        Ok(ForkResult::Child) => {
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

  pub async fn obtain_process(
    &self,
  ) -> Result<Arc<Mutex<(TokioSender, TokioRecver)>>, Box<dyn Error + Send + Sync>> {
    if self.inner.len() == 0 {
      Err(anyhow::anyhow!(
        "The process pool doesn't have any processes"
      ))?
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
      let first_random_process_references = Arc::strong_count(first_random_process);
      let second_random_process_references = Arc::strong_count(second_random_process);
      if first_random_process_references < second_random_process_references {
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

#[cfg(test)]
mod tests {
  use super::*;
  use std::io::{ErrorKind, Read, Write};
  use std::time::Duration;
  use tokio::io::{AsyncReadExt, AsyncWriteExt};
  use tokio::time::timeout;

  fn dummy_pool_fn(tx: Sender, rx: Recver) {
    // Simulate child doing some work and echoing a message
    let mut rx = rx;
    let mut tx = tx;

    let mut buffer = [0u8; 128];

    loop {
      match rx.read(&mut buffer) {
        Ok(n) => {
          let _ = tx.write_all(&buffer[..n]);
        }
        Err(err) => {
          // IMPORTANT! Don't break the read loop when the ErrorKind is `ErrorKind::WouldBlock`
          match err.kind() {
            ErrorKind::WouldBlock => {
              // The IPC channel might not be ready yet...
            }
            _ => break,
          }
        }
      }
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
    tx.write_all(b"hello").await.unwrap();
    let mut buf = vec![0; 5];
    timeout(Duration::from_secs(2), rx.read_exact(&mut buf))
      .await
      .expect("Timed out reading")
      .unwrap();

    assert_eq!(&buf, b"hello");
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
