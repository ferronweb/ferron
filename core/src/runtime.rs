//! Multi-threaded async runtime supporting both io_uring and traditional async I/O.
//!
//! The runtime uses a dual-model design:
//!
//! - **Primary tasks** run on dedicated per-CPU threads via
//!   [zincio](https://crates.io/crates/zincio) with optional `io_uring`
//!   on Linux. These are used for high-throughput I/O work (listeners,
//!   connection loops).
//! - **Secondary tasks** run on a standard tokio multi-threaded executor.
//!   These are used for background work that does not need dedicated CPU
//!   threads (metrics aggregation, certificate renewal, etc.).
//!
//! # Usage
//!
//! ```ignore
//! use ferron_core::runtime::Runtime;
//!
//! let mut runtime = Runtime::new(Default::default())?;
//!
//! // Spawn a primary task (runs once per CPU thread)
//! runtime.spawn_primary_task(move || {
//!     Box::pin(async move {
//!         // ... high-throughput I/O ...
//!     })
//! });
//!
//! // Spawn a secondary task (runs on tokio)
//! runtime.spawn_secondary_task(async {
//!     // ... background work ...
//! });
//! ```

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, LazyLock};

use zincio::blocking::DefaultBlockingThreadPool;

use crate::log_warn;

static IO_URING_FAILED_WARNING_LOGGED: std::sync::Once = std::sync::Once::new();

/// Manages async task execution across primary and secondary runtimes.
///
/// The primary runtime spawns one thread per CPU core (pinned via
/// `core_affinity`). Each thread runs a zincio executor with optional
/// `io_uring` on Linux. The secondary runtime is a standard tokio
/// multi-threaded executor.
///
/// Modules receive a `&mut Runtime` in
/// [`Module::start`](crate::Module::start) and use it to spawn tasks.
#[allow(clippy::type_complexity)]
pub struct Runtime {
    primary_task_channels: Vec<
        tokio::sync::mpsc::UnboundedSender<
            Arc<dyn Fn() -> Pin<Box<dyn Future<Output = ()>>> + Send + Sync + 'static>,
        >,
    >,
    secondary_runtime: tokio::runtime::Runtime,
}

impl Runtime {
    /// Create a new runtime with primary threads equal to available parallelism.
    ///
    /// # Arguments
    ///
    /// * `settings` -- Runtime configuration (e.g. whether to enable `io_uring`).
    ///
    /// # Errors
    ///
    /// Returns `std::io::Error` if the secondary tokio runtime fails to build.
    pub fn new(settings: RuntimeSettings) -> Result<Self, std::io::Error> {
        // Spawn multiple threads (with pinning to each CPU core) to run primary tasks
        let core_ids = core_affinity::get_core_ids();
        let available_parallelism = core_ids.as_ref().map_or_else(
            || std::thread::available_parallelism().map_or(1, |ap| ap.get()),
            |core_ids| core_ids.len(),
        );
        let mut primary_task_channels = Vec::with_capacity(available_parallelism);

        {
            let mut runtime_metrics = crate::admin::ADMIN_METRICS.runtime_metrics.write();
            runtime_metrics.primary_threads = available_parallelism;
        }

        for i in 0..available_parallelism {
            let core_id = core_ids
                .as_ref()
                .map(|core_ids| core_ids[i % core_ids.len()]);

            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<
                Arc<dyn Fn() -> Pin<Box<dyn Future<Output = ()>>> + Send + Sync + 'static>,
            >();
            let io_uring_enabled = settings.io_uring_enabled;
            std::thread::Builder::new()
                .name("Primary runtime".to_string())
                .spawn(move || {
                    if let Some(core_id) = core_id {
                        let _ = core_affinity::set_for_current(core_id);
                    }
                    let use_io_uring = io_uring_enabled && zincio::util::supports_io_uring();

                    #[allow(unused_mut)]
                    let mut rt_builder = zincio::RuntimeBuilder::new()
                        .enable_timer(true)
                        .blocking_pool(Box::new(BlockingThreadPool));

                    #[cfg(target_os = "linux")]
                    if !use_io_uring {
                        // Disable `io_uring` driver manually
                        rt_builder = rt_builder.driver(zincio::DriverKind::Mio);
                    } else {
                        let mut runtime_metrics =
                            crate::admin::ADMIN_METRICS.runtime_metrics.write();
                        runtime_metrics.io_uring_supported = true;
                    }

                    let rt = rt_builder
                        .build()
                        .expect("failed to create zincio runtime for primary tasks");

                    rt.block_on(async move {
                        {
                            let mut runtime_metrics =
                                crate::admin::ADMIN_METRICS.runtime_metrics.write();
                            if use_io_uring && !zincio::util::supports_completion() {
                                IO_URING_FAILED_WARNING_LOGGED.call_once(|| {
                                    log_warn!(
                                        "io_uring is enabled in configuration and \
                                 supported on this system, but failed to \
                                 initialize io_uring; falling back to epoll"
                                    );
                                });
                                runtime_metrics.io_uring_runtime_enabled = false;
                            } else {
                                runtime_metrics.io_uring_runtime_enabled = use_io_uring;
                            }
                        }
                        while let Some(task_factory) = rx.recv().await {
                            zincio::spawn_detached((task_factory.as_ref())());
                        }
                    });
                })?;
            primary_task_channels.push(tx);
        }

        Ok(Self {
            primary_task_channels,
            secondary_runtime: tokio::runtime::Builder::new_multi_thread()
                .thread_name("Secondary runtime".to_string())
                .worker_threads((available_parallelism / 2).max(1))
                .enable_all()
                .build()?,
        })
    }

    /// Spawn a task factory to all primary threads.
    ///
    /// The factory is called once per primary thread (one per CPU core),
    /// allowing thread-local initialization. The returned future runs on the
    /// zincio executor with optional `io_uring`.
    ///
    /// Use this for high-throughput I/O work such as TCP accept loops.
    ///
    /// # Arguments
    ///
    /// * `task_factory` -- A closure that returns a pinned future. Called
    ///   once per primary thread.
    pub fn spawn_primary_task<F>(&mut self, task_factory: F)
    where
        F: Fn() -> Pin<Box<dyn Future<Output = ()>>> + Send + Sync + 'static,
    {
        let task_factory = Arc::new(task_factory);
        for channel in &self.primary_task_channels {
            let _ = channel.send(task_factory.clone());
        }
    }

    /// Number of primary (per-CPU) threads backing this runtime.
    #[inline]
    pub fn primary_thread_count(&self) -> usize {
        self.primary_task_channels.len()
    }

    /// Spawn a task factory on a single primary thread, indexed by `index`.
    ///
    /// Unlike [`spawn_primary_task`](Self::spawn_primary_task), the factory
    /// runs exactly once, on the primary thread at `index`. Use this to pin
    /// a distinct task (such as one of several QUIC endpoints) to a specific
    /// CPU.
    ///
    /// If `index` is out of range, the call is silently ignored.
    pub fn spawn_primary_task_on<F>(&mut self, index: usize, task_factory: F)
    where
        F: Fn() -> Pin<Box<dyn Future<Output = ()>>> + Send + Sync + 'static,
    {
        let task_factory = Arc::new(task_factory);
        if let Some(channel) = self.primary_task_channels.get(index) {
            let _ = channel.send(task_factory.clone());
        }
    }

    /// Spawn a task on the secondary (tokio) runtime.
    ///
    /// Use this for background work that does not need dedicated CPU threads:
    /// metrics collection, certificate renewal, log processing, etc.
    pub fn spawn_secondary_task<F>(&self, task: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.secondary_runtime.spawn(task);
    }

    /// Block the current thread and execute a future to completion.
    ///
    /// Delegates to the secondary (tokio) runtime. Useful for synchronous
    /// initialization code that needs to await a future.
    pub fn block_on<F>(&self, task: F) -> F::Output
    where
        F: Future + 'static,
    {
        self.secondary_runtime.block_on(task)
    }
}

/// Settings for the Ferron runtime.
#[derive(Default)]
#[non_exhaustive]
pub struct RuntimeSettings {
    /// Whether to enable `io_uring` on primary threads (if the kernel
    /// supports it). If initialization fails, Ferron falls back to `epoll`
    /// and logs a warning. Default: disabled.
    pub io_uring_enabled: bool,
}

static GLOBAL_BLOCKING_POOL: LazyLock<DefaultBlockingThreadPool> =
    LazyLock::new(|| DefaultBlockingThreadPool::with_max_threads(1536));

/// A global blocking thread pool for `zincio`
struct BlockingThreadPool;

impl zincio::blocking::BlockingThreadPool for BlockingThreadPool {
    #[inline]
    fn spawn(&self, task: Box<dyn FnOnce() + Send + 'static>) {
        GLOBAL_BLOCKING_POOL.spawn(task);
    }
}
