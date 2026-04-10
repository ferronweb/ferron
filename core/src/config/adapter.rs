//! Configuration adapters and watchers for loading and monitoring configuration sources.
//!
//! This module defines the interfaces for:
//! - Loading configuration from various sources (files, databases, APIs)
//! - Watching for configuration changes to support reload

use std::collections::HashMap;

use async_trait::async_trait;

use crate::config::ServerConfiguration;

/// Watches for changes in a configuration source.
///
/// Implementations can monitor files, databases, or other sources for changes
/// and notify when a reload is needed.
#[async_trait]
pub trait ConfigurationWatcher: Send + Sync {
    /// Wait until the configuration changes, then return.
    ///
    /// This function should block asynchronously until the configuration source
    /// has changed, indicating a reload is needed.
    ///
    /// # Errors
    ///
    /// Returns an error if watching fails (e.g., file deleted, permission denied).
    async fn watch(&mut self) -> Result<(), Box<dyn std::error::Error>>;
}

/// Adapter for loading server configuration from a specific source.
///
/// Adapters are responsible for parsing configuration from their source
/// (files, databases, etc.) and producing a `ServerConfiguration`.
///
/// # Example
///
/// ```ignore
/// struct YamlConfigAdapter;
/// impl ConfigurationAdapter for YamlConfigAdapter {
///     fn adapt(
///         &self,
///         params: &HashMap<String, String>,
///     ) -> Result<(ServerConfiguration, Box<dyn ConfigurationWatcher>), Box<dyn std::error::Error>> {
///         let path = params.get("path").ok_or("missing path")?;
///         let config = load_yaml_config(path)?;
///         let watcher = FileWatcher::new(path.into());
///         Ok((config, Box::new(watcher)))
///     }
///
///     fn file_extension(&self) -> Vec<&'static str> {
///         vec!["yaml", "yml"]
///     }
/// }
/// ```
pub trait ConfigurationAdapter {
    /// Load and adapt configuration from the source.
    ///
    /// # Arguments
    ///
    /// * `params` - Source-specific parameters (e.g., file paths, database URLs)
    ///
    /// # Returns
    ///
    /// A tuple containing:
    /// - The parsed `ServerConfiguration`
    /// - A `ConfigurationWatcher` to detect future changes
    fn adapt(
        &self,
        params: &HashMap<String, String>,
    ) -> Result<(ServerConfiguration, Box<dyn ConfigurationWatcher>), Box<dyn std::error::Error>>;

    /// File extensions this adapter can handle.
    ///
    /// Used for file-based adapters to filter which files can be loaded.
    /// Return an empty vector for non-file-based adapters.
    fn file_extension(&self) -> Vec<&'static str> {
        vec![]
    }
}
