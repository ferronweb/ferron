use std::{future::Future, pin::Pin, sync::Arc};

use crate::log_warn;

static IO_URING_FAILED_WARNING_LOGGED: std::sync::Once = std::sync::Once::new();

pub struct Runtime {
    primary_task_channels: Vec<
        tokio::sync::mpsc::UnboundedSender<
            Arc<dyn Fn() -> Pin<Box<dyn Future<Output = ()>>> + Send + Sync + 'static>,
        >,
    >,
    secondary_runtime: tokio::runtime::Runtime,
}

impl Runtime {
    pub fn new(io_uring_enabled: bool) -> Result<Self, std::io::Error> {
        // Spawn multiple threads (as many threads as available parallelism
        let available_parallelism = std::thread::available_parallelism()?.get();
        let mut primary_task_channels = Vec::with_capacity(available_parallelism);

        for _ in 0..available_parallelism {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<
                Arc<dyn Fn() -> Pin<Box<dyn Future<Output = ()>>> + Send + Sync + 'static>,
            >();
            std::thread::spawn(move || {
                // TODO: option to enable/disable `io_uring` support in vibeio
                let use_io_uring = io_uring_enabled && vibeio::util::supports_io_uring();
                let rt = vibeio::RuntimeBuilder::new()
                    .enable_timer(true)
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
                .enable_all()
                .build()?,
        })
    }

    pub fn spawn_primary_task<F>(&mut self, task_factory: F)
    where
        F: Fn() -> Pin<Box<dyn Future<Output = ()>>> + Send + Sync + 'static,
    {
        let task_factory = Arc::new(task_factory);
        for channel in &self.primary_task_channels {
            let _ = channel.send(task_factory.clone());
        }
    }

    pub fn spawn_secondary_task<F>(&self, task: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.secondary_runtime.spawn(task);
    }

    pub fn block_on<F>(&self, task: F) -> F::Output
    where
        F: Future + 'static,
    {
        self.secondary_runtime.block_on(task)
    }
}
