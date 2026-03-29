use std::{future::Future, pin::Pin, sync::Arc};

pub struct Runtime {
    primary_task_channels: Vec<
        tokio::sync::mpsc::UnboundedSender<
            Arc<dyn Fn() -> Pin<Box<dyn Future<Output = ()>>> + Send + Sync + 'static>,
        >,
    >,
    secondary_runtime: tokio::runtime::Runtime,
}

impl Runtime {
    pub fn new() -> Result<Self, std::io::Error> {
        // Spawn multiple threads (as many threads as available parallelism
        let available_parallelism = std::thread::available_parallelism()?.get();
        let mut primary_task_channels = Vec::with_capacity(available_parallelism);

        for _ in 0..available_parallelism {
            // TODO: replace Tokio with `vibeio`...
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;

            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<
                Arc<dyn Fn() -> Pin<Box<dyn Future<Output = ()>>> + Send + Sync + 'static>,
            >();
            std::thread::spawn(move || {
                rt.block_on(async move {
                    tokio::task::LocalSet::new()
                        .run_until(async move {
                            while let Some(task_factory) = rx.recv().await {
                                tokio::task::spawn_local((task_factory.as_ref())());
                            }
                        })
                        .await;
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
