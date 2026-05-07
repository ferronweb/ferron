use std::collections::HashMap;

use anyhow::anyhow;
use ferron_core::{
    config::{adapter::ConfigurationAdapter, ServerConfiguration},
    loader::ModuleLoader,
};

mod translation;
mod watcher;

use translation::{load_top_level_statements, translate_configuration};
use watcher::{DisabledConfigurationWatcher, FerronConfConfigurationWatcher};

struct FerronConfConfigurationAdapter;

impl FerronConfConfigurationAdapter {
    #[inline]
    pub fn new() -> Self {
        Self
    }
}

impl ConfigurationAdapter for FerronConfConfigurationAdapter {
    fn adapt(
        &self,
        params: &HashMap<String, String>,
    ) -> Result<
        (
            ServerConfiguration,
            Box<dyn ferron_core::config::adapter::ConfigurationWatcher>,
        ),
        Box<dyn std::error::Error>,
    > {
        let filename = params.get("file").ok_or(anyhow!(
            "'file' parameter is required for 'ferronconf' configuration adapter"
        ))?;

        let mut include_stack = Vec::new();
        let mut loaded_files = Vec::new();
        let statements = load_top_level_statements(
            std::path::Path::new(filename),
            &mut include_stack,
            &mut loaded_files,
        )?;

        let watch_enabled = params
            .get("watch")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);

        let watcher: Box<dyn ferron_core::config::adapter::ConfigurationWatcher> = if watch_enabled
        {
            Box::new(FerronConfConfigurationWatcher::new(loaded_files)?)
        } else {
            Box::new(DisabledConfigurationWatcher)
        };

        Ok((translate_configuration(&statements)?, watcher))
    }

    #[inline]
    fn file_extension(&self) -> Vec<&'static str> {
        vec!["conf"]
    }
}

pub struct FerronConfConfigurationAdapterModuleLoader;

impl ModuleLoader for FerronConfConfigurationAdapterModuleLoader {
    fn register_configuration_adapters(
        &mut self,
        registry: &mut HashMap<&'static str, Box<dyn ConfigurationAdapter>>,
    ) {
        registry.insert(
            "ferronconf",
            Box::new(FerronConfConfigurationAdapter::new()),
        );
    }
}

#[cfg(test)]
mod tests;
