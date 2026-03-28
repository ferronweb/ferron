use std::collections::HashMap;

use async_trait::async_trait;

use crate::config::ServerConfiguration;

#[async_trait]
pub trait ConfigurationWatcher: Send + Sync {
    /// Watches for changes in the configuration.
    /// This function should block until the configuration changes.
    async fn watch(self) -> Result<(), Box<dyn std::error::Error>>;
}

pub trait ConfigurationAdapter {
    /// Adapts the configuration to the server configuration.
    /// Returns the server configuration and a watcher that will watch for changes in the configuration.
    fn adapt(
        &self,
        params: &mut HashMap<String, String>,
    ) -> Result<(ServerConfiguration, Box<dyn ConfigurationWatcher>), Box<dyn std::error::Error>>;

    /// The file extensions that this adapter can handle (if configuration adapter is file-based).
    /// If the file extension is not in this list, the adapter will not be used.
    fn file_extension(&self) -> Vec<&'static str> {
        vec![]
    }
}
