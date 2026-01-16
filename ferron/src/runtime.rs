use std::future::Future;

// Compilation errors
#[cfg(all(feature = "runtime-monoio", feature = "runtime-tokio"))]
compile_error!("Can't compile Ferron with both main runtimes enabled");
#[cfg(not(any(feature = "runtime-monoio", feature = "runtime-tokio")))]
compile_error!("Can't compile Ferron with no main runtimes enabled");

/// A representation of an asynchronous runtime
pub struct Runtime {
  inner: RuntimeInner,
  io_uring_enable_configured: Option<i32>,
}

enum RuntimeInner {
  #[cfg(all(feature = "runtime-monoio", target_os = "linux"))]
  MonoioIouring(monoio::Runtime<monoio::time::TimeDriver<monoio::IoUringDriver>>),
  #[cfg(feature = "runtime-monoio")]
  MonoioLegacy(monoio::Runtime<monoio::time::TimeDriver<monoio::LegacyDriver>>),
  #[cfg(feature = "runtime-tokio")]
  Tokio(tokio::runtime::Runtime),
  TokioOnly(tokio::runtime::Runtime),
}

impl Runtime {
  /// Creates a new asynchronous runtime
  pub fn new_runtime(enable_uring: Option<bool>) -> Result<Self, std::io::Error> {
    #[allow(unused_mut)]
    let mut io_uring_enable_configured = None;

    #[cfg(all(feature = "runtime-monoio", target_os = "linux"))]
    if enable_uring.is_none_or(|x| x) && monoio::utils::detect_uring() {
      let rt = monoio::RuntimeBuilder::<monoio::IoUringDriver>::new()
        .enable_all()
        .attach_thread_pool(Box::new(BlockingThreadPool))
        .build()?;
      return Ok(Self {
        inner: RuntimeInner::MonoioIouring(rt),
        io_uring_enable_configured: None,
      });
    }
    #[cfg(not(all(feature = "runtime-monoio", target_os = "linux")))]
    let _ = enable_uring;

    // `io_uring` is either disabled or not supported
    #[cfg(feature = "runtime-monoio")]
    let rt_inner = RuntimeInner::MonoioLegacy(
      monoio::RuntimeBuilder::<monoio::LegacyDriver>::new()
        .enable_all()
        .attach_thread_pool(Box::new(BlockingThreadPool))
        .build()?,
    );
    #[cfg(feature = "runtime-tokio")]
    let rt_inner = RuntimeInner::Tokio(tokio::runtime::Builder::new_current_thread().enable_all().build()?);

    Ok(Self {
      inner: rt_inner,
      io_uring_enable_configured,
    })
  }

  /// Creates a new asynchronous runtime using only Tokio
  pub fn new_runtime_tokio_only() -> Result<Self, std::io::Error> {
    Ok(Self {
      inner: RuntimeInner::TokioOnly(tokio::runtime::Builder::new_current_thread().enable_all().build()?),
      io_uring_enable_configured: None,
    })
  }

  /// Return the OS error if `io_uring` couldn't be configured
  #[allow(dead_code)]
  pub fn return_io_uring_error(&self) -> Option<std::io::Error> {
    self.io_uring_enable_configured.map(std::io::Error::from_raw_os_error)
  }

  /// Run a future on the runtime
  pub fn run(&mut self, fut: impl Future) {
    match self.inner {
      #[cfg(all(feature = "runtime-monoio", target_os = "linux"))]
      RuntimeInner::MonoioIouring(ref mut rt) => rt.block_on(fut),
      #[cfg(feature = "runtime-monoio")]
      RuntimeInner::MonoioLegacy(ref mut rt) => rt.block_on(fut),
      #[cfg(feature = "runtime-tokio")]
      RuntimeInner::Tokio(ref mut rt) => rt.block_on(async move {
        let local_set = tokio::task::LocalSet::new();
        local_set.run_until(future).await;
      }),
      RuntimeInner::TokioOnly(ref mut rt) => rt.block_on(fut),
    };
  }
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
