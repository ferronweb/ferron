use ferron_http::{BasicHttpModule, HelloStage, LoggingStage, NotFoundStage};
use ferron_module_api::ProvidesServer;
use ferron_registry::RegistryBuilder;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    // =====================
    // Create registry with pre-loaded stages
    // =====================
    // Stages define their own names and constraints via the Stage trait

    let registry = RegistryBuilder::new()
        // Register stages (names and constraints defined in the stage structs)
        .with_stage::<ferron_core::http::HttpContext, _>(|| Arc::new(LoggingStage::default()))
        .with_stage::<ferron_core::http::HttpContext, _>(|| Arc::new(HelloStage::default()))
        .with_stage::<ferron_core::http::HttpContext, _>(|| Arc::new(NotFoundStage::default()))
        .build();

    // Create the HTTP module from the registry (pipeline is built from registered stages)
    let http_module = BasicHttpModule::from_registry(&registry);

    // Start all server modules
    if let Some(server) = http_module.server() {
        server.start().await;
    }
}
