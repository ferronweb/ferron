use std::error::Error;
use std::ops::Deref;
use std::os::fd::OwnedFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use interprocess::os::unix::unnamed_pipe::UnnamedPipeExt;
use interprocess::unnamed_pipe::tokio::{Recver as TokioRecver, Sender as TokioSender};
use interprocess::unnamed_pipe::{Recver, Sender};
use nix::sys::signal::{SigSet, SigmaskHow};
use nix::unistd::{ForkResult, Pid};
use tokio::sync::{Mutex, RwLock};

#[allow(clippy::type_complexity)]
pub struct PreforkedProcessPool {
  inner: Vec<(
    Arc<RwLock<Option<Result<Arc<Mutex<(TokioSender, TokioRecver)>>, std::io::Error>>>>,
    Pid,
    Arc<Mutex<Option<(OwnedFd, OwnedFd)>>>,
  )>,
  async_ipc_initialized: Arc<RwLock<AtomicBool>>,
}

impl PreforkedProcessPool {
  // This function is `unsafe`, due to forking function in `nix` crate also being `unsafe`.
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
      let tx_parent_fd: OwnedFd = tx_parent.try_into()?;
      let rx_parent_fd: OwnedFd = rx_parent.try_into()?;
      let tx_child_fd: OwnedFd = tx_child.try_into()?;
      let rx_child_fd: OwnedFd = rx_child.try_into()?;

      match nix::unistd::fork() {
        Ok(ForkResult::Parent { child }) => {
          processes.push((
            Arc::new(RwLock::new(None)),
            child,
            Arc::new(Mutex::new(Some((tx_parent_fd, rx_parent_fd)))),
          ));
        }
        Ok(ForkResult::Child) => {
          // Block all the signals
          nix::sys::signal::sigprocmask(SigmaskHow::SIG_SETMASK, Some(&SigSet::all()), None)
            .unwrap_or_default();

          pool_fn(tx_child_fd.into(), rx_child_fd.into());

          // Exit the process in the process pool
          std::process::exit(0);
        }
        Err(errno) => {
          Err(errno)?;
        }
      }
    }
    Ok(Self {
      inner: processes,
      async_ipc_initialized: Arc::new(RwLock::new(AtomicBool::new(false))),
    })
  }

  pub async fn init_async_ipc(&self) {
    if !self
      .async_ipc_initialized
      .read()
      .await
      .load(Ordering::Relaxed)
    {
      for inner_process in &self.inner {
        let fds_option = inner_process.2.lock().await.take();
        if let Some((tx_fd, rx_fd)) = fds_option {
          let ipc_io_result = match tx_fd.try_into() {
            Ok(tx) => match rx_fd.try_into() {
              Ok(rx) => Ok(Arc::new(Mutex::new((tx, rx)))),
              Err(err) => Err(err),
            },
            Err(err) => Err(err),
          };

          inner_process.0.write().await.replace(ipc_io_result);
        }
      }

      self
        .async_ipc_initialized
        .write()
        .await
        .store(true, Ordering::Relaxed);
    }
  }

  pub async fn obtain_process(
    &self,
  ) -> Result<Arc<Mutex<(TokioSender, TokioRecver)>>, Box<dyn Error + Send + Sync>> {
    if self.inner.is_empty() {
      Err(anyhow::anyhow!(
        "The process pool doesn't have any processes"
      ))?
    } else if self.inner.len() == 1 {
      let process_option = self.inner[0].0.read().await;
      let process = match process_option.as_ref() {
        Some(arc_mutex_result) => arc_mutex_result
          .as_ref()
          .map_err(|e| std::io::Error::new(e.kind(), e.to_string()))?,
        None => Err(anyhow::anyhow!("Asynchronous IPC not initialized yet"))?,
      };
      Ok(process.clone())
    } else {
      let first_random_choice = rand::random_range(0..self.inner.len());
      let second_random_choice_reduced = rand::random_range(0..self.inner.len() - 1);
      let second_random_choice = if second_random_choice_reduced < first_random_choice {
        second_random_choice_reduced
      } else {
        second_random_choice_reduced + 1
      };
      let first_random_process_option = self.inner[first_random_choice].0.read().await;
      let second_random_process_option = self.inner[second_random_choice].0.read().await;
      let first_random_process = match first_random_process_option.as_ref() {
        Some(arc_mutex_result) => arc_mutex_result
          .as_ref()
          .map_err(|e| std::io::Error::new(e.kind(), e.to_string()))?,
        None => Err(anyhow::anyhow!("Asynchronous IPC not initialized yet"))?,
      };
      let second_random_process = match second_random_process_option.as_ref() {
        Some(arc_mutex_result) => arc_mutex_result
          .as_ref()
          .map_err(|e| std::io::Error::new(e.kind(), e.to_string()))?,
        None => Err(anyhow::anyhow!("Asynchronous IPC not initialized yet"))?,
      };
      let first_random_process_reference = Arc::strong_count(first_random_process);
      let second_random_process_reference = Arc::strong_count(second_random_process);
      if first_random_process_reference < second_random_process_reference {
        Ok(first_random_process.clone())
      } else {
        Ok(second_random_process.clone())
      }
    }
  }

  pub async fn obtain_process_with_init_async_ipc(
    &self,
  ) -> Result<Arc<Mutex<(TokioSender, TokioRecver)>>, Box<dyn Error + Send + Sync>> {
    self.init_async_ipc().await;
    self.obtain_process().await
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
  use std::io::{Read, Write};
  use std::time::Duration;
  use tokio::io::{AsyncReadExt, AsyncWriteExt};
  use tokio::time::timeout;

  fn dummy_pool_fn(mut tx: Sender, mut rx: Recver) {
    // Simulate child doing some work and echoing a message
    let mut buffer = [0u8; 128];

    loop {
      match rx.read(&mut buffer) {
        Ok(n) => {
          let _ = tx.write_all(&buffer[..n]);
        }
        Err(_) => break,
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
    let proc = pool.obtain_process_with_init_async_ipc().await.unwrap();
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

    let _p1 = pool.obtain_process_with_init_async_ipc().await.unwrap();
    let _p2 = pool.obtain_process_with_init_async_ipc().await.unwrap();
    let _p3 = pool.obtain_process_with_init_async_ipc().await.unwrap();

    // This ensures reference counts differ
    let chosen = pool.obtain_process().await;
    assert!(chosen.is_ok());
  }

  #[tokio::test]
  async fn test_obtain_process_empty_pool() {
    let empty_pool = PreforkedProcessPool {
      inner: Vec::new(),
      async_ipc_initialized: Arc::new(RwLock::new(AtomicBool::new(false))),
    };
    let result = empty_pool.obtain_process_with_init_async_ipc().await;
    assert!(result.is_err());
  }
}
