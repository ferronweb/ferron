use ferron_common::loader::ModuleLoader;
use ferron_common::registry::{Registry, RegistryBuilder};
use ferron_common::runtime::Runtime;
use ferron_http::BasicHttpModuleLoader;
use std::sync::Arc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut loaders: Vec<Box<dyn ModuleLoader>> = vec![Box::new(BasicHttpModuleLoader)];
    let mut registry_builder = RegistryBuilder::new();

    for loader in &mut loaders {
        registry_builder = loader.register_stages(registry_builder);
    }

    let registry = registry_builder.build();

    load_modules(loaders, registry)?;

    Ok(())
}

fn load_modules(
    mut loaders: Vec<Box<dyn ModuleLoader>>,
    registry: Arc<Registry>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut modules = Vec::new();

    for loader in &mut loaders {
        loader.register_modules(&registry, &mut modules);
    }

    let mut runtime = Runtime::new()?;

    // Start all modules
    for module in modules {
        println!("Starting module: {}", module.name());
        module.start(&mut runtime)?;
    }

    // Run the runtime
    runtime.run()?;

    Ok(())
}
