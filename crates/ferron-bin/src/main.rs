use ferron_common::runtime::Runtime;
use ferron_http::HttpContext;
use ferron_http::{BasicHttpModule, HelloStage, LoggingStage, NotFoundStage};
use ferron_registry::RegistryBuilder;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    // =====================
    // Create registry with stages and modules
    // =====================
    // Stages define their own names and constraints via the Stage trait.
    // The HTTP module will build an ordered pipeline from registered stages
    // using DAG-based topological sort.

    let registry = RegistryBuilder::new()
        // Register stages (names and constraints defined in the stage structs)
        // Order will be determined by constraints: logging -> hello -> not_found
        .with_stage::<HttpContext, _>(|| Arc::new(LoggingStage::default()))
        .with_stage::<HttpContext, _>(|| Arc::new(HelloStage::default()))
        .with_stage::<HttpContext, _>(|| Arc::new(NotFoundStage::default()))
        .build();

    // Create the HTTP module from the registry
    // The module builds its pipeline from the registered stages using DAG ordering
    let http_module = BasicHttpModule::from_registry(&registry);
    registry.register_module(http_module);

    let mut runtime = Runtime::new().expect("Failed to create runtime"); // TODO: proper error handling

    // Start all modules
    for module in registry.modules() {
        println!("Starting module: {}", module.name());
        module.start(&mut runtime).expect("Failed to start module");
    }

    // Run the runtime
    runtime.run().expect("Failed to run runtime");
}
