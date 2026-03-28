use ferron_core::http::HttpContext;
use ferron_http::{BasicHttpModule, HelloStage, LoggingStage, NotFoundStage};
use ferron_registry::ModuleRegistryBuilder;
use ferron_runtime::pipeline::Pipeline;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    // =====================
    // Create module registry with pre-loaded stages and modules
    // =====================

    let registry = ModuleRegistryBuilder::new()
        // Pre-load HTTP stages
        .with_http_stage("logging", || Arc::new(LoggingStage))
        .with_http_stage("hello", || Arc::new(HelloStage))
        .with_http_stage("not_found", || Arc::new(NotFoundStage))
        // Register modules (pipeline will be built from registered stages)
        .with_module({
            // Build pipeline from pre-loaded stages
            let pipeline = Pipeline::<HttpContext>::new();
            BasicHttpModule::new(pipeline)
        })
        .build();

    // =====================
    // Orchestrate: build pipelines and start all servers
    // =====================

    let handles = registry.orchestrate();

    // Wait forever
    futures_util::future::join_all(handles).await;
}
