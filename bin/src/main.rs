use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use ferron_config::BlankConfigurationAdapterModuleLoader;
use ferron_core::config::adapter::ConfigurationAdapter;
use ferron_core::loader::ModuleLoader;
use ferron_core::registry::{Registry, RegistryBuilder};
use ferron_core::runtime::Runtime;
use ferron_http::BasicHttpModuleLoader;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut loaders: Vec<Box<dyn ModuleLoader>> = vec![
        Box::new(BasicHttpModuleLoader::default()),
        Box::new(BlankConfigurationAdapterModuleLoader),
    ];

    let mut config_registry = HashMap::new();
    let mut registry_builder = RegistryBuilder::new();
    let mut global_validator_registry = Vec::new();
    let mut per_protocol_validator_registry = HashMap::new();
    for loader in &mut loaders {
        loader.register_per_protocol_configuration_validators(&mut per_protocol_validator_registry);
        loader.register_global_configuration_validators(&mut global_validator_registry);
        loader.register_configuration_adapters(&mut config_registry);
        registry_builder = loader.register_stages(registry_builder);
    }
    let registry = registry_builder.build();

    // TODO: choose configuration adapter from CLI arguments
    let config_adapter_name = "blank";
    let config_adapter_params = HashMap::new();

    let config_adapter = config_registry
        .get(config_adapter_name)
        .ok_or(anyhow::anyhow!("Configuration adapter not found"))?;

    load_modules(
        loaders,
        registry,
        config_adapter.as_ref(),
        config_adapter_params,
        global_validator_registry,
        per_protocol_validator_registry,
    )?;

    Ok(())
}

fn load_modules(
    mut loaders: Vec<Box<dyn ModuleLoader>>,
    registry: Arc<Registry>,
    config_adapter: &dyn ConfigurationAdapter,
    config_adapter_params: HashMap<String, String>,
    global_validator_registry: Vec<Box<dyn ferron_core::config::validator::ConfigurationValidator>>,
    per_protocol_validator_registry: HashMap<
        &'static str,
        Box<dyn ferron_core::config::validator::ConfigurationValidator>,
    >,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut runtime = Runtime::new()?;

    loop {
        let (mut config, mut watcher) = config_adapter.adapt(&config_adapter_params)?;

        let mut config_blocks_registry = HashMap::new();
        let mut modules = Vec::new();

        // configuration validation
        // TODO: used directives and error reporting for invalid directives
        for validator in &global_validator_registry {
            validator.validate_block(&config.global_config, &mut HashSet::new())?;
        }
        for loader in &mut loaders {
            loader.register_per_protocol_configuration_blocks(&config, &mut config_blocks_registry);
        }
        for (protocol, blocks) in &config_blocks_registry {
            if let Some(validator) = per_protocol_validator_registry.get(protocol) {
                for block in blocks {
                    validator.validate_block(block.1, &mut HashSet::new())?;
                }
            }
        }
        // TODO: check unused directives

        for loader in &mut loaders {
            loader.register_modules(&registry, &mut modules, &mut config);
        }

        // Start all modules
        for module in modules {
            println!("Starting module: {}", module.name());
            module.start(&mut runtime)?;
        }

        // Run the runtime
        runtime.block_on(async move { watcher.watch().await })?;
    }
}
