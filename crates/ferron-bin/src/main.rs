use ferron_common::loader::ModuleLoader;
use ferron_common::registry::RegistryBuilder;
use ferron_common::runtime::Runtime;
use ferron_http::{BasicHttpModule, HelloStage, LoggingStage, NotFoundStage};
use ferron_http::{BasicHttpModuleLoader, HttpContext};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut loaders = vec![BasicHttpModuleLoader];
    let mut registry_builder = RegistryBuilder::new();

    for loader in &mut loaders {
        registry_builder = loader.register_stages(registry_builder);
    }

    let registry = registry_builder.build();
    for loader in &mut loaders {
        loader.register_modules(&registry);
    }

    let mut runtime = Runtime::new()?;

    // Start all modules
    for module in registry.modules() {
        println!("Starting module: {}", module.name());
        module.start(&mut runtime)?;
    }

    // Run the runtime
    runtime.run()?;

    Ok(())
}
