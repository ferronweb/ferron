use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use ferron_core::{config::adapter::ConfigurationAdapter, loader::ModuleLoader};
use notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebounceEventResult};
use tokio::sync::mpsc;

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
            "'file' parameter is required for 'json' configuration adapter"
        ))?;
        let file_contents = std::fs::read_to_string(filename)
            .map_err(|e| anyhow::anyhow!("Failed to read configuration file '{filename}': {e}",))?;

        let watch_enabled = params
            .get("watch")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);

        let watcher = if watch_enabled {
            Box::new(JsonConfigurationWatcher::new(PathBuf::from(filename))?)
                as Box<dyn ferron_core::config::adapter::ConfigurationWatcher>
        } else {
            Box::new(DisabledConfigurationWatcher)
                as Box<dyn ferron_core::config::adapter::ConfigurationWatcher>
        };

        Ok((
            serde_json::from_str(&file_contents).map_err(|e| {
                anyhow::anyhow!("Failed to parse configuration file '{filename}': {e}",)
            })?,
            watcher,
        ))
    }

    #[inline]
    fn file_extension(&self) -> Vec<&'static str> {
        vec!["json"]
    }
}

struct DisabledConfigurationWatcher;

#[async_trait]
impl ferron_core::config::adapter::ConfigurationWatcher for DisabledConfigurationWatcher {
    async fn watch(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        std::future::pending().await
    }
}

struct JsonConfigurationWatcher {
    _debouncer: notify_debouncer_mini::Debouncer<notify::RecommendedWatcher>,
    change_rx: mpsc::Receiver<DebounceEventResult>,
}

impl JsonConfigurationWatcher {
    fn new(path: PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let (tx, rx) = mpsc::channel(32);

        let mut debouncer = new_debouncer(
            Duration::from_millis(100),
            move |result: DebounceEventResult| {
                let _ = tx.blocking_send(result);
            },
        )?;

        debouncer
            .watcher()
            .watch(&path, RecursiveMode::NonRecursive)?;

        Ok(Self {
            _debouncer: debouncer,
            change_rx: rx,
        })
    }
}

#[async_trait]
impl ferron_core::config::adapter::ConfigurationWatcher for JsonConfigurationWatcher {
    async fn watch(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        match self.change_rx.recv().await {
            Some(Ok(_events)) => Ok(()),
            Some(Err(e)) => Err(Box::new(e)),
            None => Err("Watcher channel closed".into()),
        }
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
