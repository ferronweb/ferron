use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use notify::RecursiveMode;
use notify_debouncer_full::{new_debouncer_opt, DebounceEventResult};
use tokio::sync::mpsc;

pub(super) struct DisabledConfigurationWatcher;

#[async_trait]
impl ferron_core::config::adapter::ConfigurationWatcher for DisabledConfigurationWatcher {
    async fn watch(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        std::future::pending().await
    }
}

pub(super) struct FerronConfConfigurationWatcher {
    _debouncer: notify_debouncer_full::Debouncer<
        notify::RecommendedWatcher,
        notify_debouncer_full::NoCache,
    >,
    change_rx: mpsc::Receiver<Result<(), Box<dyn std::error::Error + Send + Sync>>>,
}

impl FerronConfConfigurationWatcher {
    pub(super) fn new(files: &[PathBuf]) -> Result<Self, Box<dyn std::error::Error>> {
        let (tx, rx) = mpsc::channel(32);

        // Default notify::Config would exclude access events.
        let debouncer_config = notify_debouncer_full::Config::default()
            .with_timeout(Duration::from_millis(100))
            .with_notify_config(notify::Config::default());
        let mut debouncer = new_debouncer_opt(
            debouncer_config,
            move |result: DebounceEventResult| {
                let new_result: Result<(), Box<dyn std::error::Error + Send + Sync>> =
                    result.map(|_| ()).map_err(|e| {
                        if let Some(e) = e.into_iter().next() {
                            Box::new(e).into()
                        } else {
                            "Unknown watcher error".into()
                        }
                    });
                let _ = tx.blocking_send(new_result);
            },
            notify_debouncer_full::NoCache,
        )?;

        for file in files {
            debouncer.watch(file, RecursiveMode::NonRecursive)?;
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
            Some(Ok(_)) => Ok(()),
            Some(Err(e)) => Err(e),
            None => Err("Watcher channel closed".into()),
        }
    }
}
