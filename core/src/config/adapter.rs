//! Configuration adapters and watchers for loading and monitoring configuration sources.
//!
//! This module defines the interfaces for:
//! - Loading configuration from various sources (files, databases, APIs)
//! - Watching for configuration changes to support reload

use std::collections::HashMap;
use std::time::SystemTime;

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

/// Metadata about the loaded configuration source.
///
/// Returned alongside the parsed configuration to enable drift detection
/// and observability without re-reading the source.
pub struct ConfigurationMetadata {
    /// Content hash of the configuration source (e.g., xxh3 hex of all loaded files).
    pub config_hash: String,
    /// Last modification time of the configuration source.
    pub config_mtime: SystemTime,
}

/// Result type for `ConfigurationAdapter::adapt()`.
///
/// Contains the parsed configuration, a watcher for future changes,
/// and metadata about the configuration source.
pub type AdaptResult = Result<
    (
        ServerConfiguration,
        Box<dyn ConfigurationWatcher>,
        ConfigurationMetadata,
    ),
    Box<dyn std::error::Error>,
>;

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
///     ) -> Result<(ServerConfiguration, Box<dyn ConfigurationWatcher>, ConfigurationMetadata), Box<dyn std::error::Error>> {
///         let path = params.get("path").ok_or("missing path")?;
///         let config = load_yaml_config(path)?;
///         let watcher = FileWatcher::new(path.into());
///         let metadata = ConfigurationMetadata {
///             config_hash: compute_hash(path),
///             config_mtime: std::fs::metadata(path)?.modified()?,
///         };
///         Ok((config, Box::new(watcher), metadata))
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
    /// - `ConfigurationMetadata` with content hash and modification time
    fn adapt(&self, params: &HashMap<String, String>) -> AdaptResult;

    /// File extensions this adapter can handle.
    ///
    /// Used for file-based adapters to filter which files can be loaded.
    /// Return an empty vector for non-file-based adapters.
    fn file_extension(&self) -> Vec<&'static str> {
        vec![]
    }
}
