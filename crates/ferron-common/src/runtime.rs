use std::{future::Future, pin::Pin, sync::Arc};

pub struct Runtime {
    primary_task_factories: Vec<Arc<dyn Fn() -> Pin<Box<dyn Future<Output = ()>>> + Send + Sync>>,
    secondary_runtime: tokio::runtime::Runtime,
}

impl Runtime {
    pub fn new() -> Result<Self, std::io::Error> {
        Ok(Self {
            primary_task_factories: Vec::new(),
            secondary_runtime: tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?,
        })
    }

    pub fn spawn_primary_task<F>(&mut self, task_factory: F)
    where
        F: Fn() -> Pin<Box<dyn Future<Output = ()>>> + Send + Sync + 'static,
    {
        self.primary_task_factories.push(Arc::new(task_factory));
    }

    pub fn spawn_secondary_task<F>(&self, task: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.secondary_runtime.spawn(task);
    }

    pub fn run(self) -> Result<(), std::io::Error> {
        // Spawn multiple threads (as many threads as available parallelism
        let available_parallelism = std::thread::available_parallelism()?.get();
        let mut threads = Vec::with_capacity(available_parallelism);

        let factories = Arc::new(self.primary_task_factories);
        for _ in 0..available_parallelism {
            // TODO: replace Tokio with `vibeio`...
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;

            let factories = factories.clone();
            threads.push(std::thread::spawn(move || {
                rt.block_on(async move {
                    tokio::task::LocalSet::new()
                        .run_until(async move {
                            let join_handles = factories
                                .iter()
                                .map(|task_factory| tokio::task::spawn_local(task_factory()))
                                .collect::<Vec<_>>();
                            for join_handle in join_handles {
                                let _ = join_handle.await;
                            }
                        })
                        .await;
                });
            }));
        }

        for thread in threads {
            thread.join().map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Thread panicked: {:?}", e),
                )
            })?;
        }

        Ok(())
    }
}
