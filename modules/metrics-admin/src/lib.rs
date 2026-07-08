use std::sync::Arc;

use ferron_core::loader::ModuleLoader;
use ferron_core::registry::Registry;
use ferron_core::Module;
use ferron_observability::build_composite_sink;

mod collect;

/// Module loader for the admin metrics collector.
#[derive(Default)]
pub struct AdminMetricsModuleLoader {
    cache: Option<Arc<AdminMetricsModule>>,
}

impl ModuleLoader for AdminMetricsModuleLoader {
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
            let module = Arc::new(AdminMetricsModule::new(event_sink));
            self.cache = Some(module.clone());
            modules.push(module);
        }

        Ok(())
    }
}

/// The process metrics module that spawns the background collection task.
pub struct AdminMetricsModule {
    event_sink: Arc<ferron_observability::CompositeEventSink>,
    cancel_token: tokio_util::sync::CancellationToken,
}

impl AdminMetricsModule {
    fn new(event_sink: Arc<ferron_observability::CompositeEventSink>) -> Self {
        Self {
            event_sink,
            cancel_token: tokio_util::sync::CancellationToken::new(),
        }
    }
}

impl Module for AdminMetricsModule {
    fn name(&self) -> &str {
        "metrics-admin"
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
            collect::collect_admin_metrics(event_sink, cancel_token).await;
        });

        Ok(())
    }
}

impl Drop for AdminMetricsModule {
    fn drop(&mut self) {
        self.cancel_token.cancel();
    }
}
