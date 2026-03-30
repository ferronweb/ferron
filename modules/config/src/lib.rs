use std::collections::HashMap;

use async_trait::async_trait;
use ferron_core::{
    config::{ServerConfigurationBlock, ServerConfigurationPort, adapter::ConfigurationAdapter},
    loader::ModuleLoader,
};

struct BlankConfigurationAdapter;

impl BlankConfigurationAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl ConfigurationAdapter for BlankConfigurationAdapter {
    fn adapt(
        &self,
        _params: &std::collections::HashMap<String, String>,
    ) -> Result<
        (
            ferron_core::config::ServerConfiguration,
            Box<dyn ferron_core::config::adapter::ConfigurationWatcher>,
        ),
        Box<dyn std::error::Error>,
    > {
        Ok((
            ferron_core::config::ServerConfigurationBuilder::new()
                .global_config(ferron_core::config::ServerConfigurationBlockBuilder::new().directive("runtime", ferron_core::config::ServerConfigurationDirectiveEntry {
                    args: vec![],
                    children: Some(ferron_core::config::ServerConfigurationBlockBuilder::new().directive(
                        "io_uring",
                        ferron_core::config::ServerConfigurationDirectiveEntry {
                            args: vec![ferron_core::config::ServerConfigurationValue::Boolean(true, None)],
                            children: None,
                            ..Default::default()
                        },
                    ).build()),
                    ..Default::default()
                }).build())
                .port(
                    "http",
                    ServerConfigurationPort {
                        port: 8080,
                        hosts: vec![],
                    },
                )
                .build(),
            Box::new(BlankConfigurationWatcher),
        ))
    }
}

struct BlankConfigurationWatcher;

#[async_trait]
impl ferron_core::config::adapter::ConfigurationWatcher for BlankConfigurationWatcher {
    async fn watch(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        std::future::pending().await
    }
}

pub struct BlankConfigurationAdapterModuleLoader;

impl ModuleLoader for BlankConfigurationAdapterModuleLoader {
    fn register_configuration_adapters(
        &mut self,
        registry: &mut HashMap<&'static str, Box<dyn ConfigurationAdapter>>,
    ) {
        registry.insert("blank", Box::new(BlankConfigurationAdapter));
    }
}
