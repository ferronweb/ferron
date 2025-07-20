use std::{error::Error, path::Path};

use super::ServerConfiguration;

#[cfg(feature = "config-docker-auto")]
pub mod docker_auto;
pub mod kdl;
#[cfg(feature = "config-yaml-legacy")]
pub mod yaml_legacy;

/// A trait defining a Ferron server configuration file adapter
pub trait ConfigurationAdapter {
  /// Loads a server configuration for processing from the file specified by the path
  fn load_configuration(&self, path: &Path) -> Result<Vec<ServerConfiguration>, Box<dyn Error + Send + Sync>>;
}
