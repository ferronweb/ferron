//! Multi-threaded async runtime supporting both io_uring and traditional async I/O.
//!
//! The runtime consists of:
//! - Primary tasks: Executed on dedicated threads using vibeio with optional io_uring
//! - Secondary tasks: Executed on a tokio multi-threaded runtime

use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, LazyLock},
};

use vibeio::blocking::DefaultBlockingThreadPool;

use crate::log_warn;

static IO_URING_FAILED_WARNING_LOGGED: std::sync::Once = std::sync::Once::new();

/// Manages async task execution across primary and secondary runtimes.
///
/// The runtime uses a dual-runtime model:
/// - Primary threads run vibeio tasks (one per CPU core) with optional io_uring
/// - Secondary runtime is a tokio multi-threaded executor for other tasks
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
    /// * `io_uring_enabled` - Whether to enable io_uring on primary threads (if supported)
    ///
    /// # Errors
    ///
    /// Returns `std::io::Error` if runtime creation fails.
    pub fn new(io_uring_enabled: bool) -> Result<Self, std::io::Error> {
        // Spawn multiple threads (with pinning to each CPU core) to run primary tasks
        let core_ids = core_affinity::get_core_ids();
        let available_parallelism = core_ids.as_ref().map_or_else(
            || std::thread::available_parallelism().unwrap().get(),
            |core_ids| core_ids.len(),
        );
        let mut primary_task_channels = Vec::with_capacity(available_parallelism);

        for i in 0..available_parallelism {
            let core_id = core_ids
                .as_ref()
                .map(|core_ids| core_ids[i % core_ids.len()]);

            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<
                Arc<dyn Fn() -> Pin<Box<dyn Future<Output = ()>>> + Send + Sync + 'static>,
            >();
            std::thread::spawn(move || {
                if let Some(core_id) = core_id {
                    let _ = core_affinity::set_for_current(core_id);
                }
                let use_io_uring = io_uring_enabled && vibeio::util::supports_io_uring();
                let rt = vibeio::RuntimeBuilder::new()
                    .enable_timer(true)
                    .blocking_pool(Box::new(BlockingThreadPool))
                    .build()
                    .expect("failed to create vibeio runtime for primary tasks");

                rt.block_on(async move {
                    if use_io_uring && !vibeio::util::supports_completion() {
                        IO_URING_FAILED_WARNING_LOGGED.call_once(|| {
                            log_warn!(
                                "io_uring is enabled in configuration and \
                                 supported on this system, but failed to \
                                 initialize io_uring; falling back to epoll"
                            );
                        });
                    }
                    while let Some(task_factory) = rx.recv().await {
                        vibeio::spawn((task_factory.as_ref())());
                    }
                });
            });
            primary_task_channels.push(tx);
        }

        Ok(Self {
            primary_task_channels,
            secondary_runtime: tokio::runtime::Builder::new_multi_thread()
                .worker_threads((available_parallelism / 2).max(1))
                .enable_all()
                .build()?,
        })
    }

    /// Spawn a task factory to all primary threads.
    ///
    /// The factory will be called once on each primary thread, allowing
    /// thread-local initialization for each concurrent task.
    pub fn spawn_primary_task<F>(&mut self, task_factory: F)
    where
        F: Fn() -> Pin<Box<dyn Future<Output = ()>>> + Send + Sync + 'static,
    {
        let task_factory = Arc::new(task_factory);
        for channel in &self.primary_task_channels {
            let _ = channel.send(task_factory.clone());
        }
    }

    /// Spawn a task on the secondary (tokio) runtime.
    pub fn spawn_secondary_task<F>(&self, task: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.secondary_runtime.spawn(task);
    }

    /// Block the current thread and execute a future to completion.
    pub fn block_on<F>(&self, task: F) -> F::Output
    where
        F: Future + 'static,
    {
        self.secondary_runtime.block_on(task)
    }
}

static GLOBAL_BLOCKING_POOL: LazyLock<DefaultBlockingThreadPool> =
    LazyLock::new(|| DefaultBlockingThreadPool::with_max_threads(1536));

/// A global blocking thread pool for `vibeio`
struct BlockingThreadPool;

impl vibeio::blocking::BlockingThreadPool for BlockingThreadPool {
    #[inline]
    fn spawn(&self, task: Box<dyn FnOnce() + Send + 'static>) {
        GLOBAL_BLOCKING_POOL.spawn(task);
    }
}
