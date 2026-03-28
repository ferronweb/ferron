//! Runtime initialization for Ferron
//!
//! Provides unified initialization for primary (thread-per-core) and secondary
//! (multi-threaded) runtimes.

use ferron_common::ModuleContext;
use ferron_registry::Registry;
use std::sync::Arc;
use tokio::runtime::{Builder, Handle, Runtime};

/// Runtime handles for both primary and secondary runtimes
pub struct RuntimeHandles {
    pub primary: Handle,
    pub secondary: Handle,
}

/// Primary runtime configuration for thread-per-core architecture
///
/// Each thread runs a single-threaded runtime with LocalSet for spawning
/// thread-local tasks.
pub struct PrimaryRuntime {
    runtimes: Vec<Option<Runtime>>,
    handles: Vec<Handle>,
}

impl PrimaryRuntime {
    /// Create a new primary runtime with one thread per core
    pub fn new() -> Self {
        let num_cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);

        let mut runtimes = Vec::with_capacity(num_cores);
        let mut handles = Vec::with_capacity(num_cores);

        for _ in 0..num_cores {
            let rt = Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Failed to create primary runtime");
            let handle = rt.handle().clone();
            runtimes.push(Some(rt));
            handles.push(handle);
        }

        Self { runtimes, handles }
    }

    /// Get the number of primary runtime threads
    pub fn num_threads(&self) -> usize {
        self.handles.len()
    }

    /// Get a handle to a specific primary runtime thread
    pub fn handle(&self, index: usize) -> &Handle {
        &self.handles[index]
    }

    /// Get all handles
    pub fn handles(&self) -> &[Handle] {
        &self.handles
    }

    /// Run a function on each primary runtime thread
    pub fn run_on_all<F>(&mut self, f: F)
    where
        F: Fn(usize) + Send + Sync + Clone + 'static,
    {
        let mut join_handles = Vec::new();

        for (index, rt_option) in self.runtimes.iter_mut().enumerate() {
            let rt = rt_option.take().expect("Runtime already taken");
            let f = f.clone();

            let join_handle = std::thread::spawn(move || {
                let local = tokio::task::LocalSet::new();
                local.block_on(&rt, async {
                    f(index);
                });
            });

            join_handles.push(join_handle);
        }

        for handle in join_handles {
            handle.join().expect("Primary runtime thread panicked");
        }
    }
}

impl Default for PrimaryRuntime {
    fn default() -> Self {
        Self::new()
    }
}

/// Secondary runtime configuration for multi-threaded tasks
///
/// This is a regular multi-threaded Tokio runtime for control plane work
/// and other background tasks.
pub struct SecondaryRuntime {
    _runtime: Runtime,
    handle: Handle,
}

impl SecondaryRuntime {
    /// Create a new secondary multi-threaded runtime
    pub fn new() -> Self {
        let rt = Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed to create secondary runtime");
        let handle = rt.handle().clone();

        Self {
            _runtime: rt,
            handle,
        }
    }

    /// Get the runtime handle
    pub fn handle(&self) -> &Handle {
        &self.handle
    }
}

impl Default for SecondaryRuntime {
    fn default() -> Self {
        Self::new()
    }
}

/// Initialize both primary and secondary runtimes and start all modules
pub fn run_with_runtimes(registry: Arc<Registry>) {
    let mut primary = PrimaryRuntime::new();
    let secondary = SecondaryRuntime::new();

    let handles = RuntimeHandles {
        primary: primary.handle(0).clone(),
        secondary: secondary.handle().clone(),
    };

    // Start all modules with runtime handles
    // Modules are started on the secondary runtime, but can spawn on both
    let ctx = ModuleContext::new(handles.primary, handles.secondary);

    for module in registry.modules() {
        println!("Starting module: {}", module.name());
        let future = module.start(ctx.clone());
        secondary.handle().spawn(future);
    }

    // Run primary runtime threads
    // Each thread will handle request processing
    primary.run_on_all(|index| {
        println!("Primary runtime thread {} started", index);

        // Keep the thread alive
        // In a real implementation, this would be where LocalSet tasks run
        loop {
            std::thread::park();
        }
    });
}
