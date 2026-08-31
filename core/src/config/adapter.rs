//! Configuration adapters and watchers for loading and monitoring configuration sources.
//!
//! An [`ConfigurationAdapter`] loads server configuration from a specific
//! source (file, database, API, etc.) and returns a
//! [`ConfigurationWatcher`] for detecting future changes. Adapters are
//! registered by name in
//! [`ModuleLoader::register_configuration_adapters`](crate::loader::ModuleLoader::register_configuration_adapters)
//! and selected by the user via the `--config-adapter` CLI flag.
//!
//! # Writing an adapter
//!
//! 1. Implement [`ConfigurationAdapter`]. Return a
//!    [`ServerConfiguration`](crate::config::ServerConfiguration), a boxed
//!    [`ConfigurationWatcher`], and [`ConfigurationMetadata`].
//! 2. Register it in your [`ModuleLoader`](crate::loader::ModuleLoader) via
//!    `registry.insert("my_adapter", Box::new(MyAdapter))`.
//!
//! # Example
//!
//! ```ignore
//! use ferron_core::config::adapter::{ConfigurationAdapter, AdaptResult};
//!
//! struct JsonAdapter;
//!
//! impl ConfigurationAdapter for JsonAdapter {
//!     fn adapt(
//!         &self,
//!         params: &HashMap<String, String>,
//!     ) -> AdaptResult {
//!         let path = params.get("file").ok_or("missing 'file' param")?;
//!         let config = std::fs::read_to_string(path)?;
//!         let parsed: ServerConfiguration = serde_json::from_str(&config)?;
//!         // ... create watcher and metadata ...
//!         Ok((parsed, Box::new(watcher), metadata))
//!     }
//!
//!     fn file_extension(&self) -> Vec<&'static str> {
//!         vec!["json"]
//!     }
//! }
//! ```

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;

use async_trait::async_trait;

use super::validator::ConfigurationValidationError;
use crate::config::ServerConfiguration;

/// An error that occurred while adapting configuration from a source,
/// with an optional span.
pub type ConfigurationAdapterError = ConfigurationValidationError;

/// Watches for changes in a configuration source.
///
/// Implement this trait to detect when the underlying configuration source
/// has changed (e.g. file modification, database update). The server calls
/// [`watch`](Self::watch) in a loop to block until a change is detected,
/// then triggers a configuration reload.
///
/// For lightweight change detection without full re-parsing, implement
/// [`check_drift`](Self::check_drift).
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

    /// Check whether the configuration source has drifted since the last load.
    ///
    /// This performs a lightweight check (e.g., re-stat files) without
    /// re-parsing the configuration. Returns `true` if the source has changed.
    fn check_drift(&self, _metadata: &ConfigurationMetadata) -> bool {
        false
    }
}

/// Metadata about a loaded configuration source.
///
/// Returned alongside the parsed configuration to enable drift detection
/// and observability without re-reading the source. The server stores
/// this metadata in [`AdminMetrics`](crate::admin::AdminMetrics) and uses
/// it for the `doctor` subcommand.
pub struct ConfigurationMetadata {
    /// Content hash of the configuration source (e.g., xxh3 hex of all loaded files).
    pub config_hash: String,
    /// Last modification time of the configuration source.
    pub config_mtime: SystemTime,
    /// Files that were loaded to produce this configuration.
    pub config_files: Vec<PathBuf>,
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
    ConfigurationAdapterError,
>;

/// Adapter for loading server configuration from a specific source.
///
/// Adapters are responsible for parsing configuration from their source
/// and producing a [`ServerConfiguration`](crate::config::ServerConfiguration).
/// They are registered by name in
/// [`ModuleLoader::register_configuration_adapters`](crate::loader::ModuleLoader::register_configuration_adapters)
/// and selected by the user via `--config-adapter <name>`.
///
/// # Arguments
///
/// The `params` HashMap contains source-specific parameters passed by the
/// user via `--config-params key=value`. For file-based adapters, the
/// `file` key is conventional.
///
/// # Example
///
/// ```ignore
/// struct YamlAdapter;
/// impl ConfigurationAdapter for YamlAdapter {
///     fn adapt(
///         &self,
///         params: &HashMap<String, String>,
///     ) -> AdaptResult {
///         let path = params.get("file").ok_or("missing 'file' param")?;
///         let config = load_yaml(path)?;
///         let watcher = FileWatcher::new(path.into());
///         let metadata = ConfigurationMetadata { /* ... */ };
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
    /// * `params` -- Source-specific parameters (e.g. file paths, database
    ///   URLs). Passed from the user via `--config-params`.
    ///
    /// # Returns
    ///
    /// A tuple of:
    ///
    /// 1. The parsed [`ServerConfiguration`](crate::config::ServerConfiguration).
    /// 2. A [`ConfigurationWatcher`] for detecting future changes.
    /// 3. [`ConfigurationMetadata`] with content hash and modification time.
    fn adapt(&self, params: &HashMap<String, String>) -> AdaptResult;

    /// File extensions this adapter can handle.
    ///
    /// Used by file-based adapters to select the right adapter from a list
    /// of loaded files. Return an empty vector for non-file-based adapters.
    fn file_extension(&self) -> Vec<&'static str> {
        vec![]
    }
}
