use std::collections::HashMap;

use async_trait::async_trait;
use ferron_core::{config::adapter::ConfigurationAdapter, loader::ModuleLoader};

struct JsonConfigurationAdapter;

impl ConfigurationAdapter for JsonConfigurationAdapter {
    fn adapt(
        &self,
        params: &std::collections::HashMap<String, String>,
    ) -> Result<
        (
            ferron_core::config::ServerConfiguration,
            Box<dyn ferron_core::config::adapter::ConfigurationWatcher>,
        ),
        Box<dyn std::error::Error>,
    > {
        let filename = params.get("file").ok_or(anyhow::anyhow!(
            "'file' parameter is required for 'ferronconf' configuration adapter"
        ))?;
        let file_contents = std::fs::read_to_string(filename)
            .map_err(|e| anyhow::anyhow!("Failed to read configuration file '{filename}': {e}",))?;

        Ok((
            serde_json::from_str(&file_contents).map_err(|e| {
                anyhow::anyhow!("Failed to parse configuration file '{filename}': {e}",)
            })?,
            Box::new(JsonConfigurationWatcher),
        ))
    }

    #[inline]
    fn file_extension(&self) -> Vec<&'static str> {
        vec!["json"]
    }
}

struct JsonConfigurationWatcher;

#[async_trait]
impl ferron_core::config::adapter::ConfigurationWatcher for JsonConfigurationWatcher {
    async fn watch(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        std::future::pending().await
    }
}

pub struct JsonConfigurationAdapterModuleLoader;

impl ModuleLoader for JsonConfigurationAdapterModuleLoader {
    fn register_configuration_adapters(
        &mut self,
        registry: &mut HashMap<&'static str, Box<dyn ConfigurationAdapter>>,
    ) {
        registry.insert("json", Box::new(JsonConfigurationAdapter));
    }
}
