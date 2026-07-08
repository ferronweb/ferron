use std::sync::Arc;

use ferron_core::loader::ModuleLoader;
use ferron_core::registry::Registry;
use ferron_core::Module;
use ferron_observability::build_composite_sink;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod windows;

/// Module loader for the process metrics collector.
///
/// This module discovers available observability providers from the registry,
/// materializes their sinks using the actual global observability configuration,
/// and spawns a background task that periodically emits process-level metrics
/// (CPU time, CPU utilization, memory usage) through the composite event sink.
#[derive(Default)]
pub struct ProcessMetricsModuleLoader {
    cache: Option<Arc<ProcessMetricsModule>>,
}

impl ModuleLoader for ProcessMetricsModuleLoader {
    fn register_modules(
        &mut self,
        registry: Arc<Registry>,
        modules: &mut Vec<Arc<dyn Module>>,
        config: Arc<ferron_core::config::ServerConfiguration>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Build the composite sink from all observability providers using the
        // actual global observability configuration
        let event_sink = build_composite_sink(&registry, &config.global_config, None)?;

        if self.cache.is_none() {
            let module = Arc::new(ProcessMetricsModule::new(event_sink));
            self.cache = Some(module.clone());
            modules.push(module);
        }

        Ok(())
    }
}

/// The process metrics module that spawns the background collection task.
pub struct ProcessMetricsModule {
    event_sink: Arc<ferron_observability::CompositeEventSink>,
    cancel_token: tokio_util::sync::CancellationToken,
}

impl ProcessMetricsModule {
    fn new(event_sink: Arc<ferron_observability::CompositeEventSink>) -> Self {
        Self {
            event_sink,
            cancel_token: tokio_util::sync::CancellationToken::new(),
        }
    }
}

impl Module for ProcessMetricsModule {
    fn name(&self) -> &str {
        "metrics-process"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn start(
        &self,
        runtime: &mut ferron_core::runtime::Runtime,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let cancel_token = self.cancel_token.clone();
        let event_sink = self.event_sink.clone();

        runtime.spawn_secondary_task(async move {
            run_metrics_collection(event_sink, cancel_token).await;
        });

        Ok(())
    }
}

impl Drop for ProcessMetricsModule {
    fn drop(&mut self) {
        self.cancel_token.cancel();
    }
}

async fn run_metrics_collection(
    event_sink: Arc<ferron_observability::CompositeEventSink>,
    cancel_token: tokio_util::sync::CancellationToken,
) {
    #[cfg(target_os = "linux")]
    linux::collect_process_metrics(event_sink, cancel_token).await;
    #[cfg(windows)]
    windows::collect_process_metrics(event_sink, cancel_token).await;
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = event_sink; // Suppress unused variable warning
        cancel_token.cancelled().await;
    }
}
