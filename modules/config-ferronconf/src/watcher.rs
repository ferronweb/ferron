use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebounceEventResult};
use tokio::sync::mpsc;

pub(super) struct DisabledConfigurationWatcher;

#[async_trait]
impl ferron_core::config::adapter::ConfigurationWatcher for DisabledConfigurationWatcher {
    async fn watch(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        std::future::pending().await
    }
}

pub(super) struct FerronConfConfigurationWatcher {
    _debouncer: notify_debouncer_mini::Debouncer<notify::RecommendedWatcher>,
    change_rx: mpsc::Receiver<DebounceEventResult>,
}

impl FerronConfConfigurationWatcher {
    pub(super) fn new(files: Vec<PathBuf>) -> Result<Self, Box<dyn std::error::Error>> {
        let (tx, rx) = mpsc::channel(32);

        let mut debouncer = new_debouncer(
            Duration::from_millis(100),
            move |result: DebounceEventResult| {
                let _ = tx.blocking_send(result);
            },
        )?;

        let watcher = debouncer.watcher();
        for file in &files {
            watcher.watch(file, RecursiveMode::NonRecursive)?;
        }

        Ok(Self {
            _debouncer: debouncer,
            change_rx: rx,
        })
    }
}

#[async_trait]
impl ferron_core::config::adapter::ConfigurationWatcher for FerronConfConfigurationWatcher {
    async fn watch(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        match self.change_rx.recv().await {
            Some(Ok(_events)) => Ok(()),
            Some(Err(e)) => Err(Box::new(e)),
            None => Err("Watcher channel closed".into()),
        }
    }
}
