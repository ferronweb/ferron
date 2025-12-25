use std::error::Error;
use std::future::Future;

// Compilation errors
#[cfg(all(feature = "runtime-monoio", feature = "runtime-tokio"))]
compile_error!("Can't compile Ferron with both main runtimes enabled");
#[cfg(not(any(feature = "runtime-monoio", feature = "runtime-tokio")))]
compile_error!("Can't compile Ferron with no main runtimes enabled");

/// Creates a new asynchronous runtime using Monoio
#[cfg(feature = "runtime-monoio")]
pub fn new_runtime(future: impl Future, enable_uring: bool) -> Result<(), Box<dyn Error + Send + Sync>> {
  #[cfg(target_os = "linux")]
  if enable_uring && monoio::utils::detect_uring() {
    let mut rt = monoio::RuntimeBuilder::<monoio::IoUringDriver>::new()
      .enable_all()
      .attach_thread_pool(Box::new(BlockingThreadPool))
      .build()?;
    rt.block_on(future);
    return Ok(());
  }

  // `io_uring` is either disabled or not supported
  let mut rt = monoio::RuntimeBuilder::<monoio::LegacyDriver>::new()
    .enable_all()
    .attach_thread_pool(Box::new(BlockingThreadPool))
    .build()?;
  rt.block_on(future);

  Ok(())
}

/// Creates a new asynchronous runtime using Tokio
#[cfg(feature = "runtime-tokio")]
pub fn new_runtime(future: impl Future, _enable_uring: bool) -> Result<(), Box<dyn Error + Send + Sync>> {
  let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
  rt.block_on(async move {
    let local_set = tokio::task::LocalSet::new();
    local_set.run_until(future).await;
  });
  Ok(())
}

pub use ferron_common::runtime::*;

/// A blocking thread pool for Monoio, implemented using `blocking` crate
#[cfg(feature = "runtime-monoio")]
struct BlockingThreadPool;

#[cfg(feature = "runtime-monoio")]
impl monoio::blocking::ThreadPool for BlockingThreadPool {
  #[inline]
  fn schedule_task(&self, task: monoio::blocking::BlockingTask) {
    blocking::unblock(move || task.run()).detach();
  }
}
